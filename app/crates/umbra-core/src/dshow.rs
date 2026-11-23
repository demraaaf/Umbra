//! Cameras only DirectShow knows: OBS's virtual camera and other software cameras, which
//! Media Foundation (`camera`, Umbra's main path) does not list. Read through DirectShow's
//! Sample Grabber into memory, converted to BGRA there, and from then on filmed like any
//! camera. Devices both list (a USB webcam) are left to Media Foundation.

// The Sample Grabber's interfaces keep the method names Windows gives them.
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::ffi::{
    SWS_CS_DEFAULT, SWS_CS_ITU709, sws_getCoefficients, sws_setColorspaceDetails,
};
use ffmpeg_next::format::Pixel;
use ffmpeg_next::software::scaling;
use ffmpeg_next::util::frame;
use windows::Win32::Media::DirectShow::{
    IAMStreamConfig, IBaseFilter, ICaptureGraphBuilder2, ICreateDevEnum, IGraphBuilder,
    IMediaControl, IMediaFilter,
};
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, CLSID_CaptureGraphBuilder2, CLSID_FilterGraph, CLSID_SystemDeviceEnum,
    CLSID_VideoInputDeviceCategory, FORMAT_VideoInfo, MEDIASUBTYPE_ARGB32, MEDIASUBTYPE_RGB24,
    MEDIASUBTYPE_RGB32, MEDIATYPE_Video, MFVideoFormat_I420, MFVideoFormat_IYUV,
    MFVideoFormat_NV12, MFVideoFormat_YUY2, MFVideoFormat_YV12, PIN_CATEGORY_CAPTURE,
    VIDEOINFOHEADER,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, CreateBindCtx, IEnumMoniker, IMoniker,
    MkParseDisplayName,
};
use windows::Win32::System::Variant::{VARIANT, VariantClear};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, w};
use windows_core::{implement, interface};

/// The id prefix of a DirectShow camera; its display name follows.
pub const PREFIX: &str = "dshow:";

// qedit.h's Sample Grabber: deprecated and missing from the Windows metadata, still shipped
// with every Windows.
const CLSID_SAMPLE_GRABBER: GUID = GUID::from_u128(0xc1f400a0_3f08_11d3_9f0b_006008039e37);
const CLSID_NULL_RENDERER: GUID = GUID::from_u128(0xc1f400a4_3f08_11d3_9f0b_006008039e37);

// Under the names Windows gives their methods (hence the module's `non_snake_case`).
#[interface("6B652FFF-11FE-4fce-92AD-0266B5D7C78F")]
unsafe trait ISampleGrabber: IUnknown {
    fn SetOneShot(&self, one_shot: windows::core::BOOL) -> HRESULT;
    fn SetMediaType(&self, kind: *const AM_MEDIA_TYPE) -> HRESULT;
    fn GetConnectedMediaType(&self, kind: *mut AM_MEDIA_TYPE) -> HRESULT;
    fn SetBufferSamples(&self, buffer: windows::core::BOOL) -> HRESULT;
    fn GetCurrentBuffer(&self, size: *mut i32, buffer: *mut i32) -> HRESULT;
    fn GetCurrentSample(&self, sample: *mut *mut c_void) -> HRESULT;
    fn SetCallback(&self, callback: *mut c_void, which: i32) -> HRESULT;
}

#[interface("0579154A-2B53-4994-B0D0-E773148EFF85")]
unsafe trait ISampleGrabberCB: IUnknown {
    fn SampleCB(&self, time: f64, sample: *mut c_void) -> HRESULT;
    fn BufferCB(&self, time: f64, buffer: *mut u8, length: i32) -> HRESULT;
}

/// A DirectShow video device.
#[derive(Clone, Debug)]
pub struct Device {
    /// The moniker's display name, which finds it again.
    pub display_name: String,
    pub name: String,
    /// For a hardware device, its path; software cameras have none.
    pub device_path: Option<String>,
}

/// How a camera's pictures are laid out in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Nv12,
    I420,
    /// I420 with the colour planes the other way round.
    Yv12,
    Yuy2,
    /// Blue, green, red; rows padded to 4 bytes.
    Rgb24,
    /// Blue, green, red and an unused byte.
    Rgb32,
    /// Blue, green, red and alpha.
    Argb32,
}

