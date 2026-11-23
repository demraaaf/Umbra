//! Webcams, through Media Foundation: the cameras there are with the sizes they film at, and
//! a worker that films one and encodes it as a video stream of its own. The camera is drawn
//! over the screen only by the player and by export, so it can still be moved in a clip.
//! Cameras only DirectShow lists (OBS's virtual camera and other software cameras) are filmed
//! through `dshow` and from then on handled the same.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::Packet;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::core::{GUID, Interface, PWSTR};

use crate::clock::Clock;
use crate::d3d::Gpu;
use crate::encode::{
    EncoderTuning, RateControl, StreamInfo, VideoCodec, VideoEncoder, VideoSettings,
};

/// Track name of the camera stream in a clip.
pub const CAMERA_TITLE: &str = "Camera";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraDevice {
    /// Windows' name for the device (its symbolic link); stays the same across restarts.
    pub id: String,
    pub name: String,
    /// The sizes it films at, smallest first.
    pub sizes: Vec<(u32, u32)>,
    /// `Media Foundation` or `DirectShow`.
    pub api: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraInput {
    Device(String),
    /// Colour bars instead of a camera, for testing without one.
    Pattern,
}

#[derive(Clone, Debug)]
pub struct CameraConfig {
    pub input: CameraInput,
    /// Size of the camera stream; a camera filming at another size is scaled to it.
    pub width: u32,
    pub height: u32,
    /// Most frames per second to keep; a camera filming faster is thinned out.
    pub max_fps: u32,
    pub codec: VideoCodec,
    pub preset: u8,
    /// Off closes the camera, so its light goes out, and records nothing until it is on again.
    pub on: Arc<AtomicBool>,
}

/// One way a camera films: a size, a frame rate and a pixel format.
#[derive(Clone, Copy, Debug)]
struct Mode {
    width: u32,
    height: u32,
    fps: f64,
    subtype: GUID,
    index: u32,
}

/// Media Foundation for as long as this lives (it counts starts and shutdowns).
struct Mf;

impl Mf {
    fn start() -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).context("cannot start Media Foundation")?;
        }
        Ok(Self)
    }
}

impl Drop for Mf {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// The cameras connected now. Opening them to read their sizes does not turn their light on.
pub fn list_cameras() -> Vec<CameraDevice> {
    let result = (|| -> Result<Vec<CameraDevice>> {
        let _mf = Mf::start()?;
        let mut cameras = Vec::new();
        for activate in unsafe { activates()? } {
            let (Some(id), Some(name)) = (
                unsafe {
                    string(
                        &activate,
                        &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                    )
                },
                unsafe { string(&activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME) },
            ) else {
                continue;
            };
            let mut sizes: Vec<(u32, u32)> = match unsafe { device_modes(&id) } {
                Ok(modes) => modes.iter().map(|m| (m.width, m.height)).collect(),
                Err(err) => {
                    log::warn!("cannot read the sizes of camera {name}: {err:#}");
                    Vec::new()
                }
            };
            sizes.sort_by_key(|&(w, h)| (h, w));
            sizes.dedup();
            cameras.push(CameraDevice {
                id,
                name,
                sizes,
                api: "Media Foundation",
            });
        }
        // Then what only DirectShow has; a webcam it lists too is Media Foundation's.
        let known: Vec<String> = cameras
            .iter()
            .map(|c| crate::dshow::instance(&c.id))
            .collect();
        let devices = unsafe { crate::dshow::devices() }.unwrap_or_else(|err| {
            log::warn!("cannot list DirectShow cameras: {err:#}");
            Vec::new()
        });
        for device in devices {
            if device
                .device_path
                .as_deref()
                .is_some_and(|path| known.contains(&crate::dshow::instance(path)))
            {
                continue;
            }
            // A camera that films nothing now (OBS closed) is still listed, without sizes.
            let mut sizes: Vec<(u32, u32)> = unsafe { crate::dshow::modes(&device.display_name) }
                .map(|modes| modes.iter().map(|m| (m.width, m.height)).collect())
                .unwrap_or_default();
            sizes.sort_by_key(|&(w, h)| (h, w));
            sizes.dedup();
            cameras.push(CameraDevice {
                id: format!("{}{}", crate::dshow::PREFIX, device.display_name),
                name: device.name,
                sizes,
                api: "DirectShow",
            });
        }
        Ok(cameras)
    })();
    result.unwrap_or_else(|err| {
        log::warn!("cannot list cameras: {err:#}");
        Vec::new()
    })
}

unsafe fn activates() -> Result<Vec<IMFActivate>> {
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 1)?;
        let attributes = attributes.context("no attributes")?;
        attributes.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attributes, &mut list, &mut count)?;
        if list.is_null() {
            return Ok(Vec::new());
        }
        // Taking each one out hands its reference over; the array itself is freed here.
        let found = std::slice::from_raw_parts_mut(list, count as usize)
            .iter_mut()
            .filter_map(Option::take)
            .collect();
        CoTaskMemFree(Some(list as *const c_void));
        Ok(found)
    }
}

