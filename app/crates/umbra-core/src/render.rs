//! Rendering an edit into a new file: several parts one after another, stretches sped up or
//! slowed down, fades, and any output format, from MP4 to GIF or audio alone.
//!
//! The parts are files the exporter already made (trimmed, cut, cropped, with their tracks
//! mixed into one), so this only has to join, retime and re-encode. Video is written at a fixed
//! frame rate: a slowed stretch repeats frames, a sped-up one skips them. Audio is read faster
//! or slower too, which changes its pitch the way a tape does, matching the editor's preview.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::ptr;

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::ffi::{
    AV_CODEC_FLAG_GLOBAL_HEADER, AVCodecContext, AVFrame, AVPixelFormat, AVRational,
    AVSampleFormat, av_channel_layout_copy, av_channel_layout_default, av_frame_alloc,
    av_frame_free, av_frame_get_buffer, av_frame_make_writable, av_opt_set, avcodec_alloc_context3,
    avcodec_find_encoder_by_name, avcodec_free_context, avcodec_open2,
    avcodec_parameters_from_context, avcodec_receive_packet, avcodec_send_frame,
};
use ffmpeg_next::format::Pixel;
use ffmpeg_next::packet::Mut;
use ffmpeg_next::software::scaling;
use ffmpeg_next::util::frame;
use ffmpeg_next::{Packet, Rational, codec, format, media};
use serde::{Deserialize, Serialize};

use crate::encode::{StreamInfo, VideoCodec};
use crate::mux::Mp4Writer;

const AVERROR_EAGAIN: i32 = -11;
const AVERROR_EOF: i32 = -541_478_725;
/// Audio is worked on at this rate, in stereo.
const RATE: u32 = 48_000;

/// What the file is written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    #[default]
    Mp4,
    Mov,
    Mkv,
    Webm,
    Gif,
    Mp3,
    M4a,
    Wav,
    Flac,
    Opus,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
            Self::Gif => "gif",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Opus => "opus",
        }
    }

    fn container(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Mkv => "matroska",
            Self::Webm => "webm",
            Self::Gif => "gif",
            Self::Mp3 => "mp3",
            Self::M4a => "ipod",
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Opus => "ogg",
        }
    }

    pub fn has_video(self) -> bool {
        matches!(
            self,
            Self::Mp4 | Self::Mov | Self::Mkv | Self::Webm | Self::Gif
        )
    }

    pub fn has_audio(self) -> bool {
        self != Self::Gif
    }

    /// Audio encoder names, best first.
    fn audio_encoders(self) -> &'static [&'static str] {
        match self {
            Self::Webm | Self::Opus => &["libopus", "opus"],
            Self::Mp3 => &["libmp3lame"],
            Self::Wav => &["pcm_s16le"],
            Self::Flac => &["flac"],
            _ => &["aac"],
        }
    }
}

/// A stretch played at another speed: 0.5 is half speed, 2 twice as fast.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedRange {
    pub start: f64,
    pub end: f64,
    pub speed: f64,
}

/// One input: a file with one video stream and at most one audio stream (an export of a clip
/// with its tracks mixed), and the stretches of it, in its own seconds, at another speed.
#[derive(Clone, Debug)]
pub struct RenderPart {
    pub path: PathBuf,
    pub speeds: Vec<SpeedRange>,
}

/// Fades, in seconds: from black and silence at the start, to them at the end, and through
/// them where one part meets the next (this long on each side). 0 leaves one out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Fades {
    pub fade_in: f64,
    pub fade_out: f64,
    pub between: f64,
}

#[derive(Clone, Debug)]
pub struct RenderJob {
    pub parts: Vec<RenderPart>,
    pub output: PathBuf,
    pub format: OutputFormat,
    /// Video codec for MP4, MOV and MKV; `None` keeps the first part's. WebM is always AV1 or
    /// VP9.
    pub codec: Option<VideoCodec>,
    /// Picture size; `None` keeps the first part's. Other parts are fitted into it with bars.
    pub size: Option<(u32, u32)>,
    /// Frames per second; `None` keeps the first part's (15 for GIF).
    pub fps: Option<u32>,
    /// Video bitrate in bit/s; `None` follows the first part's.
    pub video_bitrate: Option<i64>,
    pub audio_kbps: u32,
    /// Width of a GIF; its height follows.
    pub gif_width: u32,
    pub fades: Fades,
    /// Decoder threads, 0 for one per core.
    pub threads: u32,
}

/// Seconds out for seconds in, through the speed changes.
#[derive(Clone, Debug, Default)]
pub struct TimeMap {
    speeds: Vec<SpeedRange>,
}

impl TimeMap {
    pub fn new(speeds: &[SpeedRange]) -> Self {
        let mut speeds: Vec<SpeedRange> = speeds
            .iter()
            .copied()
            .filter(|s| s.end > s.start && s.speed > 0.0 && (s.speed - 1.0).abs() > 1e-6)
            .map(|s| SpeedRange {
                speed: s.speed.clamp(0.1, 16.0),
                ..s
            })
            .collect();
        speeds.sort_by(|a, b| a.start.total_cmp(&b.start));
        Self { speeds }
    }

    pub fn speed_at(&self, t: f64) -> f64 {
        self.speeds
            .iter()
            .find(|s| t >= s.start && t < s.end)
            .map_or(1.0, |s| s.speed)
    }

    /// Where input time `t` lands in the output.
    pub fn out_of(&self, t: f64) -> f64 {
        let mut out = t;
        for s in &self.speeds {
            if t <= s.start {
                break;
            }
            let covered = t.min(s.end) - s.start;
            out += covered / s.speed - covered;
        }
        out
    }
}

