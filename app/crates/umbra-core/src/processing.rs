//! Files Umbra is still working on after recording, such as a long recording being rewritten
//! as an ordinary MP4, so the library can show them with how far they are.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use crate::recordings::RecordingKind;

#[derive(Clone, Debug)]
pub struct Processing {
    /// Where the file ends up.
    pub path: PathBuf,
    pub game: String,
    pub kind: RecordingKind,
    /// Bytes of the recording read so far, of `total`.
    pub done: u64,
    pub total: u64,
    pub started: Instant,
    pub started_at: SystemTime,
    /// `finish` (a recording made quick to open) or `shrink` (an old recording made smaller).
    pub task: &'static str,
}

static ITEMS: Mutex<Vec<Processing>> = Mutex::new(Vec::new());

/// One file being worked on; it leaves the list when dropped.
pub struct Job {
    path: PathBuf,
}

pub fn begin(path: &Path, game: &str, kind: RecordingKind, total: u64) -> Job {
    begin_task(path, game, kind, total, "finish")
}

/// Like [`begin`], for another kind of work.
pub fn begin_task(
    path: &Path,
    game: &str,
    kind: RecordingKind,
    total: u64,
    task: &'static str,
) -> Job {
    let mut items = ITEMS.lock().unwrap();
    items.retain(|p| p.path != path);
    items.push(Processing {
        path: path.to_path_buf(),
        game: game.to_string(),
        kind,
        done: 0,
        total,
        started: Instant::now(),
        started_at: SystemTime::now(),
        task,
    });
    Job {
        path: path.to_path_buf(),
    }
}

impl Job {
    pub fn progress(&self, done: u64, total: u64) {
        if let Some(item) = ITEMS
            .lock()
            .unwrap()
            .iter_mut()
            .find(|p| p.path == self.path)
        {
            item.done = done;
            if total > 0 {
                item.total = total;
            }
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        ITEMS.lock().unwrap().retain(|p| p.path != self.path);
    }
}

/// Everything being worked on, oldest first.
pub fn list() -> Vec<Processing> {
    ITEMS.lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_report_progress_and_leave_when_dropped() {
        let path = Path::new(r"C:\clips\Game\Game 2026-01-01 10-00-00 (session).mp4");
        let job = begin(path, "Game", RecordingKind::Session, 100);
        job.progress(40, 0);
        let item = list().into_iter().find(|p| p.path == path).unwrap();
        assert_eq!((item.done, item.total), (40, 100));
        job.progress(50, 200);
        let item = list().into_iter().find(|p| p.path == path).unwrap();
        assert_eq!((item.done, item.total), (50, 200));
        drop(job);
        assert!(list().iter().all(|p| p.path != path));
    }
}
