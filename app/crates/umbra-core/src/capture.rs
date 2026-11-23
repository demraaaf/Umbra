//! Screen capture through Windows Graphics Capture.

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
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    HDC, HMONITOR, MONITORINFOEXW,
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

/// A captured frame. The texture belongs to the capture frame pool and is returned to it
/// when this value is dropped.
pub struct CapturedFrame {
    frame: Direct3D11CaptureFrame,
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
        let _ = self.frame.Close();
    }
}

struct Shared {
    latest: Mutex<Option<Direct3D11CaptureFrame>>,
    arrived: AtomicU64,
    /// Windows closed the capture (the screen setup changed, the window closed): no frame
    /// comes any more, and the capture has to be started again.
    closed: AtomicBool,
}

pub struct Capture {
    _item: GraphicsCaptureItem,
    device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    shared: Arc<Shared>,
    token: i64,
    closed_token: i64,
    format: DirectXPixelFormat,
    pub width: u32,
    pub height: u32,
}

// WinRT capture objects are agile; the pool is created free-threaded.
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

            let shared = Arc::new(Shared {
                latest: Mutex::new(None),
                arrived: AtomicU64::new(0),
                closed: AtomicBool::new(false),
            });
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

            Ok(Self {
                _item: item,
                device: d3d_device,
                pool,
                session,
                shared,
                token,
                closed_token,
                format,
                width: size.Width as u32,
                height: size.Height as u32,
            })
        }
    }

    /// Takes the newest frame, if one arrived since the last call.
    pub fn take_latest(&self) -> Option<CapturedFrame> {
        let frame = self.shared.latest.lock().unwrap().take()?;
        let convert = || -> Result<CapturedFrame> {
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe { texture.GetDesc(&mut desc) };
            let size = frame.ContentSize()?;
            let time = frame.SystemRelativeTime()?;
            Ok(CapturedFrame {
                frame: frame.clone(),
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

    /// Tells Windows the surface changed size (a window was resized). Frames from then on
    /// have the new size; one or two older-sized frames may still arrive first.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.pool.Recreate(
            &self.device,
            self.format,
            3,
            windows::Graphics::SizeInt32 {
                Width: width as i32,
                Height: height as i32,
            },
        )?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Frames delivered by Windows since capture started.
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
        let _ = self.pool.RemoveFrameArrived(self.token);
        let _ = self._item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
        if let Some(frame) = self.shared.latest.lock().unwrap().take() {
            let _ = frame.Close();
        }
    }
}