/// The fade at output time `t` (0 black and silent, 1 untouched).
fn fade_at(fades: &Fades, t: f64, total: f64, joins: &[f64]) -> f32 {
    let mut level = 1.0f64;
    if fades.fade_in > 0.0 && t < fades.fade_in {
        level = level.min(t / fades.fade_in);
    }
    if fades.fade_out > 0.0 && total - t < fades.fade_out {
        level = level.min((total - t) / fades.fade_out);
    }
    if fades.between > 0.0 {
        for &join in joins {
            let d = (t - join).abs();
            if d < fades.between {
                level = level.min(d / fades.between);
            }
        }
    }
    level.clamp(0.0, 1.0) as f32
}

struct Probe {
    duration: f64,
    width: u32,
    height: u32,
    fps: f64,
    codec: Option<VideoCodec>,
    bits_per_sec: f64,
}

fn probe(path: &Path) -> Result<Probe> {
    let input = format::input(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let duration = input.duration().max(0) as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
    let video = crate::library::screen_stream(&input);
    let (width, height, fps, codec) = match &video {
        Some(stream) => {
            let params = stream.parameters();
            let (w, h) = unsafe { ((*params.as_ptr()).width, (*params.as_ptr()).height) };
            let rate = crate::library::frame_rate(&input, stream);
            let fps = if rate.denominator() > 0 && rate.numerator() > 0 {
                rate.numerator() as f64 / rate.denominator() as f64
            } else {
                60.0
            };
            let codec = match params.id() {
                codec::Id::H264 => Some(VideoCodec::H264),
                codec::Id::HEVC => Some(VideoCodec::Hevc),
                codec::Id::AV1 => Some(VideoCodec::Av1),
                _ => None,
            };
            (w.max(2) as u32, h.max(2) as u32, fps, codec)
        }
        None => (0, 0, 0.0, None),
    };
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    Ok(Probe {
        duration,
        width,
        height,
        fps,
        codec,
        bits_per_sec: if duration > 0.0 {
            size as f64 * 8.0 / duration
        } else {
            0.0
        },
    })
}

fn check(code: i32, what: &str) -> Result<i32> {
    if code < 0 {
        Err(anyhow!("{what}: {}", ffmpeg_next::Error::from(code)))
    } else {
        Ok(code)
    }
}

unsafe fn set_opt(ctx: *mut AVCodecContext, key: &str, value: &str) {
    unsafe {
        if (*ctx).priv_data.is_null() {
            return;
        }
        let (Ok(k), Ok(v)) = (std::ffi::CString::new(key), std::ffi::CString::new(value)) else {
            return;
        };
        av_opt_set((*ctx).priv_data, k.as_ptr(), v.as_ptr(), 0);
    }
}

// ---------------------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------------------

struct Encoder {
    ctx: *mut AVCodecContext,
    name: String,
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { avcodec_free_context(&mut self.ctx) };
    }
}

impl Encoder {
    fn open(names: &[&str], configure: &dyn Fn(*mut AVCodecContext, &str)) -> Result<Self> {
        let mut failures = Vec::new();
        for name in names {
            unsafe {
                let c_name = std::ffi::CString::new(*name)?;
                let codec = avcodec_find_encoder_by_name(c_name.as_ptr());
                if codec.is_null() {
                    failures.push(format!("{name}: not in this FFmpeg"));
                    continue;
                }
                let ctx = avcodec_alloc_context3(codec);
                if ctx.is_null() {
                    bail!("avcodec_alloc_context3 failed");
                }
                let encoder = Self {
                    ctx,
                    name: name.to_string(),
                };
                configure(ctx, name);
                (*ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                match check(avcodec_open2(ctx, codec, ptr::null_mut()), "open") {
                    Ok(_) => return Ok(encoder),
                    Err(err) => failures.push(format!("{name}: {err:#}")),
                }
            }
        }
        bail!("no encoder could be opened ({})", failures.join("; "))
    }

    fn stream_info(&self) -> Result<StreamInfo> {
        let mut params = codec::Parameters::new();
        unsafe {
            check(
                avcodec_parameters_from_context(params.as_mut_ptr(), self.ctx),
                "avcodec_parameters_from_context",
            )?;
            Ok(StreamInfo {
                params,
                time_base: Rational::new((*self.ctx).time_base.num, (*self.ctx).time_base.den),
                title: None,
            })
        }
    }

    /// Sends a frame (null flushes) and writes what comes out as `stream`.
    fn send(&mut self, frame: *const AVFrame, stream: usize, out: &mut Mp4Writer) -> Result<()> {
        unsafe {
            loop {
                let code = avcodec_send_frame(self.ctx, frame);
                if code == AVERROR_EAGAIN {
                    self.drain(stream, out)?;
                    continue;
                }
                if code == AVERROR_EOF && frame.is_null() {
                    break;
                }
                check(code, "avcodec_send_frame")?;
                break;
            }
            self.drain(stream, out)
        }
    }

    unsafe fn drain(&mut self, stream: usize, out: &mut Mp4Writer) -> Result<()> {
        unsafe {
            loop {
                let mut packet = Packet::empty();
                let code = avcodec_receive_packet(self.ctx, packet.as_mut_ptr());
                if code == AVERROR_EAGAIN || code == AVERROR_EOF {
                    return Ok(());
                }
                check(code, "avcodec_receive_packet")?;
                out.write(stream, &packet, 0)?;
            }
        }
    }
}

fn video_encoder_names(format: OutputFormat, codec: VideoCodec) -> &'static [&'static str] {
    if format == OutputFormat::Webm {
        return &["av1_nvenc", "av1_amf", "av1_qsv", "libvpx-vp9"];
    }
    match codec {
        VideoCodec::H264 => &["h264_nvenc", "h264_amf", "h264_qsv", "libopenh264"],
        VideoCodec::Hevc => &["hevc_nvenc", "hevc_amf", "hevc_qsv", "libkvazaar"],
        VideoCodec::Av1 => &["av1_nvenc", "av1_amf", "av1_qsv", "libsvtav1"],
    }
}

