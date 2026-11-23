//! Draws the camera, the logo and the input overlay into exported frames, where the player
//! shows them over the clip. `ui/src/lib/editor/overlay.ts` places them with the same
//! arithmetic.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::error::EAGAIN;
use ffmpeg_next::ffi::{
    AV_FRAME_CROP_UNALIGNED, AVFrame, AVPixelFormat, AVSEEK_FLAG_BACKWARD, SWS_CS_ITU709,
    av_frame_apply_cropping, av_frame_make_writable, av_seek_frame, sws_getCoefficients,
    sws_setColorspaceDetails,
};
use ffmpeg_next::software::scaling;
use ffmpeg_next::util::format::Pixel;
use ffmpeg_next::{Error as AvError, codec, format, frame, media};

use crate::export::CAMERA_GAP_SECS;
use crate::settings::{OverlayShape, Placement, Rect};

/// The clip's camera drawn in, except for the hidden ranges (seconds from the start of the
/// clip).
#[derive(Clone, Debug)]
pub struct CameraBurn {
    pub placement: Placement,
    pub hidden: Vec<(f64, f64)>,
}

/// A picture (a logo) drawn in.
#[derive(Clone, Debug)]
pub struct LogoBurn {
    pub path: PathBuf,
    pub placement: Placement,
}

/// The clip's keyboard and mouse input drawn in, as `overlay` shows it.
#[derive(Clone, Debug)]
pub struct InputBurn {
    pub overlay: crate::settings::InputOverlayPrefs,
}

/// What is drawn into each frame of one export.
pub(crate) struct Burn {
    camera: Option<CameraLayer>,
    logo: Option<Layer>,
    input: Option<InputLayer>,
}

/// The input overlay: drawn again only when what it shows changes, and kept for a while,
/// since the same keys come back often (W, then W and A).
struct InputLayer {
    timeline: crate::inputdraw::Timeline,
    composition: crate::inputdraw::Composition,
    overlay: crate::settings::InputOverlayPrefs,
    painter: crate::inputdraw::Painter,
    /// Where it goes on the frame, and its width in pixels.
    place: (usize, usize, u32),
    drawn: std::collections::HashMap<crate::inputdraw::Visual, Layer>,
}

impl InputLayer {
    const KEPT: usize = 256;

    fn open(video: &Path, burn: &InputBurn, width: u32, height: u32) -> Result<Option<Self>> {
        let Some(path) = crate::inputfile::existing_sidecar(video) else {
            return Ok(None);
        };
        let Some(composition) = crate::inputdraw::Composition::new(&burn.overlay) else {
            return Ok(None);
        };
        let timeline = crate::inputdraw::Timeline::load(&path)?;
        let placement = burn.overlay.placement.clone().normalized();
        let aspect = composition.width / composition.height;
        let plain = Placement {
            crop: Rect::default(),
            shape: OverlayShape::Rectangle,
            border: 0.0,
            mirror: false,
            ..placement
        };
        let (left, top, w, _) = overlay_box(&plain, aspect, f64::from(width), f64::from(height));
        let even = |v: f64| ((v / 2.0).round() as usize) * 2;
        let w = (even(w).max(2) as u32).min(width & !1);
        Ok(Some(Self {
            timeline,
            composition,
            overlay: burn.overlay.clone(),
            painter: crate::inputdraw::Painter::new()?,
            place: (even(left), even(top), w),
            drawn: Default::default(),
        }))
    }

    fn at(&mut self, clip_secs: f64, frame_w: u32, frame_h: u32) -> Result<&Layer> {
        let visual = self.timeline.visual(clip_secs, &self.composition);
        if !self.drawn.contains_key(&visual) {
            if self.drawn.len() >= Self::KEPT {
                self.drawn.clear();
            }
            let (x, y, w) = self.place;
            let picture = self
                .painter
                .paint(&self.composition, &visual, w, &self.overlay)?;
            let layer = Layer::from_rgba(x, y, &picture, frame_w, frame_h)?;
            self.drawn.insert(visual.clone(), layer);
        }
        Ok(&self.drawn[&visual])
    }
}

struct CameraLayer {
    reader: CameraReader,
    placement: Placement,
    hidden: Vec<(f64, f64)>,
    scaler: Scaler,
    layer: Option<Layer>,
}

