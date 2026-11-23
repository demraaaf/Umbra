//! Dragging clips out of Umbra into Discord, Explorer or anything else that takes files.
//!
//! The window's page starts it when a clip is dragged; this hands the files to Windows' own
//! drag and drop (`SHDoDragDrop`) as shell items, the way Explorer does, so every program
//! that accepts dropped files takes them and the usual drag picture and cursor show. It runs
//! on the window's thread, which Windows needs for a drag; it returns once the files are
//! dropped or the drag is given up.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use tauri::{AppHandle, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, ILCreateFromPathW, ILFree, IShellItemArray, SHCreateShellItemArrayFromIDLists,
    SHDoDragDrop,
};
use windows::core::PCWSTR;

/// Drags the clips at `paths` (in the clip folder) out of `window`. True when they were dropped
/// somewhere.
pub fn drag(app: &AppHandle, window: &WebviewWindow, paths: &[String]) -> Result<bool> {
    if paths.is_empty() {
        return Ok(false);
    }
    let files: Vec<PathBuf> = paths
        .iter()
        .map(|p| crate::clips::inside_clip_folder(app, p))
        .collect::<Result<_>>()?;
    let hwnd = window.hwnd().context("no window to drag from")?;
    // Only while the mouse button is still held: a drag started after it was let go would
    // drop the files wherever the pointer moves next.
    if !primary_button_down() {
        return Ok(false);
    }
    unsafe {
        // Shell item ids for the files; freed whatever happens.
        struct Ids(Vec<*mut ITEMIDLIST>);
        impl Drop for Ids {
            fn drop(&mut self) {
                for id in &self.0 {
                    unsafe { ILFree(Some(*id)) };
                }
            }
        }
        let mut ids = Ids(Vec::new());
        for file in &files {
            let wide: Vec<u16> = file
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let id = ILCreateFromPathW(PCWSTR(wide.as_ptr()));
            if id.is_null() {
                bail!("{} cannot be dragged", file.display());
            }
            ids.0.push(id);
        }
        let list: Vec<*const ITEMIDLIST> = ids.0.iter().map(|p| *p as *const _).collect();
        let items: IShellItemArray =
            SHCreateShellItemArrayFromIDLists(&list).context("cannot list the files to drag")?;
        let data: IDataObject = items
            .BindToHandler(None, &BHID_DataObject)
            .context("cannot offer the files")?;
        // A null drop source lets the shell supply its own, with the drag picture.
        let effect: DROPEFFECT = SHDoDragDrop(
            Some(HWND(hwnd.0 as *mut _)),
            &data,
            None::<&windows::Win32::System::Ole::IDropSource>,
            DROPEFFECT_COPY,
        )
        .context("the drag did not start")?;
        Ok(effect.0 != 0)
    }
}

/// The primary mouse button (the right one when the buttons are swapped) is held down.
fn primary_button_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
    unsafe {
        let key = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        };
        (GetAsyncKeyState(key.0 as i32) as u16 & 0x8000) != 0
    }
}