impl Layout {
    pub fn of(subtype: &GUID) -> Option<Self> {
        Some(match *subtype {
            s if s == MFVideoFormat_NV12 => Self::Nv12,
            s if s == MFVideoFormat_I420 || s == MFVideoFormat_IYUV => Self::I420,
            s if s == MFVideoFormat_YV12 => Self::Yv12,
            s if s == MFVideoFormat_YUY2 => Self::Yuy2,
            s if s == MEDIASUBTYPE_RGB24 => Self::Rgb24,
            s if s == MEDIASUBTYPE_RGB32 => Self::Rgb32,
            s if s == MEDIASUBTYPE_ARGB32 => Self::Argb32,
            _ => return None,
        })
    }

    pub fn has_alpha(self) -> bool {
        self == Self::Argb32
    }

    fn rgb(self) -> bool {
        matches!(self, Self::Rgb24 | Self::Rgb32 | Self::Argb32)
    }
}

/// The pictures a capture delivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub layout: Layout,
    pub width: u32,
    pub height: u32,
    /// Rows from the bottom up, as Windows bitmaps with a positive height are.
    pub bottom_up: bool,
}

impl Format {
    /// Bytes a picture of this format takes.
    pub fn size(&self) -> usize {
        let (w, h) = (self.width as usize, self.height as usize);
        match self.layout {
            Layout::Nv12 | Layout::I420 | Layout::Yv12 => {
                w * h + 2 * (w.div_ceil(2) * h.div_ceil(2))
            }
            Layout::Yuy2 => w * h * 2,
            Layout::Rgb24 => (w * 3).next_multiple_of(4) * h,
            Layout::Rgb32 | Layout::Argb32 => w * h * 4,
        }
    }
}

/// One way the device films.
#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub layout: Layout,
    index: i32,
}

/// The one instance path a device is listed under by both APIs: the part before its
/// interface's class (`#{...}`), lower case.
pub fn instance(path: &str) -> String {
    let lower = path.to_lowercase();
    match lower.find("#{") {
        Some(at) => lower[..at].to_string(),
        None => lower,
    }
}

/// The video devices DirectShow lists.
///
/// # Safety
/// COM must be started on the calling thread.
pub unsafe fn devices() -> Result<Vec<Device>> {
    unsafe {
        let enumerator: ICreateDevEnum =
            CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER)?;
        let mut monikers: Option<IEnumMoniker> = None;
        enumerator.CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut monikers, 0)?;
        // None when the category is empty.
        let Some(monikers) = monikers else {
            return Ok(Vec::new());
        };
        let mut devices = Vec::new();
        loop {
            let mut one: [Option<IMoniker>; 1] = [None];
            let mut fetched = 0u32;
            if monikers.Next(&mut one, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(moniker) = one[0].take() else {
                break;
            };
            let context = CreateBindCtx(0)?;
            let display = moniker.GetDisplayName(&context, None)?;
            let display_name = display.to_string().unwrap_or_default();
            CoTaskMemFree(Some(display.0 as *const c_void));
            let bag: Result<IPropertyBag> = moniker
                .BindToStorage(&context, None)
                .map_err(anyhow::Error::from);
            let (name, device_path) = match bag {
                Ok(bag) => (
                    text(&bag, w!("FriendlyName")).unwrap_or_else(|| display_name.clone()),
                    text(&bag, w!("DevicePath")),
                ),
                Err(_) => (display_name.clone(), None),
            };
            devices.push(Device {
                display_name,
                name,
                device_path,
            });
        }
        Ok(devices)
    }
}

unsafe fn text(bag: &IPropertyBag, name: PCWSTR) -> Option<String> {
    unsafe {
        let mut value = VARIANT::default();
        bag.Read(name, &mut value, None).ok()?;
        let read = value.Anonymous.Anonymous.Anonymous.bstrVal.to_string();
        let _ = VariantClear(&mut value);
        Some(read).filter(|s| !s.is_empty())
    }
}

/// The device as a filter, not yet in a graph.
unsafe fn source(display_name: &str) -> Result<IBaseFilter> {
    unsafe {
        let context = CreateBindCtx(0)?;
        let wide: Vec<u16> = display_name.encode_utf16().chain(Some(0)).collect();
        let mut eaten = 0u32;
        let mut moniker: Option<IMoniker> = None;
        MkParseDisplayName(&context, PCWSTR(wide.as_ptr()), &mut eaten, &mut moniker)
            .context("the camera is not there")?;
        let moniker = moniker.context("the camera is not there")?;
        moniker
            .BindToObject(&context, None)
            .context("the camera cannot be opened")
    }
}