unsafe fn string(attributes: &IMFAttributes, key: &GUID) -> Option<String> {
    unsafe {
        let mut value = PWSTR::null();
        let mut length = 0u32;
        attributes
            .GetAllocatedString(key, &mut value, &mut length)
            .ok()?;
        let text = value.to_string().ok();
        CoTaskMemFree(Some(value.0 as *const c_void));
        text
    }
}

/// The camera with this id, opened but not filming.
unsafe fn open_source(id: &str) -> Result<IMFMediaSource> {
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 2)?;
        let attributes = attributes.context("no attributes")?;
        attributes.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )?;
        let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        attributes.SetString(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
            windows::core::PCWSTR(wide.as_ptr()),
        )?;
        MFCreateDeviceSource(&attributes).context("the camera is not connected")
    }
}

const FIRST_VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

unsafe fn native_modes(reader: &IMFSourceReader) -> Vec<Mode> {
    let mut modes = Vec::new();
    for index in 0.. {
        let Ok(kind) = (unsafe { reader.GetNativeMediaType(FIRST_VIDEO, index) }) else {
            break;
        };
        let read = || -> Result<Mode> {
            unsafe {
                let size = kind.GetUINT64(&MF_MT_FRAME_SIZE)?;
                let rate = kind.GetUINT64(&MF_MT_FRAME_RATE)?;
                let (num, den) = ((rate >> 32) as f64, (rate & 0xffff_ffff) as f64);
                Ok(Mode {
                    width: (size >> 32) as u32,
                    height: (size & 0xffff_ffff) as u32,
                    fps: if den > 0.0 { num / den } else { 0.0 },
                    subtype: kind.GetGUID(&MF_MT_SUBTYPE)?,
                    index,
                })
            }
        };
        if let Ok(mode) = read() {
            modes.push(mode);
        }
    }
    modes
}

unsafe fn device_modes(id: &str) -> Result<Vec<Mode>> {
    unsafe {
        let source = open_source(id)?;
        let modes = MFCreateSourceReaderFromMediaSource(&source, None::<&IMFAttributes>)
            .map(|reader| native_modes(&reader));
        let _ = source.Shutdown();
        Ok(modes?)
    }
}

/// The way to film at `width`×`height`: the fastest up to `max_fps`, and no faster than needed.
/// Among equally fast ones MJPEG comes first: it is what the camera itself sends at sizes past
/// what USB carries uncompressed. Windows also offers NV12 and H.264 at those sizes, made from
/// the camera's own stream, and they can come far slower than they claim (an HD Web Camera's
/// "30 fps" NV12 720p gave 5 fps, after a 3 s stall). A camera that cannot film at that size
/// films at the closest one and is scaled.
fn pick_mode(modes: &[Mode], width: u32, height: u32, max_fps: u32) -> Option<Mode> {
    let exact: Vec<&Mode> = modes
        .iter()
        .filter(|m| m.width == width && m.height == height)
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
    let cap = max_fps as f64;
    candidates
        .into_iter()
        .max_by(|a, b| {
            // MJPEG, then the camera's plain formats, then what Windows makes from them.
            let rank = |m: &Mode| {
                if m.subtype == MFVideoFormat_MJPG {
                    2
                } else if m.subtype == MFVideoFormat_YUY2 {
                    1
                } else {
                    0
                }
            };
            a.fps
                .min(cap)
                .total_cmp(&b.fps.min(cap))
                .then(rank(a).cmp(&rank(b)))
                .then(b.fps.total_cmp(&a.fps))
        })
        .copied()
}

