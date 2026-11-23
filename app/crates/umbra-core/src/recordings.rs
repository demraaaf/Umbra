//! What Umbra keeps about its recordings besides the file itself: the bookmarks set with the
//! clip hotkey while recording. They live in `recordings.json` in the settings folder, keyed
//! by the file's path, because a file being written as it records can't take them in later.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How a recording was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingKind {
    /// With the Record button or hotkey.
    Manual,
    /// A whole game session: from the game starting (or the hotkey) until it closes.
    Session,
}

impl RecordingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Session => "session",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingInfo {
    /// Seconds from the start of the recording.
    pub bookmarks: Vec<f64>,
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn store_path() -> PathBuf {
    crate::settings::data_dir().join("recordings.json")
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn read(file: &Path) -> BTreeMap<String, RecordingInfo> {
    std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write(file: &Path, all: &BTreeMap<String, RecordingInfo>) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let partial = file.with_extension("json.part");
    let saved = serde_json::to_string_pretty(all)
        .ok()
        .and_then(|text| std::fs::write(&partial, text).ok())
        .and_then(|_| std::fs::rename(&partial, file).ok());
    if saved.is_none() {
        log::warn!("cannot write {}", file.display());
    }
}

/// Everything known, keyed by lowercased path.
pub fn load_in(file: &Path) -> BTreeMap<String, RecordingInfo> {
    let _guard = LOCK.lock().unwrap();
    read(file)
}

/// The bookmarks of the recording at `path`, if it has any.
pub fn get_in(file: &Path, path: &Path) -> Option<RecordingInfo> {
    load_in(file).remove(&key(path))
}

pub fn put_in(file: &Path, path: &Path, info: RecordingInfo) {
    let _guard = LOCK.lock().unwrap();
    let mut all = read(file);
    all.insert(key(path), info);
    write(file, &all);
}

/// Follows a renamed recording.
pub fn rename_in(file: &Path, from: &Path, to: &Path) {
    let _guard = LOCK.lock().unwrap();
    let mut all = read(file);
    if let Some(info) = all.remove(&key(from)) {
        all.insert(key(to), info);
        write(file, &all);
    }
}

pub fn remove_in(file: &Path, path: &Path) {
    let _guard = LOCK.lock().unwrap();
    let mut all = read(file);
    if all.remove(&key(path)).is_some() {
        write(file, &all);
    }
}

pub fn load() -> BTreeMap<String, RecordingInfo> {
    load_in(&store_path())
}

pub fn get(path: &Path) -> Option<RecordingInfo> {
    get_in(&store_path(), path)
}

pub fn put(path: &Path, info: RecordingInfo) {
    put_in(&store_path(), path, info)
}

pub fn rename(from: &Path, to: &Path) {
    rename_in(&store_path(), from, to)
}

pub fn remove(path: &Path) {
    remove_in(&store_path(), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_follow_renames_and_go_with_deletes() {
        let dir = std::env::temp_dir().join("umbra-recordings-test");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("recordings.json");
        let a = Path::new(r"C:\Clips\Game\Session.mp4");
        let b = Path::new(r"C:\Clips\Game\Best bits.mp4");
        assert_eq!(get_in(&file, a), None);
        put_in(
            &file,
            a,
            RecordingInfo {
                bookmarks: vec![12.5, 300.0],
            },
        );
        // Windows paths are not case-sensitive.
        let upper = PathBuf::from(r"C:\CLIPS\Game\Session.mp4");
        assert_eq!(get_in(&file, &upper).unwrap().bookmarks, vec![12.5, 300.0]);
        rename_in(&file, a, b);
        assert_eq!(get_in(&file, a), None);
        assert_eq!(get_in(&file, b).unwrap().bookmarks.len(), 2);
        remove_in(&file, b);
        assert!(load_in(&file).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
