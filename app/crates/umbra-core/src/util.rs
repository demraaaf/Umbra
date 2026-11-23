use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::core::BOOL;

/// Makes monitor and window sizes report real pixels. Call once at startup.
pub fn set_dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Total CPU time (kernel plus user) this process has used.
pub fn process_cpu_time() -> Duration {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return Duration::ZERO;
    }
    let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

static CTRL_C: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_ctrl(kind: u32) -> BOOL {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        CTRL_C.store(true, Ordering::Relaxed);
        return true.into();
    }
    false.into()
}

/// Makes Ctrl+C set a flag instead of killing the process, so the caller can shut down cleanly.
pub fn install_ctrl_c_handler() {
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(on_ctrl), true);
    }
}

pub fn ctrl_c_pressed() -> bool {
    CTRL_C.load(Ordering::Relaxed)
}

/// Local time as `2026-09-30 18-05-42`, safe to use in a file name.
pub fn local_timestamp() -> String {
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

pub fn local_datetime() -> crate::naming::DateTime {
    let t = unsafe { GetLocalTime() };
    crate::naming::DateTime {
        year: t.wYear,
        month: t.wMonth,
        day: t.wDay,
        hour: t.wHour,
        minute: t.wMinute,
        second: t.wSecond,
    }
}