/// A camera filming, its frames coming out as BGRA textures on `gpu`.
struct Filming {
    source: IMFMediaSource,
    reader: IMFSourceReader,
}

impl Drop for Filming {
    fn drop(&mut self) {
        // Without this the camera stays taken (and lit) until the process ends.
        unsafe {
            let _ = self.source.Shutdown();
        }
    }
}

unsafe fn start_filming(
    id: &str,
    manager: &IMFDXGIDeviceManager,
    size: (u32, u32),
    max_fps: u32,
) -> Result<Filming> {
    unsafe {
        let source = open_source(id)?;
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 2)?;
        let attributes = attributes.context("no attributes")?;
        // Frames on the GPU, converted (and scaled) there. MJPEG is decoded by Windows' own
        // decoder on the CPU, not the graphics card's: NVIDIA's MJPEG decoder hands frames
        // over before they are written, so every 24th frame of a 1080p camera showed the one
        // from 12 frames earlier.
        attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let reader = match MFCreateSourceReaderFromMediaSource(&source, &attributes) {
            Ok(reader) => reader,
            Err(err) => {
                let _ = source.Shutdown();
                return Err(err).context("cannot read from the camera");
            }
        };
        let filming = Filming { source, reader };
        let modes = native_modes(&filming.reader);
        let mode = pick_mode(&modes, size.0, size.1, max_fps)
            .ok_or_else(|| anyhow!("the camera reports no video format"))?;
        let native = filming.reader.GetNativeMediaType(FIRST_VIDEO, mode.index)?;
        filming
            .reader
            .SetCurrentMediaType(FIRST_VIDEO, None, &native)
            .context("the camera refused its own format")?;
        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        // ARGB32 is DXGI's BGRA, the encoder's format; RGB32 would be BGRX, which a texture
        // copy into the encoder's frames silently skips.
        output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_ARGB32)?;
        output.SetUINT64(&MF_MT_FRAME_SIZE, ((size.0 as u64) << 32) | size.1 as u64)?;
        filming
            .reader
            .SetCurrentMediaType(FIRST_VIDEO, None, &output)
            .context("the camera's frames cannot be converted")?;
        log::info!(
            "camera filming {}x{} at {:.0} fps ({}), recorded at {}x{}",
            mode.width,
            mode.height,
            mode.fps,
            if mode.subtype == MFVideoFormat_MJPG {
                "MJPEG"
            } else {
                "uncompressed"
            },
            size.0,
            size.1
        );
        Ok(filming)
    }
}

pub struct CameraWorker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Why the camera is not filming while it should, such as it not being connected.
    problem: Arc<Mutex<Option<String>>>,
    pub fps: u32,
}