/// Frees a media type DirectShow handed out.
unsafe fn free_type(kind: *mut AM_MEDIA_TYPE, and_itself: bool) {
    if kind.is_null() {
        return;
    }
    unsafe {
        let k = &mut *kind;
        if !k.pbFormat.is_null() {
            CoTaskMemFree(Some(k.pbFormat as *const c_void));
            k.pbFormat = std::ptr::null_mut();
        }
        std::mem::ManuallyDrop::drop(&mut k.pUnk);
        if and_itself {
            CoTaskMemFree(Some(kind as *const c_void));
        }
    }
}

/// The format a media type describes, when it is one Umbra reads.
unsafe fn format_of(kind: &AM_MEDIA_TYPE) -> Option<(Format, f64)> {
    if kind.formattype != FORMAT_VideoInfo
        || kind.pbFormat.is_null()
        || (kind.cbFormat as usize) < size_of::<VIDEOINFOHEADER>()
    {
        return None;
    }
    let info = unsafe { &*(kind.pbFormat as *const VIDEOINFOHEADER) };
    let layout = Layout::of(&kind.subtype)?;
    let header = info.bmiHeader;
    let width = u32::try_from(header.biWidth).ok().filter(|w| *w > 0)?;
    let height = header.biHeight.unsigned_abs();
    let fps = if info.AvgTimePerFrame > 0 {
        1e7 / info.AvgTimePerFrame as f64
    } else {
        30.0
    };
    Some((
        Format {
            layout,
            width,
            height,
            // Packed RGB with a positive height runs bottom up; YUV is always top down.
            bottom_up: layout.rgb() && header.biHeight > 0,
        },
        fps,
    ))
}

/// The ways the device films (none when it does not say, as some software cameras don't).
unsafe fn modes_of(builder: &ICaptureGraphBuilder2, source: &IBaseFilter) -> Vec<Mode> {
    let mut found = Vec::new();
    unsafe {
        let mut raw = std::ptr::null_mut();
        if builder
            .FindInterface(
                Some(&PIN_CATEGORY_CAPTURE),
                Some(&MEDIATYPE_Video),
                source,
                &IAMStreamConfig::IID,
                &mut raw,
            )
            .is_err()
            || raw.is_null()
        {
            return found;
        }
        let config = IAMStreamConfig::from_raw(raw);
        let (mut count, mut size) = (0i32, 0i32);
        if config
            .GetNumberOfCapabilities(&mut count, &mut size)
            .is_err()
        {
            return found;
        }
        let mut caps = vec![0u8; size.max(0) as usize + 256];
        for index in 0..count {
            let mut kind: *mut AM_MEDIA_TYPE = std::ptr::null_mut();
            if config
                .GetStreamCaps(index, &mut kind, caps.as_mut_ptr())
                .is_err()
            {
                continue;
            }
            if let Some((format, fps)) = format_of(&*kind) {
                found.push(Mode {
                    width: format.width,
                    height: format.height,
                    fps,
                    layout: format.layout,
                    index,
                });
            }
            free_type(kind, true);
        }
    }
    found
}

/// The ways `display_name` films, for the settings' list of sizes.
///
/// # Safety
/// COM must be started on the calling thread.
pub unsafe fn modes(display_name: &str) -> Result<Vec<Mode>> {
    unsafe {
        let source = source(display_name)?;
        let graph: IGraphBuilder =
            CoCreateInstance(&CLSID_FilterGraph, None, CLSCTX_INPROC_SERVER)?;
        let builder: ICaptureGraphBuilder2 =
            CoCreateInstance(&CLSID_CaptureGraphBuilder2, None, CLSCTX_INPROC_SERVER)?;
        builder.SetFiltergraph(&graph)?;
        graph.AddFilter(&source, w!("Camera"))?;
        Ok(modes_of(&builder, &source))
    }
}

