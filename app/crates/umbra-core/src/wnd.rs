//! Listing the windows that can be captured.

use windows::Win32::Foundation::RECT;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, GetWindowPlacement,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, WINDOWPLACEMENT,
    WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::process::list_processes;

#[derive(Clone, Debug)]
pub struct WindowEntry {
    /// Raw HWND value.
    pub handle: isize,
    pub pid: u32,
    pub exe: String,
    pub title: String,
}

/// Top-level windows a user would recognise: visible, titled, not hidden helper windows.
pub fn list_windows() -> Vec<WindowEntry> {
    unsafe extern "system" fn callback(window: HWND, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<(isize, u32, String)>) };
        unsafe {
            if !IsWindowVisible(window).as_bool() {
                return true.into();
            }
            // Windows on other virtual desktops and suspended store apps are "cloaked".
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(
                window,
                DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                size_of::<u32>() as u32,
            );
            if cloaked != 0 {
                return true.into();
            }
            if GetWindow(window, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
                return true.into();
            }
            if GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                return true.into();
            }
            let mut buffer = [0u16; 256];
            let len = GetWindowTextW(window, &mut buffer);
            if len <= 0 {
                return true.into();
            }
            let title = String::from_utf16_lossy(&buffer[..len as usize]);
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            list.push((window.0 as isize, pid, title));
        }
        true.into()
    }

    let mut found: Vec<(isize, u32, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut found as *mut _ as isize));
    }
    let processes = list_processes();
    found
        .into_iter()
        .filter_map(|(handle, pid, title)| {
            let exe = processes.iter().find(|p| p.pid == pid)?.exe.clone();
            Some(WindowEntry {
                handle,
                pid,
                exe,
                title,
            })
        })
        .collect()
}

/// The window to capture for a saved choice: same program, and a title containing the saved
/// title if there is one. The first match wins.
pub fn find_window(exe: &str, title: &str) -> Option<WindowEntry> {
    let windows = list_windows();
    let same_exe = || windows.iter().filter(|w| w.exe.eq_ignore_ascii_case(exe));
    let wanted = title.to_lowercase();
    same_exe()
        .find(|w| !wanted.is_empty() && w.title.to_lowercase().contains(&wanted))
        .or_else(|| same_exe().next())
        .cloned()
}

/// True while the window is minimized. Capturing it then gives only a strip of title bar.
pub fn is_minimized(handle: isize) -> bool {
    unsafe { IsIconic(HWND(handle as *mut _)) }.as_bool()
}

/// False once the window is destroyed (its program closed it or made a new one).
pub fn exists(handle: isize) -> bool {
    unsafe { IsWindow(Some(HWND(handle as *mut _))) }.as_bool()
}

/// The size a minimized window has when it is shown again, rounded down to even numbers
/// for the encoder.
pub fn restored_size(handle: isize) -> Option<(u32, u32)> {
    let mut placement = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(HWND(handle as *mut _), &mut placement) }.ok()?;
    let r = placement.rcNormalPosition;
    let (w, h) = (
        (r.right - r.left) as u32 & !1,
        (r.bottom - r.top) as u32 & !1,
    );
    (w >= 160 && h >= 96).then_some((w, h))
}

/// The size of a window as it appears on screen (without the invisible resize border),
/// which is what capturing it produces.
pub fn window_size(handle: isize) -> Option<(u32, u32)> {
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            HWND(handle as *mut _),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut _,
            size_of::<RECT>() as u32,
        )
        .ok()?;
    }
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    (w > 0 && h > 0).then_some((w as u32, h as u32))
}