impl CameraWorker {
    /// Makes the camera's encoder, so its stream is known before recording starts, and films
    /// on a thread of its own while `config.on` is set. `sink` gets each packet (timed in
    /// frames of `fps`) and its time in seconds on `clock`.
    pub fn spawn(
        config: CameraConfig,
        clock: Clock,
        mut sink: Box<dyn FnMut(Packet, f64) + Send>,
    ) -> Result<(Self, StreamInfo)> {
        if config.width % 2 != 0 || config.height % 2 != 0 || config.width == 0 {
            bail!(
                "camera size {}x{} must be even and not zero",
                config.width,
                config.height
            );
        }
        let max_fps = config.max_fps.clamp(1, 60);
        let fps = match &config.input {
            CameraInput::Device(id) if id.starts_with(crate::dshow::PREFIX) => {
                let _mf = Mf::start()?;
                let display = &id[crate::dshow::PREFIX.len()..];
                unsafe { crate::dshow::modes(display) }
                    .ok()
                    .and_then(|modes| {
                        crate::dshow::pick(&modes, config.width, config.height, max_fps)
                    })
                    .map_or(30.min(max_fps), |m| {
                        (m.fps.round() as u32).clamp(1, max_fps)
                    })
            }
            CameraInput::Device(id) => {
                let _mf = Mf::start()?;
                unsafe { device_modes(id) }
                    .ok()
                    .and_then(|modes| pick_mode(&modes, config.width, config.height, max_fps))
                    .map_or(30.min(max_fps), |m| {
                        (m.fps.round() as u32).clamp(1, max_fps)
                    })
            }
            CameraInput::Pattern => 30.min(max_fps),
        };
        // Its own device: Media Foundation works on it from threads of its own.
        let gpu = Gpu::new()?;
        // About 0.1 bit a pixel: a 720p30 camera takes 2.8 Mbit/s.
        let kbps = (config.width as u64 * config.height as u64 * fps as u64 / 10_000) as u32;
        let mut encoder = VideoEncoder::new(
            &gpu,
            VideoSettings {
                codec: config.codec,
                width: config.width,
                height: config.height,
                fps,
                rate: RateControl::Vbr {
                    kbps,
                    max_kbps: kbps * 2,
                },
                preset: config.preset,
                // A clip starts on a screen keyframe and its camera on the next camera one, so
                // camera keyframes come often: the camera is missing at most this long.
                keyframe_interval_secs: 1,
                tuning: EncoderTuning::default(),
                hdr10: false,
            },
        )
        .context("cannot make the camera's encoder")?;
        let mut stream = encoder.stream_info();
        stream.title = Some(CAMERA_TITLE.to_string());

        let stop = Arc::new(AtomicBool::new(false));
        let problem: Arc<Mutex<Option<String>>> = Default::default();
        let (thread_stop, thread_problem) = (stop.clone(), problem.clone());
        let thread = std::thread::Builder::new()
            .name("umbra-camera".into())
            .spawn(move || {
                let mut last_pts = -1i64;
                let mut timed = |packet: Packet| {
                    let time = packet.pts().unwrap_or(0) as f64 / fps as f64;
                    sink(packet, time);
                };
                let mut encode = |texture: &ID3D11Texture2D| {
                    let pts = (clock.now_secs() * fps as f64).round() as i64;
                    // A camera a little faster than `fps` gives two frames the same slot.
                    if pts <= last_pts {
                        return;
                    }
                    last_pts = pts;
                    if let Err(err) = encoder.encode_texture(texture, pts, &mut timed) {
                        log::warn!("camera frame not encoded: {err:#}");
                    }
                };
                let result = match &config.input {
                    CameraInput::Device(id) if id.starts_with(crate::dshow::PREFIX) => film_dshow(
                        &id[crate::dshow::PREFIX.len()..],
                        &gpu,
                        &config,
                        max_fps,
                        &thread_stop,
                        &thread_problem,
                        &mut encode,
                    ),
                    CameraInput::Device(id) => film(
                        id,
                        &gpu,
                        &config,
                        max_fps,
                        &thread_stop,
                        &thread_problem,
                        &mut encode,
                    ),
                    CameraInput::Pattern => pattern(&gpu, &config, fps, &thread_stop, &mut encode),
                };
                if let Err(err) = result {
                    log::error!("camera stopped: {err:#}");
                    *thread_problem.lock().unwrap() = Some(format!("{err:#}"));
                }
            })
            .context("cannot start the camera thread")?;
        Ok((
            Self {
                stop,
                thread: Some(thread),
                problem,
                fps,
            },
            stream,
        ))
    }

    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }
}

impl Drop for CameraWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        // Waiting for a frame can block if the camera stalls; the thread then ends on its own
        // once one comes or the camera fails, rather than holding up the recorder.
        let deadline = Instant::now() + Duration::from_secs(1);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            log::warn!("camera thread left to finish on its own");
        }
    }
}

fn bgra_texture(gpu: &Gpu, width: u32, height: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        ..Default::default()
    };
    let mut texture = None;
    unsafe {
        gpu.device
            .CreateTexture2D(&desc, None, Some(&mut texture))?
    };
    texture.context("no texture returned")
}

