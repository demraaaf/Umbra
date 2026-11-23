//! Files on the clipboard, the way Explorer's Copy puts them there, so a clip pastes straight
//! into Discord, a chat or a folder.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use windows::Win32::Foundation::{GlobalFree, HANDLE};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::DROPFILES;

/// Replaces the clipboard with these files.
pub fn copy_files(paths: &[&Path]) -> Result<()> {
    if paths.is_empty() {
        bail!("no files to copy");
    }
    // A DROPFILES header, then each path in UTF-16 ending in a zero, then one more zero.
    let mut list: Vec<u16> = Vec::new();
    for path in paths {
        list.extend(path.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);
    let header = size_of::<DROPFILES>();
    let bytes = header + list.len() * 2;

    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)?;
        let data = GlobalLock(memory) as *mut u8;
        if data.is_null() {
            let _ = GlobalFree(Some(memory));
            bail!("cannot prepare the clipboard data");
        }
        let drop_files = data as *mut DROPFILES;
        (*drop_files).pFiles = header as u32;
        (*drop_files).fWide = true.into();
        std::ptr::copy_nonoverlapping(list.as_ptr() as *const u8, data.add(header), list.len() * 2);
        let _ = GlobalUnlock(memory);

        // Another program can hold the clipboard for a moment.
        let mut open = false;
        for _ in 0..20 {
            if OpenClipboard(None).is_ok() {
                open = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !open {
            let _ = GlobalFree(Some(memory));
            bail!("another program is using the clipboard; try again");
        }
        let result = EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(memory.0))));
        let _ = CloseClipboard();
        match result {
            // The clipboard owns the memory now.
            Ok(_) => Ok(()),
            Err(err) => {
                let _ = GlobalFree(Some(memory));
                Err(anyhow!("cannot put the file on the clipboard: {err}"))
            }
        }
    }
}