/// The way to film at `width`×`height`: that size if it has it, else the closest; the
/// fastest up to `max_fps`, and no faster than needed.
pub fn pick(modes: &[Mode], width: u32, height: u32, max_fps: u32) -> Option<Mode> {
    let exact: Vec<&Mode> = modes
        .iter()
        .filter(|m| (m.width, m.height) == (width, height))
        .collect();
    let candidates: Vec<&Mode> = if exact.is_empty() {
        let closest = modes
            .iter()
            .min_by_key(|m| (m.height as i64 - height as i64).abs())?;
        modes
            .iter()
            .filter(|m| (m.width, m.height) == (closest.width, closest.height))
            .collect()
    } else {
        exact
    };
    let cap = f64::from(max_fps);
    candidates
        .into_iter()
        .max_by(|a, b| {
            a.fps
                .min(cap)
                .total_cmp(&b.fps.min(cap))
                .then(b.fps.total_cmp(&a.fps))
        })
        .copied()
}

/// The newest picture, handed over from DirectShow's thread.
#[derive(Default)]
struct Slot {
    picture: Mutex<(u64, Vec<u8>)>,
    fresh: Condvar,
}

#[implement(ISampleGrabberCB)]
struct Grabbed {
    slot: Arc<Slot>,
}

impl ISampleGrabberCB_Impl for Grabbed_Impl {
    unsafe fn SampleCB(&self, _time: f64, _sample: *mut c_void) -> HRESULT {
        HRESULT(0)
    }

    unsafe fn BufferCB(&self, _time: f64, buffer: *mut u8, length: i32) -> HRESULT {
        if buffer.is_null() || length <= 0 {
            return HRESULT(0);
        }
        let bytes = unsafe { std::slice::from_raw_parts(buffer, length as usize) };
        let mut picture = self.slot.picture.lock().unwrap();
        picture.0 += 1;
        picture.1.clear();
        picture.1.extend_from_slice(bytes);
        drop(picture);
        self.slot.fresh.notify_one();
        HRESULT(0)
    }
}

/// A DirectShow camera filming; stops when dropped.
pub struct Capture {
    control: IMediaControl,
    _graph: IGraphBuilder,
    _callback: ISampleGrabberCB,
    slot: Arc<Slot>,
    seen: u64,
    pub format: Format,
    pub fps: f64,
}

