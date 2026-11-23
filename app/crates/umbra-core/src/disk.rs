//! Free space on the drive recordings go to.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::core::PCWSTR;

/// Bytes free for this user on the drive holding `path` (or its nearest existing parent).
pub fn free_bytes(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    let wide: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_drive_has_some_room() {
        let free = free_bytes(&std::env::temp_dir().join("does-not-exist-yet")).unwrap();
        assert!(free > 0);
    }
}
