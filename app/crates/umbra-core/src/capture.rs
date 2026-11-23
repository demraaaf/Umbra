//! Screen capture through Windows Graphics Capture.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows_future::AsyncStatus;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_GDI_COMPATIBLE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device, ID3D11DeviceContext,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIDevice, IDXGIOutput5,
    IDXGIOutputDuplication, IDXGISurface1,
};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, DeleteObject, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW,
    GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HICON,
    ICONINFO,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
use windows::core::{BOOL, IInspectable, Interface, PCWSTR};

use crate::d3d::Gpu;

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    /// Raw HMONITOR value.
    pub handle: isize,
    /// Device name such as `\\.\DISPLAY1`.
    pub device: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub primary: bool,
    /// The monitor's own resolution. It differs from `width` × `height` while the screen
    /// runs at another mode, such as a game's stretched 1440×1080 on a 1920×1080 monitor.
    pub native_width: u32,
    pub native_height: u32,
}

/// Asks Windows once for capture without the yellow border. Without this, Windows accepts
/// `SetIsBorderRequired(false)` but ignores it. Waits at most 2 s, so a consent prompt can
/// never hold up the capture; the border then shows until the next capture after consent.
fn allow_borderless() {
    static ASKED: std::sync::Once = std::sync::Once::new();
    ASKED.call_once(|| {
        let asked = || -> windows::core::Result<String> {
            let op = windows::Graphics::Capture::GraphicsCaptureAccess::RequestAccessAsync(
                windows::Graphics::Capture::GraphicsCaptureAccessKind::Borderless,
            )?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while op.Status()? == AsyncStatus::Started && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(match op.Status()? {
                AsyncStatus::Completed => format!("{:?}", op.GetResults()?),
                status => format!("not answered yet ({status:?})"),
            })
        };
        match asked() {
            Ok(answer) => log::info!("borderless capture: {answer}"),
            // Windows before 10.0.20348 has no such request; the border stays.
            Err(err) => log::info!("borderless capture not available: {err}"),
        }
    });
}

pub fn list_monitors() -> Vec<MonitorInfo> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            let mut mode = DEVMODEW {
                dmSize: size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let has_mode = unsafe {
                EnumDisplaySettingsW(
                    PCWSTR(info.szDevice.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &mut mode,
                )
            }
            .as_bool();
            let rect = info.monitorInfo.rcMonitor;
            let name_len = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            list.push(MonitorInfo {
                handle: monitor.0 as isize,
                device: String::from_utf16_lossy(&info.szDevice[..name_len]),
                x: rect.left,
                y: rect.top,
                width: if has_mode {
                    mode.dmPelsWidth
                } else {
                    (rect.right - rect.left) as u32
                },
                height: if has_mode {
                    mode.dmPelsHeight
                } else {
                    (rect.bottom - rect.top) as u32
                },
                refresh_hz: if has_mode { mode.dmDisplayFrequency } else { 0 },
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                native_width: 0,
                native_height: 0,
            });
        }
        true.into()
    }

    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(callback),
            LPARAM(&mut list as *mut _ as isize),
        );
    }
    for monitor in &mut list {
        let (width, height) = native_size(&monitor.device)
            .map(|native| oriented(native, (monitor.width, monitor.height)))
            .unwrap_or((monitor.width, monitor.height));
        monitor.native_width = width;
        monitor.native_height = height;
    }
    // Primary first, then left to right.
    list.sort_by_key(|m| (!m.primary, m.x, m.y));
    list
}

/// `native` turned the way the screen currently is: a monitor in portrait reports its
/// preferred mode in landscape.
fn oriented(native: (u32, u32), current: (u32, u32)) -> (u32, u32) {
    if (native.0 > native.1) != (current.0 > current.1) && current.0 != current.1 {
        (native.1, native.0)
    } else {
        native
    }
}