impl Capture {
    /// Starts filming `display_name` at about `width`×`height`, at most `max_fps`.
    ///
    /// # Safety
    /// COM must be started on the calling thread, and stay started while it films.
    pub unsafe fn open(display_name: &str, width: u32, height: u32, max_fps: u32) -> Result<Self> {
        unsafe {
            let source = source(display_name)?;
            let graph: IGraphBuilder =
                CoCreateInstance(&CLSID_FilterGraph, None, CLSCTX_INPROC_SERVER)?;
            let builder: ICaptureGraphBuilder2 =
                CoCreateInstance(&CLSID_CaptureGraphBuilder2, None, CLSCTX_INPROC_SERVER)?;
            builder.SetFiltergraph(&graph)?;
            graph.AddFilter(&source, w!("Camera"))?;

            // The size and rate asked for, when the device says what it has.
            let chosen = pick(&modes_of(&builder, &source), width, height, max_fps);
            if let Some(mode) = chosen {
                let mut raw = std::ptr::null_mut();
                builder.FindInterface(
                    Some(&PIN_CATEGORY_CAPTURE),
                    Some(&MEDIATYPE_Video),
                    &source,
                    &IAMStreamConfig::IID,
                    &mut raw,
                )?;
                let config = IAMStreamConfig::from_raw(raw);
                let mut caps = vec![0u8; 512];
                let mut kind: *mut AM_MEDIA_TYPE = std::ptr::null_mut();
                config.GetStreamCaps(mode.index, &mut kind, caps.as_mut_ptr())?;
                let set = config.SetFormat(kind);
                free_type(kind, true);
                if let Err(err) = set {
                    log::warn!("camera: format {mode:?} refused ({err}); taking its own");
                }
            }

            let grabber_filter: IBaseFilter =
                CoCreateInstance(&CLSID_SAMPLE_GRABBER, None, CLSCTX_INPROC_SERVER)
                    .context("DirectShow's Sample Grabber is missing")?;
            let grabber: ISampleGrabber = grabber_filter.cast()?;
            // Only video; the subtype the device films in, so nothing converts in between.
            let wanted = AM_MEDIA_TYPE {
                majortype: MEDIATYPE_Video,
                subtype: chosen.map_or(GUID::zeroed(), |m| match m.layout {
                    Layout::Nv12 => MFVideoFormat_NV12,
                    Layout::I420 => MFVideoFormat_I420,
                    Layout::Yv12 => MFVideoFormat_YV12,
                    Layout::Yuy2 => MFVideoFormat_YUY2,
                    Layout::Rgb24 => MEDIASUBTYPE_RGB24,
                    Layout::Rgb32 => MEDIASUBTYPE_RGB32,
                    Layout::Argb32 => MEDIASUBTYPE_ARGB32,
                }),
                ..Default::default()
            };
            grabber.SetMediaType(&wanted).ok()?;
            grabber.SetBufferSamples(false.into()).ok()?;
            grabber.SetOneShot(false.into()).ok()?;
            let slot = Arc::new(Slot::default());
            let callback: ISampleGrabberCB = Grabbed { slot: slot.clone() }.into();
            // 1: called with the picture's bytes.
            grabber.SetCallback(callback.as_raw(), 1).ok()?;
            graph.AddFilter(&grabber_filter, w!("Grabber"))?;
            let null: IBaseFilter =
                CoCreateInstance(&CLSID_NULL_RENDERER, None, CLSCTX_INPROC_SERVER)?;
            graph.AddFilter(&null, w!("Null"))?;
            builder
                .RenderStream(
                    Some(&PIN_CATEGORY_CAPTURE),
                    &MEDIATYPE_Video,
                    &source,
                    &grabber_filter,
                    &null,
                )
                .map_err(|err| {
                    log::info!("camera: DirectShow cannot connect it: {err}");
                    // A software camera has no picture until its program sends one.
                    anyhow!("the camera has no picture to send (for OBS's virtual camera, start it in OBS)")
                })?;

            let mut connected = AM_MEDIA_TYPE::default();
            grabber.GetConnectedMediaType(&mut connected).ok()?;
            let read = format_of(&connected);
            free_type(&mut connected, false);
            let (format, fps) =
                read.ok_or_else(|| anyhow!("the camera films in a format Umbra cannot read"))?;
            // Pictures as they come, not held back to a clock.
            if let Ok(filter) = graph.cast::<IMediaFilter>() {
                let _ = filter.SetSyncSource(None);
            }
            let control: IMediaControl = graph.cast()?;
            control.Run().context("the camera does not start")?;
            log::info!(
                "camera (DirectShow) filming {}x{} at {fps:.0} fps, {:?}",
                format.width,
                format.height,
                format.layout
            );
            Ok(Self {
                control,
                _graph: graph,
                _callback: callback,
                slot,
                seen: 0,
                format,
                fps,
            })
        }
    }

    /// The next picture, waiting up to `most`; `None` when none came.
    pub fn next(&mut self, most: Duration) -> Option<Vec<u8>> {
        let deadline = Instant::now() + most;
        let mut picture = self.slot.picture.lock().unwrap();
        while picture.0 == self.seen {
            let left = deadline.checked_duration_since(Instant::now())?;
            picture = self.slot.fresh.wait_timeout(picture, left).unwrap().0;
        }
        self.seen = picture.0;
        Some(picture.1.clone())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.control.Stop();
        }
    }
}

/// Turns pictures of one format into BGRA of one size, top down. Keeps its converter.
#[derive(Default)]
pub struct Converter {
    context: Option<(scaling::Context, Format, (u32, u32))>,
    source: Option<frame::Video>,
    out: Option<frame::Video>,
}

