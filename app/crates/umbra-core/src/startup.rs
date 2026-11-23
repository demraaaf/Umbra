//! Starting Umbra when Windows starts, through the per-user Run registry key. No admin
//! rights needed, and nothing outside the current user's settings is touched.

use anyhow::{Result, bail};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("Umbra");

/// Adds or removes the Run entry. `command` is the full command line to start, quoted if
/// the path has spaces.
pub fn set_launch_at_startup(enabled: bool, command: &str) -> Result<()> {
    unsafe {
        let mut key = HKEY::default();
        let status = RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key);
        if status.is_err() {
            bail!("cannot open the startup registry key: {status:?}");
        }
        let result = if enabled {
            let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(key, VALUE_NAME, None, REG_SZ, Some(bytes))
        } else {
            let status = RegDeleteValueW(key, VALUE_NAME);
            if status == ERROR_FILE_NOT_FOUND {
                windows::Win32::Foundation::ERROR_SUCCESS
            } else {
                status
            }
        };
        let _ = RegCloseKey(key);
        if result.is_err() {
            bail!("cannot update the startup registry key: {result:?}");
        }
        Ok(())
    }
}

/// The command line Windows starts at login, if there is an entry.
pub fn launch_command() -> Option<String> {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_QUERY_VALUE, &mut key).is_err() {
            return None;
        }
        let mut len = 0u32;
        let mut text = None;
        if RegQueryValueExW(key, VALUE_NAME, None, None, None, Some(&mut len)).is_ok() && len > 0 {
            let mut buffer = vec![0u16; (len as usize).div_ceil(2)];
            if RegQueryValueExW(
                key,
                VALUE_NAME,
                None,
                None,
                Some(buffer.as_mut_ptr() as *mut u8),
                Some(&mut len),
            )
            .is_ok()
            {
                let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
                text = Some(String::from_utf16_lossy(&buffer[..end]));
            }
        }
        let _ = RegCloseKey(key);
        text
    }
}

pub fn is_launch_at_startup() -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_QUERY_VALUE, &mut key).is_err() {
            return false;
        }
        let found = RegQueryValueExW(key, VALUE_NAME, None, None, None, None).is_ok();
        let _ = RegCloseKey(key);
        found
    }
}