/// The preferred (native) resolution of the monitor shown as `device` (`\.\DISPLAY1`).
fn native_size(device: &str) -> Option<(u32, u32)> {
    use windows::Win32::Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DISPLAYCONFIG_TARGET_PREFERRED_MODE, DisplayConfigGetDeviceInfo,
        GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
    };
    unsafe {
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            .ok()
            .ok()?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
        .ok()
        .ok()?;
        for path in &paths[..path_count as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let name = &source.viewGdiDeviceName;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            if !String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case(device) {
                continue;
            }
            let mut preferred = DISPLAYCONFIG_TARGET_PREFERRED_MODE {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_PREFERRED_MODE,
                    size: size_of::<DISPLAYCONFIG_TARGET_PREFERRED_MODE>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut preferred.header) != 0
                || preferred.width == 0
                || preferred.height == 0
            {
                return None;
            }
            return Some((preferred.width, preferred.height));
        }
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CaptureTarget {
    /// Raw HMONITOR value.
    Monitor(isize),
    /// Raw HWND value.
    Window(isize),
}

/// A captured frame. With Windows Graphics Capture the texture belongs to the capture frame
/// pool and is returned to it when this value is dropped.
pub struct CapturedFrame {
    frame: Option<Direct3D11CaptureFrame>,
    pub texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    /// Capture time on the QPC clock, in 100 ns units.
    pub time_100ns: i64,
    /// Size of `texture` itself, which is the pool's and can be smaller than the picture.
    texture_size: (u32, u32),
}

impl CapturedFrame {
    /// The whole picture is in the texture. Right after the captured surface grew (a screen
    /// back from a stretched mode, a window made bigger), a frame can still come in the old,
    /// smaller pool texture with the picture cut off; copying from it reads past its edges,
    /// which Direct3D skips, so the encoder would send an older picture instead.
    pub fn is_whole(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.texture_size.0 >= self.width
            && self.texture_size.1 >= self.height
    }
}

impl Drop for CapturedFrame {
    fn drop(&mut self) {
        if let Some(frame) = &self.frame {
            let _ = frame.Close();
        }
    }
}

struct Shared {
    latest: Mutex<Option<Direct3D11CaptureFrame>>,
    arrived: AtomicU64,
    /// Windows closed the capture (the screen setup changed, the window closed): no frame
    /// comes any more, and the capture has to be started again.
    closed: AtomicBool,
}

/// Windows Graphics Capture: windows, and screens Desktop Duplication cannot take.
struct Wgc {
    item: GraphicsCaptureItem,
    device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    token: i64,
    closed_token: i64,
    format: DirectXPixelFormat,
}

/// DXGI Desktop Duplication of a screen. Unlike Windows Graphics Capture it sees a game in
/// exclusive fullscreen, which skips the desktop compositor: Graphics Capture then only
/// repeats the last composed picture.
struct Dupl {
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    dupl: IDXGIOutputDuplication,
    /// The screen's top-left corner on the desktop, to place the cursor.
    origin: (i32, i32),
    /// Draw the cursor in; Desktop Duplication leaves it out.
    cursor: bool,
    /// Our copy of the newest picture: the duplicated one must go back before the next.
    copy: RefCell<Option<ID3D11Texture2D>>,
}

enum Backend {
    Wgc(Wgc),
    Dupl(Dupl),
}

pub struct Capture {
    backend: Backend,
    shared: Arc<Shared>,
    pub width: u32,
    pub height: u32,
}

// WinRT capture objects are agile; the pool is created free-threaded. DXGI and Direct3D
// objects are free-threaded, the context protected by `SetMultithreadProtected`.
unsafe impl Send for Capture {}

impl Capture {
    /// `max_fps` is the highest rate the caller will consume. Windows delivers at most
    /// 60 frames per second unless asked for more.
    /// `hdr` captures 16-bit float frames (scRGB), for a screen in HDR mode.
    pub fn start(
        gpu: &Gpu,
        target: CaptureTarget,
        capture_cursor: bool,
        max_fps: u32,
        hdr: bool,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            arrived: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        });
        if let CaptureTarget::Monitor(handle) = target {
            match Dupl::start(gpu, HMONITOR(handle as *mut _), capture_cursor, hdr) {
                Ok((dupl, (width, height))) => {
                    return Ok(Self {
                        backend: Backend::Dupl(dupl),
                        shared,
                        width,
                        height,
                    });
                }
                // A screen on another graphics card (a laptop's built-in screen), or too many
                // duplications at once: Graphics Capture still works, without fullscreen games.
                Err(err) => log::warn!("desktop duplication unavailable, using graphics capture: {err:#}"),
            }
        }
        let (wgc, (width, height)) = Wgc::start(gpu, target, capture_cursor, max_fps, hdr, &shared)?;
        Ok(Self {
            backend: Backend::Wgc(wgc),
            shared,
            width,
            height,
        })
    }

    /// Takes the newest frame, if one arrived since the last call.
    pub fn take_latest(&self) -> Option<CapturedFrame> {
        match &self.backend {
            Backend::Wgc(_) => wgc_take(&self.shared),
            Backend::Dupl(dupl) => match unsafe { dupl.take(&self.shared) } {
                Ok(frame) => frame,
                Err(err) => {
                    // Lost on a mode change, a fullscreen switch or the secure desktop: the
                    // recorder starts the capture again.
                    log::info!("desktop duplication ended: {err}");
                    self.shared.closed.store(true, Ordering::Relaxed);
                    None
                }
            },
        }
    }

    /// Tells Windows the surface changed size (a window was resized). Frames from then on
    /// have the new size; one or two older-sized frames may still arrive first.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if let Backend::Wgc(wgc) = &self.backend {
            wgc.pool.Recreate(
                &wgc.device,
                wgc.format,
                3,
                windows::Graphics::SizeInt32 {
                    Width: width as i32,
                    Height: height as i32,
                },
            )?;
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Frames delivered by Windows since capture started.
    /// Desktop Duplication counts a frame when it is taken, so one tick later.
    pub fn frames_arrived(&self) -> u64 {
        self.shared.arrived.load(Ordering::Relaxed)
    }

    /// Windows ended this capture; it delivers nothing more.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Relaxed)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Backend::Wgc(wgc) = &self.backend {
            let _ = wgc.pool.RemoveFrameArrived(wgc.token);
            let _ = wgc.item.RemoveClosed(wgc.closed_token);
            let _ = wgc.session.Close();
            let _ = wgc.pool.Close();
        }
        if let Some(frame) = self.shared.latest.lock().unwrap().take() {
            let _ = frame.Close();
        }
    }
}