impl Converter {
    /// `bytes` as BGRA `size` big: its rows, and the bytes from one row to the next.
    pub fn convert(
        &mut self,
        bytes: &[u8],
        format: Format,
        size: (u32, u32),
    ) -> Result<(&[u8], usize)> {
        if bytes.len() < format.size() {
            bail!(
                "the camera's picture is {} bytes, {} expected",
                bytes.len(),
                format.size()
            );
        }
        let pixel = match format.layout {
            Layout::Nv12 => Pixel::NV12,
            Layout::I420 | Layout::Yv12 => Pixel::YUV420P,
            Layout::Yuy2 => Pixel::YUYV422,
            Layout::Rgb24 => Pixel::BGR24,
            Layout::Rgb32 => Pixel::BGRZ,
            Layout::Argb32 => Pixel::BGRA,
        };
        if self
            .context
            .as_ref()
            .is_none_or(|(_, f, s)| *f != format || *s != size)
        {
            let mut context = scaling::Context::get(
                pixel,
                format.width,
                format.height,
                Pixel::BGRA,
                size.0,
                size.1,
                scaling::Flags::BILINEAR,
            )
            .context("cannot convert the camera's pictures")?;
            // Video from HD on is BT.709; smaller, BT.601.
            if !format.layout.rgb() {
                unsafe {
                    let space = if format.height >= 720 {
                        SWS_CS_ITU709
                    } else {
                        SWS_CS_DEFAULT
                    };
                    let table = sws_getCoefficients(space);
                    sws_setColorspaceDetails(
                        context.as_mut_ptr(),
                        table,
                        0,
                        table,
                        1,
                        0,
                        1 << 16,
                        1 << 16,
                    );
                }
            }
            self.context = Some((context, format, size));
            self.source = Some(frame::Video::new(pixel, format.width, format.height));
            self.out = Some(frame::Video::empty());
        }
        let source = self.source.as_mut().unwrap();
        fill(source, bytes, format);
        let out = self.out.as_mut().unwrap();
        let (context, ..) = self.context.as_mut().unwrap();
        context
            .run(source, out)
            .context("cannot convert a camera picture")?;
        Ok((out.data(0), out.stride(0)))
    }
}

