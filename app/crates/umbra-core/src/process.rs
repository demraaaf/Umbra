//! Finding processes by executable name.

use std::collections::HashSet;

use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

#[derive(Clone, Debug)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub exe: String,
}

pub fn list_processes() -> Vec<ProcessEntry> {
    let mut list = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return list;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            list.push(ProcessEntry {
                pid: entry.th32ProcessID,
                parent_pid: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
    }
    list
}

/// Process ids of the top-level processes running `exe`, meaning the ones whose parent is
/// not the same program. Capturing a root also captures its child processes, which is how
/// apps like Discord and Chrome play sound from a helper process.
pub fn find_roots(exe: &str) -> Vec<u32> {
    let all = list_processes();
    let matching: Vec<&ProcessEntry> = all
        .iter()
        .filter(|p| p.exe.eq_ignore_ascii_case(exe))
        .collect();
    let pids: HashSet<u32> = matching.iter().map(|p| p.pid).collect();
    matching
        .iter()
        .filter(|p| !pids.contains(&p.parent_pid))
        .map(|p| p.pid)
        .collect()
}