fn open_video(
    format: OutputFormat,
    codec: VideoCodec,
    (width, height): (u32, u32),
    fps: u32,
    bit_rate: i64,
) -> Result<Encoder> {
    Encoder::open(video_encoder_names(format, codec), &|ctx, name| unsafe {
        (*ctx).width = width as i32;
        (*ctx).height = height as i32;
        (*ctx).pix_fmt = AVPixelFormat::AV_PIX_FMT_YUV420P;
        (*ctx).time_base = AVRational {
            num: 1,
            den: fps as i32,
        };
        (*ctx).framerate = AVRational {
            num: fps as i32,
            den: 1,
        };
        (*ctx).gop_size = fps.max(1) as i32;
        (*ctx).max_b_frames = 0;
        (*ctx).bit_rate = bit_rate;
        (*ctx).color_range = ffmpeg_next::ffi::AVColorRange::AVCOL_RANGE_MPEG;
        if name.ends_with("_nvenc") {
            set_opt(ctx, "preset", "p5");
            set_opt(ctx, "tune", "hq");
            set_opt(ctx, "rc", "vbr");
            (*ctx).rc_max_rate = bit_rate * 3 / 2;
        } else if name == "libvpx-vp9" {
            set_opt(ctx, "deadline", "realtime");
            set_opt(ctx, "cpu-used", "8");
            set_opt(ctx, "row-mt", "1");
        } else if name == "libopenh264" {
            set_opt(ctx, "rc_mode", "bitrate");
            set_opt(ctx, "allow_skip_frames", "0");
        } else if name == "libsvtav1" {
            set_opt(ctx, "preset", "8");
        }
    })
}

/// Turns stereo float samples into the encoder's frames.
struct AudioOut {
    encoder: Encoder,
    frame: *mut AVFrame,
    format: AVSampleFormat,
    frame_size: usize,
    pending: Vec<f32>,
    pts: i64,
    stream: usize,
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        unsafe { av_frame_free(&mut self.frame) };
    }
}

fn open_audio(format: OutputFormat, kbps: u32) -> Result<Encoder> {
    Encoder::open(format.audio_encoders(), &|ctx, name| unsafe {
        let codec = (*ctx).codec;
        // The first sample format the encoder takes from this list.
        let wanted = [
            AVSampleFormat::AV_SAMPLE_FMT_FLTP,
            AVSampleFormat::AV_SAMPLE_FMT_FLT,
            AVSampleFormat::AV_SAMPLE_FMT_S16P,
            AVSampleFormat::AV_SAMPLE_FMT_S16,
            AVSampleFormat::AV_SAMPLE_FMT_S32P,
            AVSampleFormat::AV_SAMPLE_FMT_S32,
        ];
        let mut supported = Vec::new();
        #[allow(deprecated)]
        let list = (*codec).sample_fmts;
        if !list.is_null() {
            let mut i = 0;
            while *list.add(i) != AVSampleFormat::AV_SAMPLE_FMT_NONE {
                supported.push(*list.add(i));
                i += 1;
            }
        }
        (*ctx).sample_fmt = wanted
            .into_iter()
            .find(|f| supported.is_empty() || supported.contains(f))
            .unwrap_or(AVSampleFormat::AV_SAMPLE_FMT_FLTP);
        (*ctx).sample_rate = RATE as i32;
        av_channel_layout_default(&mut (*ctx).ch_layout, 2);
        (*ctx).time_base = AVRational {
            num: 1,
            den: RATE as i32,
        };
        if !name.starts_with("pcm") && name != "flac" {
            (*ctx).bit_rate = kbps as i64 * 1000;
        }
    })
}

impl AudioOut {
    fn new(encoder: Encoder, stream: usize) -> Result<Self> {
        unsafe {
            let ctx = encoder.ctx;
            let frame_size = match (*ctx).frame_size {
                n if n > 0 => n as usize,
                _ => 1024,
            };
            let frame = av_frame_alloc();
            if frame.is_null() {
                bail!("av_frame_alloc failed");
            }
            (*frame).format = (*ctx).sample_fmt as i32;
            (*frame).nb_samples = frame_size as i32;
            (*frame).sample_rate = RATE as i32;
            check(
                av_channel_layout_copy(&mut (*frame).ch_layout, &(*ctx).ch_layout),
                "av_channel_layout_copy",
            )?;
            check(av_frame_get_buffer(frame, 0), "av_frame_get_buffer")?;
            Ok(Self {
                format: (*ctx).sample_fmt,
                encoder,
                frame,
                frame_size,
                pending: Vec::new(),
                pts: 0,
                stream,
            })
        }
    }

    /// Adds interleaved stereo samples; full frames are encoded as they fill.
    fn push(&mut self, samples: &[f32], out: &mut Mp4Writer) -> Result<()> {
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= self.frame_size * 2 {
            let chunk: Vec<f32> = self.pending.drain(..self.frame_size * 2).collect();
            self.encode(&chunk, self.frame_size, out)?;
        }
        Ok(())
    }

