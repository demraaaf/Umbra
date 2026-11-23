//! Storage limits: which clips to delete so the clip folder stays under a size, and clips
//! past an age go. Oldest first; clips the user wants kept (favourites, albums) never go, and
//! neither does anything saved in the last few minutes, so a fresh clip is safe from a tight
//! limit. This module only decides; the app deletes.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A file in the clip folder.
#[derive(Clone, Debug)]
pub struct LibraryFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Limits {
    /// Most the folder may hold, in bytes.
    pub max_bytes: Option<u64>,
    /// Clips older than this go.
    pub max_age: Option<Duration>,
}

/// Anything newer than this is never deleted.
pub const GRACE: Duration = Duration::from_secs(10 * 60);

/// The files to delete, oldest first.
pub fn plan(
    files: &[LibraryFile],
    keep: &dyn Fn(&Path) -> bool,
    limits: Limits,
    now: SystemTime,
) -> Vec<PathBuf> {
    let age = |f: &LibraryFile| now.duration_since(f.modified).unwrap_or(Duration::ZERO);
    let mut candidates: Vec<&LibraryFile> = files
        .iter()
        .filter(|f| age(f) >= GRACE && !keep(&f.path))
        .collect();
    candidates.sort_by_key(|f| f.modified);

    let mut total: u64 = files.iter().map(|f| f.bytes).sum();
    let mut chosen = Vec::new();
    for file in candidates {
        let too_old = limits.max_age.is_some_and(|max| age(file) > max);
        let too_big = limits.max_bytes.is_some_and(|max| total > max);
        if too_old || too_big {
            total = total.saturating_sub(file.bytes);
            chosen.push(file.path.clone());
        }
    }
    chosen
}

/// Every clip, recording and screenshot under `folder`, with its size and time.
pub fn library_files(folder: &Path) -> Vec<LibraryFile> {
    crate::library::library_paths(folder)
        .into_iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            Some(LibraryFile {
                bytes: meta.len(),
                modified: meta.modified().ok()?,
                path,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;
    const DAY: Duration = Duration::from_secs(86_400);

    fn file(name: &str, gb: u64, days_old: u64, now: SystemTime) -> LibraryFile {
        LibraryFile {
            path: PathBuf::from(name),
            bytes: gb * GB,
            modified: now - DAY * days_old as u32,
        }
    }

    #[test]
    fn the_oldest_go_until_the_folder_fits() {
        let now = SystemTime::now();
        let files = [
            file("a", 4, 10, now),
            file("b", 4, 5, now),
            file("c", 4, 1, now),
        ];
        let limits = Limits {
            max_bytes: Some(9 * GB),
            max_age: None,
        };
        let gone = plan(&files, &|_| false, limits, now);
        assert_eq!(gone, vec![PathBuf::from("a")]);
    }

    #[test]
    fn kept_and_fresh_files_stay_even_over_the_limit() {
        let now = SystemTime::now();
        let mut fresh = file("fresh", 4, 0, now);
        fresh.modified = now - Duration::from_secs(60);
        let files = [
            file("favourite", 4, 30, now),
            file("old", 4, 20, now),
            fresh,
        ];
        let limits = Limits {
            max_bytes: Some(GB),
            max_age: None,
        };
        let gone = plan(&files, &|p| p == Path::new("favourite"), limits, now);
        assert_eq!(gone, vec![PathBuf::from("old")]);
    }

    #[test]
    fn old_clips_go_whatever_the_size() {
        let now = SystemTime::now();
        let files = [
            file("a", 1, 40, now),
            file("b", 1, 20, now),
            file("c", 1, 2, now),
        ];
        let limits = Limits {
            max_bytes: None,
            max_age: Some(DAY * 30),
        };
        assert_eq!(
            plan(&files, &|_| false, limits, now),
            vec![PathBuf::from("a")]
        );
        assert!(plan(&files, &|_| false, Limits::default(), now).is_empty());
    }
}