impl Burn {
    /// For frames of `width`×`height`; `input` is the clip, read from `from_secs` (its own
    /// clock). `None` when there is nothing to draw.
    pub(crate) fn open(
        input: &Path,
        camera: Option<&CameraBurn>,
        logo: Option<&LogoBurn>,
        keys: Option<&InputBurn>,
        width: u32,
        height: u32,
        from_secs: f64,
    ) -> Result<Option<Self>> {
        let camera = match camera {
            Some(burn) => CameraReader::open(input, from_secs)?.map(|reader| CameraLayer {
                reader,
                placement: burn.placement.clone().normalized(),
                hidden: burn.hidden.clone(),
                scaler: Scaler::default(),
                layer: None,
            }),
            None => None,
        };
        let logo = match logo {
            Some(burn) => {
                let mut image = load_image(&burn.path)
                    .with_context(|| format!("cannot read the logo {}", burn.path.display()))?;
                let placement = burn.placement.clone().normalized();
                Some(Layer::build(
                    &mut image,
                    &placement,
                    width,
                    height,
                    &mut Scaler::default(),
                )?)
            }
            None => None,
        };
        let input = match keys {
            Some(burn) => InputLayer::open(input, burn, width, height)?,
            None => None,
        };
        Ok(
            (camera.is_some() || logo.is_some() || input.is_some()).then_some(Self {
                camera,
                logo,
                input,
            }),
        )
    }