/// Films while `config.on` is set, opening the camera again (every 2 s) while it cannot be.
fn film(
    id: &str,
    gpu: &Gpu,
    config: &CameraConfig,
    max_fps: u32,
    stop: &AtomicBool,
    problem: &Mutex<Option<String>>,
    encode: &mut dyn FnMut(&ID3D11Texture2D),
) -> Result<()> {
    let _mf = Mf::start()?;
    let mut token = 0u32;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager)? };
    let manager = manager.context("no device manager")?;
    unsafe { manager.ResetDevice(&gpu.device, token)? };
    let frame = bgra_texture(gpu, config.width, config.height)?;
    let size = (config.width, config.height);
    let set_problem = |text: Option<String>| report(problem, text);

    let mut filming: Option<Filming> = None;
    let mut next_try = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if !config.on.load(Ordering::Relaxed) {
            filming = None;
            set_problem(None);
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let Some(open) = &filming else {
            if Instant::now() < next_try {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            match unsafe { start_filming(id, &manager, size, max_fps) } {
                Ok(started) => {
                    filming = Some(started);
                    set_problem(None);
                }
                Err(err) => {
                    set_problem(Some(format!("{err:#}")));
                    next_try = Instant::now() + Duration::from_secs(2);
                }
            }
            continue;
        };
        let mut flags = 0u32;
        let mut sample = None;
        let read = unsafe {
            open.reader.ReadSample(
                FIRST_VIDEO,
                0,
                None,
                Some(&mut flags),
                None,
                Some(&mut sample),
            )
        };
        let failed = flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_ENDOFSTREAM.0) as u32;
        if let Err(err) = read.as_ref().map_err(|e| e.to_string()).and_then(|_| {
            if failed != 0 {
                Err("the camera stopped (unplugged, or taken by another program)".into())
            } else {
                Ok(())
            }
        }) {
            set_problem(Some(err));
            filming = None;
            next_try = Instant::now() + Duration::from_secs(2);
            continue;
        }
        let Some(sample) = sample else {
            continue;
        };
        if let Err(err) = unsafe { copy_sample(gpu, &sample, &frame, size) } {
            set_problem(Some(format!("{err:#}")));
            filming = None;
            next_try = Instant::now() + Duration::from_secs(2);
            continue;
        }
        encode(&frame);
    }
    Ok(())
}

/// Says why the camera is not filming (logged when it changes), or that it is again.
fn report(problem: &Mutex<Option<String>>, text: Option<String>) {
    let mut slot = problem.lock().unwrap();
    if text.is_some() && *slot != text {
        log::warn!("camera: {}", text.as_deref().unwrap_or_default());
    }
    *slot = text;
}