    fn encode(&mut self, chunk: &[f32], count: usize, out: &mut Mp4Writer) -> Result<()> {
        unsafe {
            let frame = self.frame;
            check(av_frame_make_writable(frame), "av_frame_make_writable")?;
            (*frame).nb_samples = count as i32;
            let s16 = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i16;
            let s32 = |v: f32| (v.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0) as i32;
            match self.format {
                AVSampleFormat::AV_SAMPLE_FMT_FLTP => {
                    for c in 0..2 {
                        let plane =
                            std::slice::from_raw_parts_mut((*frame).data[c] as *mut f32, count);
                        for i in 0..count {
                            plane[i] = chunk[i * 2 + c];
                        }
                    }
                }
                AVSampleFormat::AV_SAMPLE_FMT_FLT => {
                    let data =
                        std::slice::from_raw_parts_mut((*frame).data[0] as *mut f32, count * 2);
                    data.copy_from_slice(&chunk[..count * 2]);
                }
                AVSampleFormat::AV_SAMPLE_FMT_S16P => {
                    for c in 0..2 {
                        let plane =
                            std::slice::from_raw_parts_mut((*frame).data[c] as *mut i16, count);
                        for i in 0..count {
                            plane[i] = s16(chunk[i * 2 + c]);
                        }
                    }
                }
                AVSampleFormat::AV_SAMPLE_FMT_S16 => {
                    let data =
                        std::slice::from_raw_parts_mut((*frame).data[0] as *mut i16, count * 2);
                    for (d, &v) in data.iter_mut().zip(chunk) {
                        *d = s16(v);
                    }
                }
                AVSampleFormat::AV_SAMPLE_FMT_S32P => {
                    for c in 0..2 {
                        let plane =
                            std::slice::from_raw_parts_mut((*frame).data[c] as *mut i32, count);
                        for i in 0..count {
                            plane[i] = s32(chunk[i * 2 + c]);
                        }
                    }
                }
                _ => {
                    let data =
                        std::slice::from_raw_parts_mut((*frame).data[0] as *mut i32, count * 2);
                    for (d, &v) in data.iter_mut().zip(chunk) {
                        *d = s32(v);
                    }
                }
            }
            (*frame).pts = self.pts;
            self.pts += count as i64;
            self.encoder.send(frame, self.stream, out)
        }
    }

    fn finish(&mut self, out: &mut Mp4Writer) -> Result<()> {
        let rest = std::mem::take(&mut self.pending);
        if !rest.is_empty() {
            // Encoders with a fixed frame size get the last frame padded with silence.
            let fixed = unsafe { (*self.encoder.ctx).frame_size } > 0
                && !self.encoder.name.starts_with("pcm")
                && self.encoder.name != "flac";
            let count = if fixed {
                self.frame_size
            } else {
                rest.len() / 2
            };
            let mut chunk = rest;
            chunk.resize(count * 2, 0.0);
            self.encode(&chunk, count, out)?;
        }
        self.encoder.send(ptr::null(), self.stream, out)
    }
}

// ---------------------------------------------------------------------------------------
// Pictures
// ---------------------------------------------------------------------------------------

/// Converts decoded frames to the output picture: the size and pixel format the encoder
/// wants, a part of another shape fitted in the middle with black bars.
struct Picture {
    size: (u32, u32),
    format: Pixel,
    scaler: Option<(scaling::Context, Pixel, u32, u32, (u32, u32))>,
}

impl Picture {
    fn new(size: (u32, u32), format: Pixel) -> Self {
        Self {
            size,
            format,
            scaler: None,
        }
    }

    fn convert(&mut self, decoded: &frame::Video) -> Result<frame::Video> {
        let (sw, sh) = (decoded.width(), decoded.height());
        let (ow, oh) = self.size;
        // The largest even size of the source's shape that fits.
        let scale = (ow as f64 / sw as f64).min(oh as f64 / sh as f64);
        let fit = (
            (((sw as f64 * scale).round() as u32) & !1).clamp(2, ow),
            (((sh as f64 * scale).round() as u32) & !1).clamp(2, oh),
        );
        let stale = self
            .scaler
            .as_ref()
            .is_none_or(|(_, f, w, h, to)| (*f, *w, *h, *to) != (decoded.format(), sw, sh, fit));
        if stale {
            let flags = if fit.0 < sw {
                scaling::Flags::LANCZOS
            } else {
                scaling::Flags::BICUBIC
            };
            let context =
                scaling::Context::get(decoded.format(), sw, sh, self.format, fit.0, fit.1, flags)
                    .context("cannot convert the picture")?;
            self.scaler = Some((context, decoded.format(), sw, sh, fit));
        }
        let (context, ..) = self.scaler.as_mut().unwrap();
        let mut scaled = frame::Video::new(self.format, fit.0, fit.1);
        context
            .run(decoded, &mut scaled)
            .context("cannot convert the frame")?;
        if fit == self.size {
            return Ok(scaled);
        }
        let mut canvas = black(self.format, ow, oh);
        let (x, y) = (((ow - fit.0) / 2) & !1, ((oh - fit.1) / 2) & !1);
        blit(&scaled, &mut canvas, x as usize, y as usize);
        Ok(canvas)
    }
}