    /// Draws into `frame` (NV12 or planar 4:2:0, 8 bits): the logo, and the camera over it
    /// as in the player. `at` is the frame's time on the clip's clock, `clip_secs` the same
    /// counted from the start of the clip.
    pub(crate) fn draw(&mut self, frame: *mut AVFrame, at: f64, clip_secs: f64) -> Result<()> {
        let (width, height) = unsafe { ((*frame).width as u32, (*frame).height as u32) };
        let camera = match self.camera.as_mut() {
            Some(cam)
                if !cam
                    .hidden
                    .iter()
                    .any(|&(a, b)| clip_secs >= a && clip_secs < b) =>
            {
                match cam.reader.at(at)? {
                    Some((picture, fresh)) => {
                        if fresh || cam.layer.is_none() {
                            cam.layer = Some(Layer::build(
                                picture,
                                &cam.placement,
                                width,
                                height,
                                &mut cam.scaler,
                            )?);
                        }
                        cam.layer.as_ref()
                    }
                    None => None,
                }
            }
            _ => None,
        };
        let input = match self.input.as_mut() {
            Some(input) => Some(input.at(clip_secs, width, height)?),
            None => None,
        };
        let layers: Vec<&Layer> = self.logo.iter().chain(camera).chain(input).collect();
        if layers.is_empty() {
            return Ok(());
        }
        unsafe {
            // The decoder may still hold the picture as a reference for the next ones.
            let code = av_frame_make_writable(frame);
            if code < 0 {
                bail!("cannot draw into the frame: {}", AvError::from(code));
            }
            for layer in layers {
                layer.blend(frame)?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// Placing and preparing a picture
// ---------------------------------------------------------------------------------------

/// The part of the source shown: the crop, made square about its middle for a circle.
/// `aspect` is the source's width over its height.
fn shown_crop(p: &Placement, aspect: f64) -> Rect {
    let c = p.crop;
    if p.shape != OverlayShape::Circle {
        return c;
    }
    let (w, h) = (c.width * aspect, c.height);
    if w > h {
        let width = h / aspect;
        Rect {
            x: c.x + (c.width - width) / 2.0,
            width,
            ..c
        }
    } else {
        Rect {
            y: c.y + (c.height - w) / 2.0,
            height: w,
            ..c
        }
    }
}

/// The picture's box on a `frame_w`×`frame_h` frame as (left, top, width, height), kept on
/// it, before rounding.
fn overlay_box(p: &Placement, aspect: f64, frame_w: f64, frame_h: f64) -> (f64, f64, f64, f64) {
    let c = shown_crop(p, aspect);
    let width = p.width * frame_w;
    let height = width / (c.width * aspect / c.height);
    let left = (p.x * frame_w).clamp(0.0, (frame_w - width).max(0.0));
    let top = (p.y * frame_h).clamp(0.0, (frame_h - height).max(0.0));
    (left, top, width, height)
}

fn corner_radius(shape: OverlayShape, width: f64, height: f64) -> f64 {
    match shape {
        OverlayShape::Circle => width.min(height) / 2.0,
        OverlayShape::Rounded => width.min(height) * 0.12,
        OverlayShape::Rectangle => 0.0,
    }
}

/// Signed distance from (`x`, `y`) to the edge of a `w`×`h` box with corners of radius `r`:
/// negative inside.
fn rounded_distance(x: f64, y: f64, w: f64, h: f64, r: f64) -> f64 {
    let qx = (x - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (y - h / 2.0).abs() - (h / 2.0 - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// `#rrggbb` or `#rgb`; white for anything else.
fn parse_color(css: &str) -> [f64; 3] {
    let hex = css.trim().trim_start_matches('#');
    let digits: Vec<u8> = hex
        .chars()
        .filter_map(|c| c.to_digit(16).map(|d| d as u8))
        .collect();
    match digits.as_slice() {
        [r, g, b] if hex.len() == 3 => [*r, *g, *b].map(|d| f64::from(d * 17)),
        [r1, r2, g1, g2, b1, b2] if hex.len() == 6 => {
            [(r1, r2), (g1, g2), (b1, b2)].map(|(a, b)| f64::from(a * 16 + b))
        }
        _ => [255.0; 3],
    }
}

/// Turns pictures into RGBA of the box's size; keeps its converter between frames.
struct Scaler {
    context: Option<(scaling::Context, (Pixel, u32, u32, u32, u32))>,
    rgba: frame::Video,
}

impl Default for Scaler {
    fn default() -> Self {
        Self {
            context: None,
            rgba: frame::Video::empty(),
        }
    }
}

impl Scaler {
    fn run(&mut self, picture: &frame::Video, width: u32, height: u32) -> Result<&frame::Video> {
        let key = (
            picture.format(),
            picture.width(),
            picture.height(),
            width,
            height,
        );
        if self.context.as_ref().is_none_or(|(_, k)| *k != key) {
            let mut context = scaling::Context::get(
                key.0,
                key.1,
                key.2,
                Pixel::RGBA,
                width,
                height,
                scaling::Flags::BICUBIC,
            )
            .with_context(|| format!("cannot convert a {:?} picture", key.0))?;
            // Umbra's videos are BT.709; the converter assumes BT.601 unless told.
            unsafe {
                let full = picture.color_range() == ffmpeg_next::color::Range::JPEG
                    || picture.format() == Pixel::YUVJ420P;
                let table = sws_getCoefficients(SWS_CS_ITU709);
                sws_setColorspaceDetails(
                    context.as_mut_ptr(),
                    table,
                    i32::from(full),
                    table,
                    1,
                    0,
                    1 << 16,
                    1 << 16,
                );
            }
            self.context = Some((context, key));
            self.rgba = frame::Video::empty();
        }
        let (context, _) = self.context.as_mut().unwrap();
        context
            .run(picture, &mut self.rgba)
            .context("cannot convert the picture")?;
        Ok(&self.rgba)
    }
}

/// A picture ready to blend: its box on the frame (even, so it lines up with the colour
/// planes) and its pixels as luma and coverage, with colour and coverage at half size.
struct Layer {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    luma: Vec<u8>,
    alpha: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    chroma_alpha: Vec<u8>,
}

impl Layer {
    /// `picture` as `placement` shows it on a `frame_w`×`frame_h` frame: cropped, scaled,
    /// mirrored, shaped and bordered. Crops `picture` in place.
    fn build(
        picture: &mut frame::Video,
        placement: &Placement,
        frame_w: u32,
        frame_h: u32,
        scaler: &mut Scaler,
    ) -> Result<Self> {
        let (sw, sh) = (picture.width(), picture.height());
        if sw == 0 || sh == 0 {
            bail!("the picture is empty");
        }
        let aspect = f64::from(sw) / f64::from(sh);
        let (left, top, width, height) =
            overlay_box(placement, aspect, f64::from(frame_w), f64::from(frame_h));
        let even = |v: f64| ((v / 2.0).round() as usize) * 2;
        let (x, y) = (even(left), even(top));
        let w = even(width)
            .max(2)
            .min((frame_w as usize).saturating_sub(x) & !1);
        let h = even(height)
            .max(2)
            .min((frame_h as usize).saturating_sub(y) & !1);
        if w < 2 || h < 2 {
            bail!("the picture does not fit on the frame");
        }

        // The part shown, cut out of the source.
        let c = shown_crop(placement, aspect);
        let px = |f: f64, size: u32| ((f * f64::from(size)).round() as u32).min(size - 1);
        let (cx, cy) = (px(c.x, sw), px(c.y, sh));
        let cw = ((c.width * f64::from(sw)).round() as u32).clamp(1, sw - cx);
        let ch = ((c.height * f64::from(sh)).round() as u32).clamp(1, sh - cy);
        if (cx, cy, cw, ch) != (0, 0, sw, sh) {
            unsafe {
                let raw = picture.as_mut_ptr();
                (*raw).crop_left = cx as usize;
                (*raw).crop_top = cy as usize;
                (*raw).crop_right = (sw - cx - cw) as usize;
                (*raw).crop_bottom = (sh - cy - ch) as usize;
                let code = av_frame_apply_cropping(raw, AV_FRAME_CROP_UNALIGNED as i32);
                if code < 0 {
                    bail!("cannot crop the picture: {}", AvError::from(code));
                }
            }
        }
        let rgba = scaler.run(picture, w as u32, h as u32)?;
        let (data, stride) = (rgba.data(0), rgba.stride(0));

        let (fw, fh) = (w as f64, h as f64);
        let radius = corner_radius(placement.shape, fw, fh);
        let border = placement.border * width;
        let color = parse_color(&placement.border_color);
        let (cw2, ch2) = (w / 2, h / 2);
        let mut layer = Self {
            x,
            y,
            w,
            h,
            luma: vec![0; w * h],
            alpha: vec![0; w * h],
            u: vec![128; cw2 * ch2],
            v: vec![128; cw2 * ch2],
            chroma_alpha: vec![0; cw2 * ch2],
        };
        // Colour summed per 2×2 block, weighted by coverage.
        let mut sums = vec![[0.0f64; 3]; cw2 * ch2];
        for row in 0..h {
            for col in 0..w {
                let d = rounded_distance(col as f64 + 0.5, row as f64 + 0.5, fw, fh, radius);
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let from = if placement.mirror { w - 1 - col } else { col };
                let p = &data[row * stride + from * 4..][..4];
                let ring = if border > 0.0 {
                    (d + border + 0.5).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let mut rgb = [0.0; 3];
                for (i, out) in rgb.iter_mut().enumerate() {
                    *out = f64::from(p[i]) * (1.0 - ring) + color[i] * ring;
                }
                let a = cover * (f64::from(p[3]) / 255.0 * (1.0 - ring) + ring);
                let [r, g, b] = rgb;
                let (luma, u, v) = yuv(r, g, b);
                layer.luma[row * w + col] = luma;
                layer.alpha[row * w + col] = (a * 255.0).round() as u8;
                let block = &mut sums[(row / 2) * cw2 + col / 2];
                block[0] += u * a;
                block[1] += v * a;
                block[2] += a;
            }
        }
        layer.finish_chroma(sums);
        Ok(layer)
    }

    /// A picture drawn at its size already (straight RGBA), at `x`, `y` on the frame.
    fn from_rgba(
        x: usize,
        y: usize,
        picture: &crate::inputdraw::Picture,
        frame_w: u32,
        frame_h: u32,
    ) -> Result<Self> {
        let w = (picture.width as usize & !1).min((frame_w as usize).saturating_sub(x) & !1);
        let h = (picture.height as usize & !1).min((frame_h as usize).saturating_sub(y) & !1);
        if w < 2 || h < 2 {
            bail!("the picture does not fit on the frame");
        }
        let (cw2, ch2) = (w / 2, h / 2);
        let mut layer = Self {
            x,
            y,
            w,
            h,
            luma: vec![0; w * h],
            alpha: vec![0; w * h],
            u: vec![128; cw2 * ch2],
            v: vec![128; cw2 * ch2],
            chroma_alpha: vec![0; cw2 * ch2],
        };
        let mut sums = vec![[0.0f64; 3]; cw2 * ch2];
        for row in 0..h {
            for col in 0..w {
                let p = &picture.rgba[(row * picture.width as usize + col) * 4..][..4];
                if p[3] == 0 {
                    continue;
                }
                let a = f64::from(p[3]) / 255.0;
                let (luma, u, v) = yuv(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
                layer.luma[row * w + col] = luma;
                layer.alpha[row * w + col] = p[3];
                let block = &mut sums[(row / 2) * cw2 + col / 2];
                block[0] += u * a;
                block[1] += v * a;
                block[2] += a;
            }
        }
        layer.finish_chroma(sums);
        Ok(layer)
    }

    fn finish_chroma(&mut self, sums: Vec<[f64; 3]>) {
        for (i, [u, v, a]) in sums.into_iter().enumerate() {
            if a > 0.0 {
                self.u[i] = (128.0 + u / a).round().clamp(0.0, 255.0) as u8;
                self.v[i] = (128.0 + v / a).round().clamp(0.0, 255.0) as u8;
                self.chroma_alpha[i] = (a / 4.0 * 255.0).round() as u8;
            }
        }
    }

    /// Mixes the layer into a writable NV12 or planar 4:2:0 frame.
    unsafe fn blend(&self, frame: *mut AVFrame) -> Result<()> {
        let mix = |dst: &mut u8, src: u8, a: u8| {
            let a = u32::from(a);
            *dst = ((u32::from(src) * a + u32::from(*dst) * (255 - a) + 127) / 255) as u8;
        };
        unsafe {
            let f = &*frame;
            let nv12 = f.format == AVPixelFormat::AV_PIX_FMT_NV12 as i32;
            if !nv12 && f.format != AVPixelFormat::AV_PIX_FMT_YUV420P as i32 {
                bail!("cannot draw on frames of format {}", f.format);
            }
            if self.x + self.w > f.width as usize || self.y + self.h > f.height as usize {
                return Err(anyhow!("the picture is outside the frame"));
            }
            for row in 0..self.h {
                let line = f.data[0].offset((self.y + row) as isize * f.linesize[0] as isize);
                let dst = std::slice::from_raw_parts_mut(line.add(self.x), self.w);
                for (col, px) in dst.iter_mut().enumerate() {
                    let i = row * self.w + col;
                    if self.alpha[i] > 0 {
                        mix(px, self.luma[i], self.alpha[i]);
                    }
                }
            }
            let (cw, ch) = (self.w / 2, self.h / 2);
            let (cx, cy) = (self.x / 2, self.y / 2);
            for row in 0..ch {
                for col in 0..cw {
                    let i = row * cw + col;
                    let a = self.chroma_alpha[i];
                    if a == 0 {
                        continue;
                    }
                    let (u, v) = if nv12 {
                        let line = f.data[1].offset((cy + row) as isize * f.linesize[1] as isize);
                        let at = line.add((cx + col) * 2);
                        (&mut *at, &mut *at.add(1))
                    } else {
                        let u = f.data[1].offset((cy + row) as isize * f.linesize[1] as isize);
                        let v = f.data[2].offset((cy + row) as isize * f.linesize[2] as isize);
                        (&mut *u.add(cx + col), &mut *v.add(cx + col))
                    };
                    mix(u, self.u[i], a);
                    mix(v, self.v[i], a);
                }
            }
        }
        Ok(())
    }
}

/// Luma (0–255) and colour (about 0) of an RGB pixel: BT.709, limited range, as Umbra
/// records.
fn yuv(r: f64, g: f64, b: f64) -> (u8, f64, f64) {
    let luma = 16.0 + (0.2126 * r + 0.7152 * g + 0.0722 * b) * 219.0 / 255.0;
    let u = (-0.1146 * r - 0.3854 * g + 0.5 * b) * 224.0 / 255.0;
    let v = (0.5 * r - 0.4542 * g - 0.0458 * b) * 224.0 / 255.0;
    (luma.round() as u8, u, v)
}

// ---------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------

/// The first picture of an image file (PNG, JPEG, WebP).
fn load_image(path: &Path) -> Result<frame::Video> {
    let mut input = format::input(&path)?;
    let stream = input
        .streams()
        .best(media::Type::Video)
        .context("it is not an image")?;
    let index = stream.index();
    let mut decoder = codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .video()?;
    for (stream, packet) in input.packets() {
        if stream.index() == index {
            decoder.send_packet(&packet)?;
            break;
        }
    }
    decoder.send_eof()?;
    let mut image = frame::Video::empty();
    decoder.receive_frame(&mut image)?;
    Ok(image)
}

/// The camera stream of a clip, read alongside the screen from a second handle on the file.
// ponytail: reads the file a second time (the screen's packets are skipped); fine from the
// disk cache, share the one demuxer if exports of long clips turn out disk-bound.
struct CameraReader {
    input: format::context::Input,
    index: usize,
    /// Seconds per tick of the stream.
    tick: f64,
    decoder: ffmpeg_next::decoder::Video,
    /// The newest frame at or before the time last asked for.
    current: Option<(f64, frame::Video)>,
    /// Decoded, not yet due.
    next: Option<(f64, frame::Video)>,
    ended: bool,
}

impl CameraReader {
    /// `None` for a clip without a camera.
    fn open(path: &Path, from_secs: f64) -> Result<Option<Self>> {
        let mut input =
            format::input(&path).with_context(|| format!("cannot open {}", path.display()))?;
        let Some(stream) = input.streams().find(crate::library::is_camera_stream) else {
            return Ok(None);
        };
        let index = stream.index();
        let time_base = stream.time_base();
        let tick = f64::from(time_base.numerator()) / f64::from(time_base.denominator().max(1));
        let decoder = codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .video()
            .context("cannot open the camera decoder")?;
        // From the camera keyframe before the start; when that fails, from the beginning.
        let ts = (from_secs / tick) as i64;
        unsafe {
            av_seek_frame(input.as_mut_ptr(), index as i32, ts, AVSEEK_FLAG_BACKWARD);
        }
        Ok(Some(Self {
            input,
            index,
            tick,
            decoder,
            current: None,
            next: None,
            ended: false,
        }))
    }

    /// The camera's picture at `t` (seconds on the clip's clock), and whether it is another
    /// one than last time; `None` where the camera was off. `t` must not go back.
    fn at(&mut self, t: f64) -> Result<Option<(&mut frame::Video, bool)>> {
        let mut fresh = false;
        loop {
            if self.next.is_none() {
                self.next = self.decode()?;
            }
            match &self.next {
                Some((time, _)) if *time <= t => {
                    self.current = self.next.take();
                    fresh = true;
                }
                _ => break,
            }
        }
        Ok(match &mut self.current {
            Some((time, picture)) if t - *time < CAMERA_GAP_SECS => Some((picture, fresh)),
            _ => None,
        })
    }

    fn decode(&mut self) -> Result<Option<(f64, frame::Video)>> {
        let mut picture = frame::Video::empty();
        loop {
            match self.decoder.receive_frame(&mut picture) {
                Ok(()) => {
                    if let Some(pts) = picture.timestamp().or(picture.pts()) {
                        return Ok(Some((pts as f64 * self.tick, picture)));
                    }
                    continue;
                }
                Err(AvError::Eof) => return Ok(None),
                Err(AvError::Other { errno }) if errno == EAGAIN => {}
                Err(err) => return Err(err).context("camera decode failed"),
            }
            if self.ended {
                return Ok(None);
            }
            loop {
                match self.input.packets().next() {
                    Some((stream, packet)) if stream.index() == self.index => {
                        match self.decoder.send_packet(&packet) {
                            Ok(()) | Err(AvError::InvalidData) => {}
                            Err(err) => return Err(err).context("camera decode failed"),
                        }
                        break;
                    }
                    Some(_) => {}
                    None => {
                        self.ended = true;
                        let _ = self.decoder.send_eof();
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_like_the_player() {
        // A 16:9 camera a fifth of a 1920x1080 frame wide, bottom right.
        let p = Placement::default();
        let (l, t, w, h) = overlay_box(&p, 16.0 / 9.0, 1920.0, 1080.0);
        assert!((w - 384.0).abs() < 1e-9 && (h - 216.0).abs() < 1e-9);
        assert!((l - 0.78 * 1920.0).abs() < 1e-9 && (t - 0.75 * 1080.0).abs() < 1e-9);
        // A circle shows a square from the middle.
        let circle = Placement {
            shape: OverlayShape::Circle,
            ..Placement::default()
        };
        let c = shown_crop(&circle, 16.0 / 9.0);
        assert!((c.width - 9.0 / 16.0).abs() < 1e-9 && (c.x - 7.0 / 32.0).abs() < 1e-9);
        let (_, _, w, h) = overlay_box(&circle, 16.0 / 9.0, 1920.0, 1080.0);
        assert!((w - h).abs() < 1e-9);
        // Kept on the frame.
        let off = Placement {
            x: 0.95,
            ..Placement::default()
        };
        let (l, ..) = overlay_box(&off, 16.0 / 9.0, 1920.0, 1080.0);
        assert!((l - (1920.0 - 384.0)).abs() < 1e-9);
    }

    #[test]
    fn shapes_and_colours() {
        assert!(rounded_distance(50.0, 50.0, 100.0, 100.0, 50.0) < 0.0);
        assert!(
            rounded_distance(1.0, 1.0, 100.0, 100.0, 50.0) > 0.0,
            "corner is cut"
        );
        assert!(
            rounded_distance(1.0, 1.0, 100.0, 100.0, 0.0) < 0.0,
            "square keeps it"
        );
        assert_eq!(parse_color("#ff8000"), [255.0, 128.0, 0.0]);
        assert_eq!(parse_color("#f80"), [255.0, 136.0, 0.0]);
        assert_eq!(parse_color("red"), [255.0; 3]);
    }
}