impl Wgc {
    fn start(
        gpu: &Gpu,
        target: CaptureTarget,
        capture_cursor: bool,
        max_fps: u32,
        hdr: bool,
        shared: &Arc<Shared>,
    ) -> Result<(Self, (u32, u32))> {
        let format = if hdr {
            DirectXPixelFormat::R16G16B16A16Float
        } else {
            DirectXPixelFormat::B8G8R8A8UIntNormalized
        };
        unsafe {
            // Fails harmlessly if the thread already has an apartment.
            let _ = RoInitialize(RO_INIT_MULTITHREADED);

            let interop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = match target {
                CaptureTarget::Monitor(handle) => interop
                    .CreateForMonitor(HMONITOR(handle as *mut _))
                    .context("cannot capture this monitor")?,
                CaptureTarget::Window(handle) => interop
                    .CreateForWindow(HWND(handle as *mut _))
                    .context("cannot capture this window")?,
            };

            let dxgi_device: IDXGIDevice = gpu.device.cast()?;
            let d3d_device: IDirect3DDevice =
                CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)?.cast()?;

            let size = item.Size()?;
            let pool =
                Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d_device, format, 3, size)?;

            let closed_shared = shared.clone();
            let closed_token = item.Closed(&TypedEventHandler::<
                GraphicsCaptureItem,
                IInspectable,
            >::new(move |_, _| {
                closed_shared.closed.store(true, Ordering::Relaxed);
                Ok(())
            }))?;
            let handler_shared = shared.clone();
            let token = pool.FrameArrived(&TypedEventHandler::<
                Direct3D11CaptureFramePool,
                IInspectable,
            >::new(move |pool, _| {
                let Some(pool) = pool.as_ref() else {
                    return Ok(());
                };
                // Keep only the newest frame; closing the old one frees its pool buffer.
                if let Ok(frame) = pool.TryGetNextFrame() {
                    handler_shared.arrived.fetch_add(1, Ordering::Relaxed);
                    let old = handler_shared.latest.lock().unwrap().replace(frame);
                    if let Some(old) = old {
                        let _ = old.Close();
                    }
                }
                Ok(())
            }))?;