fn black(format: Pixel, w: u32, h: u32) -> frame::Video {
    let mut f = frame::Video::new(format, w, h);
    if format == Pixel::YUV420P {
        for (plane, value) in [(0, 16u8), (1, 128), (2, 128)] {
            f.data_mut(plane).fill(value);
        }
    } else {
        f.data_mut(0).fill(0);
        if format == Pixel::RGBA {
            for px in f.data_mut(0).chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
    }
    f
}

/// Copies `src` into `dst` at (`x`, `y`), plane by plane (chroma planes at half the offset).
fn blit(src: &frame::Video, dst: &mut frame::Video, x: usize, y: usize) {
    let planes = if src.format() == Pixel::YUV420P { 3 } else { 1 };
    let bytes = if src.format() == Pixel::RGBA { 4 } else { 1 };
    for p in 0..planes {
        let (div, w, h) = if p == 0 {
            (1, src.width() as usize, src.height() as usize)
        } else {
            (2, src.width() as usize / 2, src.height() as usize / 2)
        };
        let (ss, ds) = (src.stride(p), dst.stride(p));
        let (sx, sy) = (x / div, y / div);
        let src_data = src.data(p).to_vec();
        let dst_data = dst.data_mut(p);
        for row in 0..h {
            let from = &src_data[row * ss..row * ss + w * bytes];
            let at = (sy + row) * ds + sx * bytes;
            dst_data[at..at + w * bytes].copy_from_slice(from);
        }
    }
}

/// A copy of `frame` faded toward black by `level` (0 black, 1 as it is).
fn faded(frame: &frame::Video, level: f32) -> frame::Video {
    let mut out = frame.clone();
    if frame.format() == Pixel::YUV420P {
        for (p, base) in [(0usize, 16.0f32), (1, 128.0), (2, 128.0)] {
            for v in out.data_mut(p).iter_mut() {
                *v = (base + (*v as f32 - base) * level)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    } else {
        for px in out.data_mut(0).chunks_exact_mut(4) {
            for c in &mut px[..3] {
                *c = (*c as f32 * level).round() as u8;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Audio in
// ---------------------------------------------------------------------------------------

/// Decoded samples as interleaved stereo float, whatever the decoder gave.
fn stereo(decoded: &frame::Audio) -> Vec<f32> {
    use ffmpeg_next::format::Sample;
    use ffmpeg_next::format::sample::Type;
    let count = decoded.samples();
    let channels = decoded.channels().max(1) as usize;
    let mut out = vec![0f32; count * 2];
    let read = |c: usize, i: usize| -> f32 {
        unsafe {
            match decoded.format() {
                Sample::F32(Type::Planar) => decoded.plane::<f32>(c)[i],
                Sample::F32(Type::Packed) => {
                    *(decoded.data(0).as_ptr() as *const f32).add(i * channels + c)
                }
                Sample::I16(Type::Planar) => decoded.plane::<i16>(c)[i] as f32 / 32768.0,
                Sample::I16(Type::Packed) => {
                    *(decoded.data(0).as_ptr() as *const i16).add(i * channels + c) as f32 / 32768.0
                }
                Sample::I32(Type::Planar) => decoded.plane::<i32>(c)[i] as f32 / 2_147_483_648.0,
                Sample::F64(Type::Planar) => decoded.plane::<f64>(c)[i] as f32,
                _ => 0.0,
            }
        }
    };
    for i in 0..count {
        let left = read(0, i);
        let right = if channels > 1 { read(1, i) } else { left };
        out[i * 2] = left;
        out[i * 2 + 1] = right;
    }
    out
}

/// Reads a part's audio at the speed of each moment, at `RATE`, by linear interpolation.
struct Retime<'a> {
    map: &'a TimeMap,
    source_rate: f64,
    /// Interleaved stereo input not yet passed; `first` is the input index of its start.
    input: Vec<f32>,
    first: usize,
    /// Read position in input samples.
    position: f64,
}

impl Retime<'_> {
    fn add(&mut self, samples: &[f32]) {
        self.input.extend_from_slice(samples);
    }

    /// Produces output samples while there is input to read; `all` reads to the very end.
    fn take(&mut self, out: &mut Vec<f32>, limit: usize) {
        let available = self.first + self.input.len() / 2;
        while out.len() / 2 < limit {
            let i = self.position.floor() as usize;
            if i + 1 >= available {
                break;
            }
            let frac = (self.position - i as f64) as f32;
            let at = (i - self.first) * 2;
            for c in 0..2 {
                let a = self.input[at + c];
                let b = self.input[at + 2 + c];
                out.push(a + (b - a) * frac);
            }
            let speed = self.map.speed_at(self.position / self.source_rate);
            self.position += speed * self.source_rate / RATE as f64;
        }
        // Let go of what has been read.
        let keep_from = (self.position.floor() as usize).saturating_sub(1);
        if keep_from > self.first + 4096 {
            let drop = keep_from - self.first;
            self.input.drain(..drop * 2);
            self.first = keep_from;
        }
    }
}

// ---------------------------------------------------------------------------------------
// The render
// ---------------------------------------------------------------------------------------

struct VideoOut {
    encoder: Option<Encoder>,
    gif: Option<gif::Encoder<BufWriter<File>>>,
    stream: usize,
    fps: f64,
    /// The next output frame's number.
    next: i64,
    /// GIF delays add up to whole hundredths; what is left over carries to the next frame.
    gif_carry: f64,
}

impl VideoOut {
    fn emit(
        &mut self,
        picture: &frame::Video,
        level: f32,
        out: Option<&mut Mp4Writer>,
    ) -> Result<()> {
        let mut shown = if level < 0.999 {
            faded(picture, level)
        } else {
            picture.clone()
        };
        if let Some(gif) = &mut self.gif {
            let (w, h) = (shown.width() as u16, shown.height() as u16);
            let stride = shown.stride(0);
            let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
            for row in 0..h as usize {
                rgba.extend_from_slice(&shown.data(0)[row * stride..row * stride + w as usize * 4]);
            }
            let mut f = gif::Frame::from_rgba_speed(w, h, &mut rgba, 10);
            let delay = 100.0 / self.fps + self.gif_carry;
            f.delay = delay.floor() as u16;
            self.gif_carry = delay - f.delay as f64;
            gif.write_frame(&f).context("cannot write the GIF")?;
        } else if let (Some(encoder), Some(out)) = (&mut self.encoder, out) {
            unsafe { (*shown.as_mut_ptr()).pts = self.next };
            encoder.send(unsafe { shown.as_ptr() }, self.stream, out)?;
        }
        self.next += 1;
        Ok(())
    }
}

/// Renders `job`. `progress` gets 0 to 1 and returns false to cancel.
pub fn render(job: &RenderJob, progress: &mut dyn FnMut(f32) -> bool) -> Result<()> {
    ffmpeg_next::init()?;
    if job.parts.is_empty() {
        bail!("there is nothing to render");
    }
    let probes: Vec<Probe> = job
        .parts
        .iter()
        .map(|p| probe(&p.path))
        .collect::<Result<_>>()?;
    let maps: Vec<TimeMap> = job.parts.iter().map(|p| TimeMap::new(&p.speeds)).collect();
    let lengths: Vec<f64> = probes
        .iter()
        .zip(&maps)
        .map(|(p, m)| m.out_of(p.duration))
        .collect();
    let total: f64 = lengths.iter().sum();
    if total <= 0.01 {
        bail!("the edit is empty");
    }
    let mut joins = Vec::new();
    let mut offsets = Vec::new();
    let mut at = 0.0;
    for len in &lengths {
        offsets.push(at);
        at += len;
        joins.push(at);
    }
    joins.pop();

    let first = &probes[0];
    let video_wanted = job.format.has_video() && first.width > 0;
    let gif = job.format == OutputFormat::Gif;
    let fps = match (job.fps, gif) {
        (Some(f), _) => f.max(1),
        (None, true) => 15,
        (None, false) => (first.fps.round() as u32).clamp(1, 360),
    };
    let size = if gif {
        let w = job.gif_width.clamp(64, first.width.max(64)) & !1;
        let h = ((first.height as f64 * w as f64 / first.width.max(1) as f64).round() as u32 & !1)
            .max(2);
        (w, h)
    } else {
        let (w, h) = job.size.unwrap_or((first.width, first.height));
        (w.max(2) & !1, h.max(2) & !1)
    };

    let partial = job
        .output
        .with_extension(format!("{}.part", job.format.extension()));
    let _ = std::fs::remove_file(&partial);
    let result = (|| -> Result<()> {
        // Encoders, then the file.
        let mut streams = Vec::new();
        let mut video = VideoOut {
            encoder: None,
            gif: None,
            stream: 0,
            fps: fps as f64,
            next: 0,
            gif_carry: 0.0,
        };
        if video_wanted && !gif {
            let codec = job.codec.or(first.codec).unwrap_or(VideoCodec::H264);
            let bit_rate = job.video_bitrate.unwrap_or_else(|| {
                // About the first part's bits per pixel, without its audio.
                let area = (size.0 * size.1) as f64 / (first.width * first.height).max(1) as f64;
                let frames = (fps as f64 / first.fps.max(1.0)).max(0.35);
                ((first.bits_per_sec - 200_000.0).max(2_000_000.0) * area * frames) as i64
            });
            let encoder = open_video(
                job.format,
                codec,
                size,
                fps,
                bit_rate.clamp(300_000, 400_000_000),
            )?;
            log::info!("rendering video with {}", encoder.name);
            video.stream = streams.len();
            streams.push(encoder.stream_info()?);
            video.encoder = Some(encoder);
        }
        let mut audio = None;
        if job.format.has_audio() {
            let encoder = open_audio(job.format, job.audio_kbps.max(64))?;
            let stream = streams.len();
            streams.push(encoder.stream_info()?);
            audio = Some(AudioOut::new(encoder, stream)?);
        }
        let mut writer = if gif {
            let file = File::create(&partial)
                .with_context(|| format!("cannot create {}", partial.display()))?;
            let mut encoder =
                gif::Encoder::new(BufWriter::new(file), size.0 as u16, size.1 as u16, &[])
                    .context("cannot start the GIF")?;
            encoder.set_repeat(gif::Repeat::Infinite).ok();
            video.gif = Some(encoder);
            None
        } else {
            Some(Mp4Writer::create_container(
                &partial,
                &streams,
                job.format.container(),
            )?)
        };

        let mut picture = Picture::new(size, if gif { Pixel::RGBA } else { Pixel::YUV420P });
        // Audio samples written so far, over all parts.
        let mut samples_out: usize = 0;

        for (index, part) in job.parts.iter().enumerate() {
            let map = &maps[index];
            let offset = offsets[index];
            let end = offset + lengths[index];
            let mut input = format::input(&part.path)
                .with_context(|| format!("cannot open {}", part.path.display()))?;
            let mut video_in = None;
            if video_wanted {
                if let Some(stream) = input.streams().best(media::Type::Video) {
                    let mut ctx = codec::context::Context::from_parameters(stream.parameters())?;
                    ctx.set_threading(codec::threading::Config {
                        kind: codec::threading::Type::Frame,
                        count: job.threads as usize,
                        ..Default::default()
                    });
                    // On the graphics card when it can; frames come back to memory below.
                    unsafe { crate::export::hw_decode::attach(ctx.as_mut_ptr()) };
                    video_in = Some((stream.index(), stream.time_base(), ctx.decoder().video()?));
                }
            }
            let mut audio_in = None;
            if audio.is_some() {
                if let Some(stream) = input.streams().best(media::Type::Audio) {
                    let decoder = codec::context::Context::from_parameters(stream.parameters())?
                        .decoder()
                        .audio()?;
                    let rate = decoder.rate().max(1) as f64;
                    audio_in = Some((stream.index(), stream.time_base(), decoder, rate));
                }
            }
            let mut retime = Retime {
                map,
                source_rate: audio_in.as_ref().map_or(RATE as f64, |a| a.3),
                input: Vec::new(),
                first: 0,
                position: 0.0,
            };
            let mut held: Option<frame::Video> = None;
            let mut decoded = frame::Video::empty();
            let mut downloaded = frame::Video::empty();
            let mut decoded_audio = frame::Audio::empty();
            let part_end_sample = (end * RATE as f64).round() as usize;
            // Silence until where this part starts, if the last one ended short.
            let start_sample = (offset * RATE as f64).round() as usize;

            let mut on_video = |frame: &frame::Video,
                                tb: Rational,
                                held: &mut Option<frame::Video>,
                                video: &mut VideoOut,
                                writer: &mut Option<Mp4Writer>|
             -> Result<()> {
                let ts = frame.timestamp().or(frame.pts()).unwrap_or(0);
                let secs = (ts as f64 * f64::from(tb)).max(0.0);
                let mapped = offset + map.out_of(secs);
                // The frame before shows until this one's time.
                if let Some(previous) = held.as_ref() {
                    while (video.next as f64) / video.fps < mapped.min(end) {
                        let t = video.next as f64 / video.fps;
                        video.emit(
                            previous,
                            fade_at(&job.fades, t, total, &joins),
                            writer.as_mut(),
                        )?;
                    }
                }
                *held = Some(picture.convert(frame)?);
                Ok(())
            };

            let flush_audio = |retime: &mut Retime,
                               audio: &mut Option<AudioOut>,
                               writer: &mut Option<Mp4Writer>,
                               samples_out: &mut usize,
                               all: bool|
             -> Result<()> {
                let Some(audio) = audio.as_mut() else {
                    return Ok(());
                };
                let mut buffer = Vec::new();
                if *samples_out < start_sample {
                    buffer.resize((start_sample - *samples_out) * 2, 0.0);
                    *samples_out = start_sample;
                }
                let mut produced = Vec::new();
                retime.take(&mut produced, part_end_sample.saturating_sub(*samples_out));
                buffer.extend_from_slice(&produced);
                *samples_out += produced.len() / 2;
                if all && *samples_out < part_end_sample {
                    buffer.resize(buffer.len() + (part_end_sample - *samples_out) * 2, 0.0);
                    *samples_out = part_end_sample;
                }
                // Fades, by each sample's time.
                let first_sample = *samples_out - buffer.len() / 2;
                for (i, pair) in buffer.chunks_exact_mut(2).enumerate() {
                    let t = (first_sample + i) as f64 / RATE as f64;
                    let level = fade_at(&job.fades, t, total, &joins);
                    if level < 0.9999 {
                        pair[0] *= level;
                        pair[1] *= level;
                    }
                }
                if let Some(out) = writer.as_mut() {
                    audio.push(&buffer, out)?;
                }
                Ok(())
            };

            for (stream, packet) in input.packets() {
                if let Some((v_index, tb, decoder)) = video_in.as_mut() {
                    if stream.index() == *v_index {
                        decoder
                            .send_packet(&packet)
                            .context("cannot decode the video")?;
                        while decoder.receive_frame(&mut decoded).is_ok() {
                            let frame =
                                crate::export::hw_decode::download(&decoded, &mut downloaded)?;
                            on_video(frame, *tb, &mut held, &mut video, &mut writer)?;
                        }
                        let t = video.next as f64 / video.fps;
                        if !progress((t / total) as f32) {
                            return Err(anyhow!(crate::export::Cancelled));
                        }
                        continue;
                    }
                }
                if let Some((a_index, _tb, decoder, _)) = audio_in.as_mut() {
                    if stream.index() == *a_index {
                        if decoder.send_packet(&packet).is_ok() {
                            while decoder.receive_frame(&mut decoded_audio).is_ok() {
                                retime.add(&stereo(&decoded_audio));
                            }
                        }
                        flush_audio(
                            &mut retime,
                            &mut audio,
                            &mut writer,
                            &mut samples_out,
                            false,
                        )?;
                        if video_in.is_none() {
                            let t = samples_out as f64 / RATE as f64;
                            if !progress((t / total) as f32) {
                                return Err(anyhow!(crate::export::Cancelled));
                            }
                        }
                    }
                }
            }
            // The rest of this part.
            if let Some((_, tb, decoder)) = video_in.as_mut() {
                let _ = decoder.send_eof();
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let frame = crate::export::hw_decode::download(&decoded, &mut downloaded)?;
                    on_video(frame, *tb, &mut held, &mut video, &mut writer)?;
                }
                if let Some(last) = held.take() {
                    while (video.next as f64) / video.fps < end - 1e-9 {
                        let t = video.next as f64 / video.fps;
                        video.emit(
                            &last,
                            fade_at(&job.fades, t, total, &joins),
                            writer.as_mut(),
                        )?;
                    }
                }
            }
            if let Some((_, _, decoder, _)) = audio_in.as_mut() {
                let _ = decoder.send_eof();
                while decoder.receive_frame(&mut decoded_audio).is_ok() {
                    retime.add(&stereo(&decoded_audio));
                }
            }
            flush_audio(&mut retime, &mut audio, &mut writer, &mut samples_out, true)?;
        }

        // Flush the encoders and close the file.
        if let Some(gif) = video.gif.take() {
            gif.into_inner()
                .context("cannot finish the GIF")?
                .into_inner()
                .map_err(|e| anyhow!("cannot finish the GIF: {e}"))?;
        }
        if let Some(mut out) = writer {
            if let Some(encoder) = video.encoder.as_mut() {
                encoder.send(ptr::null(), video.stream, &mut out)?;
            }
            if let Some(audio) = audio.as_mut() {
                audio.finish(&mut out)?;
            }
            out.finish()?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&partial, &job.output)
                .with_context(|| format!("cannot name the file {}", job.output.display()))?;
            progress(1.0);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testutil::{TempDir, make_clip};

    #[test]
    fn time_map_stretches_and_squeezes() {
        let map = TimeMap::new(&[
            SpeedRange {
                start: 1.0,
                end: 2.0,
                speed: 0.5,
            },
            SpeedRange {
                start: 3.0,
                end: 5.0,
                speed: 2.0,
            },
        ]);
        assert_eq!(map.out_of(0.5), 0.5);
        assert_eq!(map.out_of(1.5), 2.0);
        assert_eq!(map.out_of(2.0), 3.0);
        assert_eq!(map.out_of(3.0), 4.0);
        assert_eq!(map.out_of(5.0), 5.0);
        assert_eq!(map.out_of(6.0), 6.0);
        assert_eq!(map.speed_at(1.2), 0.5);
        assert_eq!(map.speed_at(2.5), 1.0);
    }

    #[test]
    fn fades_in_out_and_between() {
        let fades = Fades {
            fade_in: 1.0,
            fade_out: 2.0,
            between: 0.5,
        };
        let joins = [5.0];
        assert_eq!(fade_at(&fades, 0.0, 10.0, &joins), 0.0);
        assert!((fade_at(&fades, 0.5, 10.0, &joins) - 0.5).abs() < 1e-6);
        assert_eq!(fade_at(&fades, 3.0, 10.0, &joins), 1.0);
        assert_eq!(fade_at(&fades, 5.0, 10.0, &joins), 0.0);
        assert!((fade_at(&fades, 5.25, 10.0, &joins) - 0.5).abs() < 1e-6);
        assert!((fade_at(&fades, 9.0, 10.0, &joins) - 0.5).abs() < 1e-6);
    }

    fn duration_of(path: &Path) -> f64 {
        let input = format::input(&path).unwrap();
        input.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE)
    }

    fn job(parts: Vec<RenderPart>, output: PathBuf, format: OutputFormat) -> RenderJob {
        RenderJob {
            parts,
            output,
            format,
            codec: None,
            size: None,
            fps: None,
            video_bitrate: None,
            audio_kbps: 128,
            gif_width: 320,
            fades: Fades::default(),
            threads: 0,
        }
    }

    #[test]
    fn slow_motion_and_two_parts_add_up() {
        let dir = TempDir::new("render");
        let clip = dir.0.join("clip.mp4");
        make_clip(&clip, 3);
        let out = dir.0.join("out.mp4");
        let parts = vec![
            // 1 to 2 s at half speed: 3 s become 4 s.
            RenderPart {
                path: clip.clone(),
                speeds: vec![SpeedRange {
                    start: 1.0,
                    end: 2.0,
                    speed: 0.5,
                }],
            },
            RenderPart {
                path: clip.clone(),
                speeds: vec![],
            },
        ];
        let mut j = job(parts, out.clone(), OutputFormat::Mp4);
        j.fades = Fades {
            fade_in: 0.5,
            fade_out: 0.5,
            between: 0.3,
        };
        render(&j, &mut |_| true).unwrap();
        let secs = duration_of(&out);
        assert!((secs - 7.0).abs() < 0.15, "{secs}");
        let input = format::input(&out).unwrap();
        assert_eq!(input.streams().count(), 2);
    }

    #[test]
    fn other_formats() {
        let dir = TempDir::new("render-formats");
        let clip = dir.0.join("clip.mp4");
        make_clip(&clip, 2);
        for format in [
            OutputFormat::Mov,
            OutputFormat::Mkv,
            OutputFormat::Webm,
            OutputFormat::Mp3,
            OutputFormat::M4a,
            OutputFormat::Wav,
            OutputFormat::Flac,
            OutputFormat::Opus,
            OutputFormat::Gif,
        ] {
            let out = dir.0.join(format!("out.{}", format.extension()));
            let j = job(
                vec![RenderPart {
                    path: clip.clone(),
                    speeds: vec![],
                }],
                out.clone(),
                format,
            );
            render(&j, &mut |_| true).unwrap_or_else(|e| panic!("{format:?}: {e:#}"));
            let bytes = std::fs::metadata(&out).unwrap().len();
            assert!(bytes > 1000, "{format:?}: {bytes} bytes");
            if format != OutputFormat::Gif {
                let secs = duration_of(&out);
                assert!((secs - 2.0).abs() < 0.2, "{format:?}: {secs}");
            }
        }
    }
}