/// Copies a picture's planes into `frame`, top row first.
fn fill(frame: &mut frame::Video, bytes: &[u8], format: Format) {
    let (w, h) = (format.width as usize, format.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut copy_plane = |plane: usize, from: usize, row: usize, rows: usize, flip: bool| {
        let stride = frame.stride(plane);
        let data = frame.data_mut(plane);
        for y in 0..rows {
            let src_row = if flip { rows - 1 - y } else { y };
            let src = &bytes[from + src_row * row..][..row];
            data[y * stride..y * stride + row].copy_from_slice(src);
        }
    };
    match format.layout {
        Layout::Nv12 => {
            copy_plane(0, 0, w, h, false);
            copy_plane(1, w * h, cw * 2, ch, false);
        }
        Layout::I420 | Layout::Yv12 => {
            let (first, second) = if format.layout == Layout::I420 {
                (1, 2)
            } else {
                (2, 1)
            };
            copy_plane(0, 0, w, h, false);
            copy_plane(first, w * h, cw, ch, false);
            copy_plane(second, w * h + cw * ch, cw, ch, false);
        }
        Layout::Yuy2 => copy_plane(0, 0, w * 2, h, false),
        Layout::Rgb24 => {
            // Rows are padded to four bytes; only the pixels are copied.
            let padded = (w * 3).next_multiple_of(4);
            let stride = frame.stride(0);
            let data = frame.data_mut(0);
            for y in 0..h {
                let src_row = if format.bottom_up { h - 1 - y } else { y };
                data[y * stride..y * stride + w * 3]
                    .copy_from_slice(&bytes[src_row * padded..][..w * 3]);
            }
        }
        Layout::Rgb32 | Layout::Argb32 => copy_plane(0, 0, w * 4, h, format.bottom_up),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_both_list_is_known_by_its_instance() {
        let mf = r"\\?\usb#vid_05a3&pid_9331&mi_00#9&7c239da&2&0000#{e5323777-f976-4f5b-9b55-b94699c46e44}\global";
        let ds = r"\\?\USB#VID_05A3&PID_9331&MI_00#9&7C239DA&2&0000#{65e8773d-8f56-11d0-a3b9-00a0c9223196}\global";
        assert_eq!(instance(mf), instance(ds));
        assert_ne!(
            instance(mf),
            instance(r"\\?\usb#vid_0000&pid_0001#x#{e5323777}\global")
        );
    }

    /// A 4×2 picture: the top row red, the bottom row blue.
    fn red_over_blue(layout: Layout, bottom_up: bool) -> (Vec<u8>, Format) {
        let format = Format {
            layout,
            width: 4,
            height: 2,
            bottom_up,
        };
        let (bpp, pad) = match layout {
            Layout::Rgb24 => (3, 0),
            _ => (4, 0),
        };
        let row = |b: u8, g: u8, r: u8| -> Vec<u8> {
            let mut out = Vec::new();
            for _ in 0..4 {
                out.extend_from_slice(&[b, g, r]);
                if bpp == 4 {
                    out.push(255);
                }
            }
            out.extend(std::iter::repeat_n(0, pad));
            out
        };
        let (top, bottom) = (row(0, 0, 255), row(255, 0, 0));
        let bytes = if bottom_up {
            [bottom, top].concat()
        } else {
            [top, bottom].concat()
        };
        (bytes, format)
    }

    #[test]
    fn bottom_up_pictures_come_out_the_right_way_up() {
        crate::library::init_ffmpeg().unwrap();
        for layout in [Layout::Rgb24, Layout::Rgb32, Layout::Argb32] {
            for bottom_up in [true, false] {
                let (bytes, format) = red_over_blue(layout, bottom_up);
                let mut converter = Converter::default();
                let (data, stride) = converter.convert(&bytes, format, (4, 2)).unwrap();
                let top = &data[..4];
                let bottom = &data[stride..stride + 4];
                // BGRA out: red on top.
                assert!(
                    top[2] > 200 && top[0] < 50,
                    "{layout:?} {bottom_up}: {top:?}"
                );
                assert!(
                    bottom[0] > 200 && bottom[2] < 50,
                    "{layout:?} {bottom_up}: {bottom:?}"
                );
            }
        }
    }

    #[test]
    fn yuv_pictures_convert_and_are_sized() {
        crate::library::init_ffmpeg().unwrap();
        // Mid grey: luma 128, no colour.
        for layout in [Layout::Nv12, Layout::I420, Layout::Yv12, Layout::Yuy2] {
            let format = Format {
                layout,
                width: 4,
                height: 2,
                bottom_up: false,
            };
            let bytes = match layout {
                Layout::Yuy2 => [128u8, 128].repeat(4 * 2),
                _ => {
                    let mut b = vec![128u8; 4 * 2];
                    // Two colour samples, for 2×1 blocks of four pixels each.
                    b.extend([128u8; 4]);
                    b
                }
            };
            assert_eq!(bytes.len(), format.size(), "{layout:?}");
            let mut converter = Converter::default();
            let (data, _) = converter.convert(&bytes, format, (8, 4)).unwrap();
            let px = &data[..4];
            assert!(
                px[0].abs_diff(px[2]) < 8 && (100..160).contains(&px[1]),
                "{layout:?}: {px:?}"
            );
        }
        let short = Format {
            layout: Layout::Nv12,
            width: 4,
            height: 2,
            bottom_up: false,
        };
        assert!(
            Converter::default()
                .convert(&[0; 3], short, (4, 2))
                .is_err()
        );
    }

    #[test]
    fn the_size_asked_for_and_the_fastest_rate_up_to_the_cap() {
        let mode = |width, height, fps, index| Mode {
            width,
            height,
            fps,
            layout: Layout::Nv12,
            index,
        };
        let modes = [
            mode(1280, 720, 30.0, 0),
            mode(1280, 720, 60.0, 1),
            mode(1920, 1080, 30.0, 2),
        ];
        assert_eq!(pick(&modes, 1280, 720, 60).unwrap().index, 1);
        assert_eq!(pick(&modes, 1280, 720, 30).unwrap().index, 0);
        assert_eq!(pick(&modes, 1920, 1000, 30).unwrap().index, 2);
        assert!(pick(&[], 1280, 720, 30).is_none());
    }

    /// Films a DirectShow-only camera on this PC for real; needs one (AMD's Privacy View
    /// camera is one). `UMBRA_DSHOW_CAMERA` names it.
    #[test]
    #[ignore]
    fn a_software_camera_films() {
        let name = std::env::var("UMBRA_DSHOW_CAMERA").unwrap_or("AMD Privacy View camera".into());
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
            let device = devices()
                .unwrap()
                .into_iter()
                .find(|d| d.name == name)
                .expect("the camera is listed");
            assert!(
                device.device_path.is_none(),
                "a software camera has no device path"
            );
            assert!(!modes(&device.display_name).unwrap().is_empty());
            let mut capture = Capture::open(&device.display_name, 1280, 720, 30).unwrap();
            let picture = capture
                .next(Duration::from_secs(5))
                .expect("a picture comes");
            assert!(picture.len() >= capture.format.size());
            crate::library::init_ffmpeg().unwrap();
            let mut converter = Converter::default();
            let (data, stride) = converter
                .convert(&picture, capture.format, (640, 360))
                .unwrap();
            assert!(stride >= 640 * 4 && data.len() >= stride * 360);
        }
    }
}