            let session = pool.CreateCaptureSession(&item)?;
            allow_borderless();
            // Not available on older Windows builds; capture still works without them.
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(capture_cursor);
            // Ask for twice the consumed rate so every encoder tick finds a fresh frame
            // even when the two clocks drift against each other.
            let interval_100ns = (10_000_000 / (max_fps.max(1) as i64 * 2)).max(10_000);
            if let Err(err) = session.SetMinUpdateInterval(TimeSpan {
                Duration: interval_100ns,
            }) {
                log::warn!("cannot raise the capture rate above 60 fps: {err}");
            }
            session.StartCapture()?;

            Ok((
                Self {
                    item,
                    device: d3d_device,
                    pool,
                    session,
                    token,
                    closed_token,
                    format,
                },
                (size.Width as u32, size.Height as u32),
            ))
        }
    }
}

fn wgc_take(shared: &Shared) -> Option<CapturedFrame> {
    let frame = shared.latest.lock().unwrap().take()?;
    let convert = || -> Result<CapturedFrame> {
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        let size = frame.ContentSize()?;
        let time = frame.SystemRelativeTime()?;
        Ok(CapturedFrame {
            frame: Some(frame.clone()),
            texture,
            width: size.Width as u32,
            height: size.Height as u32,
            time_100ns: time.Duration,
            texture_size: (desc.Width, desc.Height),
        })
    };
    match convert() {
        Ok(captured) => Some(captured),
        Err(err) => {
            log::warn!("dropping captured frame: {err:#}");
            let _ = frame.Close();
            None
        }
    }
}

impl Dupl {
    /// Fails for a screen on another graphics card than `gpu`'s.
    fn start(gpu: &Gpu, monitor: HMONITOR, cursor: bool, hdr: bool) -> Result<(Self, (u32, u32))> {
        unsafe {
            let adapter = gpu.device.cast::<IDXGIDevice>()?.GetAdapter()?;
            let output = (0..)
                .map_while(|i| adapter.EnumOutputs(i).ok())
                .find(|o| o.GetDesc().is_ok_and(|d| d.Monitor == monitor))
                .context("the screen is not on the recording graphics card")?;
            let rect = output.GetDesc()?.DesktopCoordinates;
            let format = if hdr {
                DXGI_FORMAT_R16G16B16A16_FLOAT
            } else {
                DXGI_FORMAT_B8G8R8A8_UNORM
            };
            let dupl = output
                .cast::<IDXGIOutput5>()?
                .DuplicateOutput1(&gpu.device, 0, &[format])
                .context("DuplicateOutput1")?;
            let mode = dupl.GetDesc().ModeDesc;
            log::info!("capturing the screen by desktop duplication, {}x{}", mode.Width, mode.Height);
            Ok((
                Self {
                    context: gpu.context.clone(),
                    device: gpu.device.clone(),
                    dupl,
                    origin: (rect.left, rect.top),
                    // GDI draws only on 8-bit pictures.
                    // ponytail: no cursor on an HDR screen; draw the pointer shape with a shader if wanted.
                    cursor: cursor && !hdr,
                    copy: RefCell::new(None),
                },
                (mode.Width, mode.Height),
            ))
        }
    }

