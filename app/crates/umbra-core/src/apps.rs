//! Lists of programs in the settings (games, presets, never record). An entry is a program
//! name (`Game.exe`, or `Game` alone), the full path of a program, or a folder: everything
//! started from inside it.

use std::collections::HashMap;
use std::sync::Mutex;

/// True when the entry names a file or folder rather than a program name.
pub fn is_path(entry: &str) -> bool {
    entry.contains('\\') || entry.contains('/')
}

/// True when `entry` stands for the program `exe` (file name) at `path` (its full path, if
/// known). A path entry never matches without the path.
pub fn matches(entry: &str, exe: &str, path: Option<&str>) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    if !is_path(entry) {
        return exe.eq_ignore_ascii_case(entry)
            || exe
                .strip_suffix(".exe")
                .or_else(|| exe.strip_suffix(".EXE"))
                .is_some_and(|stem| stem.eq_ignore_ascii_case(entry));
    }
    let Some(path) = path else {
        return false;
    };
    let entry = normalize(entry);
    let path = normalize(path);
    if path == entry {
        return true;
    }
    // A folder: the program is inside it, at any depth.
    let folder = entry.trim_end_matches('\\');
    path.len() > folder.len() + 1
        && path.starts_with(folder)
        && path.as_bytes()[folder.len()] == b'\\'
}

/// True when `entry` could match the program `exe` once its path is known: a program name,
/// a program's path with that file name, or a folder. Saves looking paths up for nothing.
pub fn could_match(entry: &str, exe: &str) -> bool {
    if !is_path(entry) {
        return matches(entry, exe, None);
    }
    let name = normalize(entry);
    let name = name.trim_end_matches('\\');
    let file = name.rsplit('\\').next().unwrap_or(name);
    !file.ends_with(".exe") || file.eq_ignore_ascii_case(exe)
}

/// True when any entry of `list` stands for the program `exe` of the process `pid`.
pub fn listed(list: &[String], exe: &str, pid: u32) -> bool {
    let mut path: Option<Option<String>> = None;
    list.iter().any(|entry| {
        if !could_match(entry, exe) {
            return false;
        }
        if !is_path(entry) {
            return true;
        }
        let path = path.get_or_insert_with(|| path_of(pid, exe));
        matches(entry, exe, path.as_deref())
    })
}

fn normalize(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

/// The full path of the program running as `pid`, which is `exe`. Remembered per process,
/// since the lists are checked every second.
pub fn path_of(pid: u32, exe: &str) -> Option<String> {
    static PATHS: Mutex<Option<HashMap<u32, (String, Option<String>)>>> = Mutex::new(None);
    let mut paths = PATHS.lock().unwrap();
    let paths = paths.get_or_insert_with(HashMap::new);
    if let Some((known, path)) = paths.get(&pid) {
        // A process id can be reused by another program.
        if known.eq_ignore_ascii_case(exe) {
            return path.clone();
        }
    }
    if paths.len() > 4096 {
        paths.clear();
    }
    let path = crate::games::image_path(pid);
    paths.insert(pid, (exe.to_string(), path.clone()));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_paths_and_folders() {
        let path = Some(r"C:\Games\EA\Battlefield V\bfv.exe");
        assert!(matches("bfv.exe", "bfv.exe", path));
        assert!(matches("BFV.EXE", "bfv.exe", None));
        assert!(matches("bfv", "bfv.exe", None));
        assert!(!matches("bf", "bfv.exe", None));
        assert!(matches(
            r"C:\Games\EA\Battlefield V\bfv.exe",
            "bfv.exe",
            path
        ));
        assert!(matches(
            r"c:/games/ea/battlefield v/BFV.exe",
            "bfv.exe",
            path
        ));
        assert!(matches(r"C:\Games\EA", "bfv.exe", path));
        assert!(matches(r"C:\Games\EA\", "bfv.exe", path));
        // Not a folder of it: a name that only starts the same way.
        assert!(!matches(r"C:\Games\E", "bfv.exe", path));
        assert!(!matches(r"D:\Games\EA", "bfv.exe", path));
        // A path entry needs the path.
        assert!(!matches(r"C:\Games\EA", "bfv.exe", None));
        assert!(!matches("", "bfv.exe", path));
    }

    #[test]
    fn path_lookups_only_when_they_can_match() {
        assert!(could_match(r"C:\Games\EA\Battlefield V\bfv.exe", "bfv.exe"));
        assert!(!could_match(
            r"C:\Games\EA\Battlefield V\bfv.exe",
            "cs2.exe"
        ));
        assert!(could_match(r"C:\Games\EA", "cs2.exe"));
        assert!(could_match("cs2", "cs2.exe"));
        assert!(!could_match("bfv.exe", "cs2.exe"));
    }
}