/// Films a DirectShow camera while `config.on` is set, like `film`: opened again every 2 s
/// while it cannot be, and a camera that stops sending pictures (OBS closed) counts as gone,
/// so the recording has a gap there and carries on once it sends again.
fn film_dshow(
    display_name: &str,
    gpu: &Gpu,
    config: &CameraConfig,
    max_fps: u32,
    stop: &AtomicBool,
    problem: &Mutex<Option<String>>,
    encode: &mut dyn FnMut(&ID3D11Texture2D),
) -> Result<()> {
    // COM, which DirectShow needs, comes with it.
    let _mf = Mf::start()?;
    let frame = bgra_texture(gpu, config.width, config.height)?;
    let size = (config.width, config.height);
    let mut capture: Option<crate::dshow::Capture> = None;
    let mut converter = crate::dshow::Converter::default();
    let mut next_try = Instant::now();
    let mut told_alpha = false;
    while !stop.load(Ordering::Relaxed) {
        if !config.on.load(Ordering::Relaxed) {
            capture = None;
            report(problem, None);
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let Some(open) = capture.as_mut() else {
            if Instant::now() < next_try {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            match unsafe { crate::dshow::Capture::open(display_name, size.0, size.1, max_fps) } {
                Ok(started) => {
                    if started.format.layout.has_alpha() && !told_alpha {
                        // ponytail: alpha is dropped by the 4:2:0 encode; a matte stream
                        // ("Camera alpha") would keep it once a camera that sends it is around.
                        log::info!("camera sends transparency; it is recorded without it");
                        told_alpha = true;
                    }
                    capture = Some(started);
                    report(problem, None);
                }
                Err(err) => {
                    report(problem, Some(format!("{err:#}")));
                    next_try = Instant::now() + Duration::from_secs(2);
                }
            }
            continue;
        };
        let Some(picture) = open.next(Duration::from_secs(2)) else {
            report(
                problem,
                Some("the camera sends no picture (for OBS's, start its virtual camera)".into()),
            );
            capture = None;
            next_try = Instant::now() + Duration::from_secs(2);
            continue;
        };
        match converter.convert(&picture, open.format, size) {
            Ok((data, stride)) => {
                unsafe {
                    gpu.context.UpdateSubresource(
                        &frame,
                        0,
                        None,
                        data.as_ptr() as *const c_void,
                        stride as u32,
                        0,
                    );
                }
                encode(&frame);
            }
            Err(err) => {
                report(problem, Some(format!("{err:#}")));
                capture = None;
                next_try = Instant::now() + Duration::from_secs(2);
            }
        }
    }
    Ok(())
}

/// Copies a camera frame into `frame`: on the GPU when Media Foundation left it there, from
/// memory otherwise (some virtual cameras).
unsafe fn copy_sample(
    gpu: &Gpu,
    sample: &IMFSample,
    frame: &ID3D11Texture2D,
    size: (u32, u32),
) -> Result<()> {
    unsafe {
        let buffer = sample.GetBufferByIndex(0)?;
        if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
            let mut raw: *mut c_void = std::ptr::null_mut();
            dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
            let texture = ID3D11Texture2D::from_raw(raw);
            let index = dxgi.GetSubresourceIndex()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
                || desc.Width < size.0
                || desc.Height < size.1
            {
                bail!(
                    "the camera's frames came as {}x{} format {}, not {}x{} BGRA",
                    desc.Width,
                    desc.Height,
                    desc.Format.0,
                    size.0,
                    size.1
                );
            }
            gpu.context
                .CopySubresourceRegion(frame, 0, 0, 0, 0, &texture, index, None);
            return Ok(());
        }
        let buffer: IMF2DBuffer = buffer
            .cast()
            .context("the camera's frames are in an unknown kind of buffer")?;
        let (mut scanline, mut pitch) = (std::ptr::null_mut::<u8>(), 0i32);
        buffer.Lock2D(&mut scanline, &mut pitch)?;
        // Rows can run bottom-up (a negative pitch); copied top-down here.
        let row = size.0 as usize * 4;
        let mut pixels = vec![0u8; row * size.1 as usize];
        for y in 0..size.1 as usize {
            let from = scanline.offset(y as isize * pitch as isize);
            std::ptr::copy_nonoverlapping(from, pixels[y * row..].as_mut_ptr(), row);
        }
        let _ = buffer.Unlock2D();
        gpu.context.UpdateSubresource(
            frame,
            0,
            None,
            pixels.as_ptr() as *const c_void,
            row as u32,
            0,
        );
        Ok(())
    }
}

/// Colour bars at `fps`, for testing the camera stream without a camera.
fn pattern(
    gpu: &Gpu,
    config: &CameraConfig,
    fps: u32,
    stop: &AtomicBool,
    encode: &mut dyn FnMut(&ID3D11Texture2D),
) -> Result<()> {
    let bars = gpu.test_pattern(config.width, config.height)?;
    let mut ticker = crate::timer::Ticker::new(fps)?;
    while !stop.load(Ordering::Relaxed) {
        ticker.wait_next();
        if config.on.load(Ordering::Relaxed) {
            encode(&bars);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u32, height: u32, fps: f64, subtype: GUID, index: u32) -> Mode {
        Mode {
            width,
            height,
            fps,
            subtype,
            index,
        }
    }

    #[test]
    fn the_fastest_format_of_the_size_is_picked_mjpeg_first() {
        let modes = [
            mode(1920, 1080, 5.0, MFVideoFormat_YUY2, 0),
            mode(1920, 1080, 30.0, MFVideoFormat_MJPG, 1),
            mode(1280, 720, 30.0, MFVideoFormat_NV12, 2),
            mode(1280, 720, 30.0, MFVideoFormat_H264, 3),
            mode(1280, 720, 30.0, MFVideoFormat_MJPG, 4),
            mode(1280, 720, 60.0, MFVideoFormat_MJPG, 5),
            mode(640, 480, 30.0, MFVideoFormat_NV12, 6),
            mode(640, 480, 30.0, MFVideoFormat_YUY2, 7),
        ];
        assert_eq!(pick_mode(&modes, 1920, 1080, 60).unwrap().index, 1);
        assert_eq!(pick_mode(&modes, 1280, 720, 60).unwrap().index, 5);
        // At 30, the 60 fps format gains nothing; MJPEG before what Windows makes of it.
        assert_eq!(pick_mode(&modes, 1280, 720, 30).unwrap().index, 4);
        // The camera's own uncompressed format before Windows' NV12.
        assert_eq!(pick_mode(&modes, 640, 480, 30).unwrap().index, 7);
        // A size the camera does not have: the closest one, scaled.
        assert_eq!(pick_mode(&modes, 1280, 960, 30).unwrap().width, 1920);
        assert!(pick_mode(&[], 1280, 720, 30).is_none());
    }
}