    /// The newest picture, if the screen changed since the last call. An error means the
    /// duplication is lost and has to be started again.
    unsafe fn take(&self, shared: &Shared) -> windows::core::Result<Option<CapturedFrame>> {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            match self.dupl.AcquireNextFrame(0, &mut info, &mut resource) {
                Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                other => other?,
            }
            let first = shared.arrived.load(Ordering::Relaxed) == 0;
            // Only the pointer moved: new only when the cursor is drawn in.
            if info.LastPresentTime == 0 && !first && !self.cursor {
                self.dupl.ReleaseFrame()?;
                return Ok(None);
            }
            let copied = resource
                .ok_or_else(|| windows::core::Error::from(E_FAIL))
                .and_then(|r| r.cast::<ID3D11Texture2D>())
                .and_then(|source| self.copy_of(&source));
            self.dupl.ReleaseFrame()?;
            let (texture, width, height) = copied?;
            if self.cursor {
                // A cursor that cannot be drawn leaves the picture as it is.
                let _ = self.draw_cursor(&texture);
            }
            shared.arrived.fetch_add(1, Ordering::Relaxed);
            let mut now = 0i64;
            let mut freq = 1i64;
            let _ = QueryPerformanceCounter(&mut now);
            let _ = QueryPerformanceFrequency(&mut freq);
            Ok(Some(CapturedFrame {
                frame: None,
                texture,
                width,
                height,
                time_100ns: (now as i128 * 10_000_000 / freq.max(1) as i128) as i64,
                texture_size: (width, height),
            }))
        }
    }

    /// Copies `source` into our own texture, made again when the size or format changes.
    /// ponytail: a rotated screen comes out sideways; rotate on copy if anyone records portrait.
    unsafe fn copy_of(
        &self,
        source: &ID3D11Texture2D,
    ) -> windows::core::Result<(ID3D11Texture2D, u32, u32)> {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut desc);
            let mut copy = self.copy.borrow_mut();
            let fits = copy.as_ref().is_some_and(|c| {
                let mut have = D3D11_TEXTURE2D_DESC::default();
                c.GetDesc(&mut have);
                (have.Width, have.Height, have.Format) == (desc.Width, desc.Height, desc.Format)
            });
            if !fits {
                let gdi = self.cursor && desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM;
                let wanted = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: if gdi {
                        D3D11_RESOURCE_MISC_GDI_COMPATIBLE.0 as u32
                    } else {
                        0
                    },
                };
                let mut texture = None;
                self.device.CreateTexture2D(&wanted, None, Some(&mut texture))?;
                *copy = texture;
            }
            let texture = copy.clone().ok_or_else(|| windows::core::Error::from(E_FAIL))?;
            self.context.CopyResource(&texture, source);
            Ok((texture, desc.Width, desc.Height))
        }
    }

    unsafe fn draw_cursor(&self, texture: &ID3D11Texture2D) -> windows::core::Result<()> {
        unsafe {
            let mut cursor = CURSORINFO {
                cbSize: size_of::<CURSORINFO>() as u32,
                ..Default::default()
            };
            GetCursorInfo(&mut cursor)?;
            // Hidden, as in most games.
            if cursor.flags.0 & CURSOR_SHOWING.0 == 0 || cursor.hCursor.is_invalid() {
                return Ok(());
            }
            let icon = HICON(cursor.hCursor.0);
            let mut info = ICONINFO::default();
            GetIconInfo(icon, &mut info)?;
            let _ = DeleteObject(info.hbmMask.into());
            if !info.hbmColor.is_invalid() {
                let _ = DeleteObject(info.hbmColor.into());
            }
            let surface: IDXGISurface1 = texture.cast()?;
            let dc = surface.GetDC(false)?;
            let _ = DrawIconEx(
                dc,
                cursor.ptScreenPos.x - self.origin.0 - info.xHotspot as i32,
                cursor.ptScreenPos.y - self.origin.1 - info.yHotspot as i32,
                icon,
                0,
                0,
                0,
                None,
                DI_NORMAL,
            );
            surface.ReleaseDC(None)
        }
    }
}
