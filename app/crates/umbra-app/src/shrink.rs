//! Making old recordings smaller in the background (Settings, Storage).
//!
//! A recording at Umbra's quality takes about 1 GB a minute. Once it is older than the days
//! chosen, it is encoded again as HEVC at a bitrate fitting its size and frame rate, on the
//! graphics card, one file at a time. Every audio track, tag and the date stay; the new file
//! takes the original's name, so albums and bookmarks keep pointing at it. The original goes
//! (to the Recycle Bin unless Storage says otherwise) only once the new file checks out.
//! Nothing is shrunk while a game runs or a recording to a file is made, so it never competes
//! with a game; a shrink in progress stops and is tried again later. A file the user stops
//! (the Compressing card's Stop) is left alone from then on.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use umbra_core::export::{ExportJob, is_cancelled};
use umbra_core::library::{self, ClipMeta};
use umbra_core::recordings::RecordingKind;
use umbra_core::settings::{Settings, ShrinkPrefs, ShrinkQuality};

use crate::AppState;

/// Smaller than this is not worth the time.
const MIN_BYTES: u64 = 200 << 20;
/// A file already within this factor of the target bitrate is left alone.
const SMALL_ENOUGH: f64 = 1.3;

static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Files asked for by hand (right-click, Compress), at the quality asked for. They go first,
/// whatever their age and whether compressing old recordings is on.
static QUEUE: Mutex<Vec<(PathBuf, ShrinkQuality)>> = Mutex::new(Vec::new());
/// The file being compressed now.
static CURRENT: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The user asked to stop compressing the current file.
static STOP: AtomicBool = AtomicBool::new(false);

/// Starts the background worker: a look a few minutes after start, then every half hour,
/// and whenever `wake` is called (the setting turned on, Shrink now).
pub fn start(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("umbra-shrink".into())
        .spawn(move || {
            let mut first = true;
            loop {
                // Files asked for by hand are tried again soon after a game closes.
                let wait = if !QUEUE.lock().unwrap().is_empty() {
                    Duration::from_secs(60)
                } else if first {
                    Duration::from_secs(180)
                } else {
                    Duration::from_secs(1800)
                };
                first = false;
                let (lock, signal) = &WAKE;
                let mut woken = lock.lock().unwrap();
                if !*woken {
                    woken = signal.wait_timeout(woken, wait).unwrap().0;
                }
                *woken = false;
                drop(woken);
                run_pass(&app);
            }
        });
}

/// Looks for recordings to shrink now.
pub fn wake() {
    let (lock, signal) = &WAKE;
    *lock.lock().unwrap() = true;
    signal.notify_all();
}

/// What happened to a file asked for by hand, for a message in the window.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ShrinkResult {
    path: String,
    file_name: String,
    /// Bytes saved, when it was compressed.
    saved_bytes: Option<u64>,
    /// Why it was left as it is or did not work.
    problem: Option<String>,
    /// The user stopped it.
    stopped: bool,
}

/// What asking to compress files did.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShrinkRequest {
    pub queued: usize,
    /// A game runs or a recording is made: compressing starts afterwards.
    pub waiting: bool,
}

/// Queues files to compress now, at `quality` (the setting's when not given).
pub fn request(
    app: &AppHandle,
    paths: &[String],
    quality: Option<ShrinkQuality>,
) -> Result<ShrinkRequest> {
    let quality = quality.unwrap_or(
        app.state::<AppState>()
            .controller
            .settings()
            .storage
            .shrink
            .quality,
    );
    let mut queue = QUEUE.lock().unwrap();
    let mut queued = 0;
    for p in paths {
        let path = crate::clips::inside_clip_folder(app, p)?;
        if library::is_image_path(&path) || queue.iter().any(|(q, _)| *q == path) {
            continue;
        }
        forget_skipped(&path);
        queue.push((path, quality));
        queued += 1;
    }
    drop(queue);
    wake();
    Ok(ShrinkRequest {
        queued,
        waiting: busy(app),
    })
}

/// Stops compressing these files: the one being compressed now, and any still waiting. They
/// are not compressed automatically afterwards either. Returns how many were stopped.
pub fn stop(app: &AppHandle, paths: &[String]) -> Result<usize> {
    let mut stopped = 0;
    for p in paths {
        let path = crate::clips::inside_clip_folder(app, p)?;
        remember_skipped(&path);
        let mut queue = QUEUE.lock().unwrap();
        let before = queue.len();
        queue.retain(|(q, _)| *q != path);
        let removed = before != queue.len();
        drop(queue);
        if CURRENT.lock().unwrap().as_ref() == Some(&path) {
            STOP.store(true, Ordering::SeqCst);
            stopped += 1;
        } else if removed {
            send_result(app, &path, None, None, true);
            stopped += 1;
        }
    }
    Ok(stopped)
}

/// Files the user stopped, kept across restarts so the automatic pass leaves them alone.
fn skip_file() -> PathBuf {
    umbra_core::settings::cache_dir().join("compress-skip.txt")
}

fn skipped() -> Vec<PathBuf> {
    std::fs::read_to_string(skip_file())
        .map(|text| {
            text.lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn remember_skipped(path: &Path) {
    let mut list = skipped();
    if list.iter().any(|p| p == path) {
        return;
    }
    list.push(path.to_path_buf());
    let text: String = list.iter().map(|p| format!("{}\n", p.display())).collect();
    let _ = std::fs::create_dir_all(umbra_core::settings::cache_dir());
    let _ = std::fs::write(skip_file(), text);
}

/// Asking for a file by hand compresses it even when it was stopped before.
fn forget_skipped(path: &Path) {
    let list = skipped();
    if !list.iter().any(|p| p == path) {
        return;
    }
    let text: String = list
        .iter()
        .filter(|p| *p != path)
        .map(|p| format!("{}\n", p.display()))
        .collect();
    let _ = std::fs::write(skip_file(), text);
}

fn report(app: &AppHandle, path: &Path, saved: Option<u64>, problem: Option<String>) {
    send_result(app, path, saved, problem, false);
}

fn send_result(
    app: &AppHandle,
    path: &Path,
    saved: Option<u64>,
    problem: Option<String>,
    stopped: bool,
) {
    let _ = app.emit(
        "shrink-result",
        ShrinkResult {
            stopped,
            path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            saved_bytes: saved,
            problem,
        },
    );
}

/// Works through the files asked for by hand. Leaves the rest queued when something gets busy.
fn run_queue(app: &AppHandle, settings: &Settings) {
    loop {
        if busy(app) {
            return;
        }
        let Some((path, quality)) = QUEUE.lock().unwrap().first().cloned() else {
            return;
        };
        let done = |problem: Option<String>, saved: Option<u64>| {
            QUEUE.lock().unwrap().retain(|(p, _)| *p != path);
            report(app, &path, saved, problem);
        };
        let Ok(meta) = library::probe(&path) else {
            done(Some("it cannot be read".into()), None);
            continue;
        };
        if library::is_ten_bit(&path) {
            done(
                Some("it is an HDR recording, which stays as it is".into()),
                None,
            );
            continue;
        }
        let prefs = ShrinkPrefs {
            quality,
            ..settings.storage.shrink.clone()
        };
        let rate = target_bitrate(meta.width, meta.height, meta.fps, &prefs);
        let fewer_frames = fps_cap(meta.fps, &prefs).is_some();
        if !fewer_frames && meta.bit_rate > 0 && meta.bit_rate as f64 <= rate as f64 * SMALL_ENOUGH
        {
            done(Some("it is already about that small".into()), None);
            continue;
        }
        match shrink_one(app, settings, &meta, rate, &prefs) {
            Ok(saved) => {
                log::info!(
                    "compressed {} by {:.1} GB",
                    path.display(),
                    saved as f64 / 1e9
                );
                let _ = app.emit("clips-changed", ());
                done(None, Some(saved));
            }
            Err(err) if is_cancelled(&err) && STOP.swap(false, Ordering::SeqCst) => {
                log::info!("compressing {} stopped", path.display());
                QUEUE.lock().unwrap().retain(|(p, _)| *p != path);
                send_result(app, &path, None, None, true);
            }
            // Stopped for a game: it stays queued and goes on later.
            Err(err) if is_cancelled(&err) => return,
            Err(err) => {
                log::warn!("cannot compress {}: {err:#}", path.display());
                done(Some(format!("{err:#}")), None);
            }
        }
    }
}

/// What shrinking would do now.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ShrinkEstimate {
    pub files: usize,
    pub bytes: u64,
    /// About how big they would be afterwards.
    pub after_bytes: u64,
    /// A shrink is running right now.
    pub running: bool,
}

pub fn estimate(app: &AppHandle) -> ShrinkEstimate {
    let settings = app.state::<AppState>().controller.settings();
    let list = candidates(app, &settings);
    ShrinkEstimate {
        files: list.len(),
        bytes: list.iter().map(|(m, _)| m.size_bytes).sum(),
        after_bytes: list
            .iter()
            .map(|(m, rate)| (*rate as f64 / 8.0 * m.duration_secs) as u64)
            .sum(),
        running: RUNNING.load(Ordering::Relaxed),
    }
}

/// The video bitrate (bit/s) a recording of this size and frame rate is shrunk to.
pub fn target_bitrate(width: u32, height: u32, fps: f64, prefs: &ShrinkPrefs) -> i64 {
    let fps = fps_cap(fps, prefs).map_or(fps, f64::from);
    let rate =
        width as f64 * height as f64 * fps.clamp(24.0, 240.0) * prefs.quality.bits_per_pixel();
    (rate as i64).clamp(2_000_000, 80_000_000)
}

/// The frame rate a recording at `fps` is compressed to, when fewer than its own.
fn fps_cap(fps: f64, prefs: &ShrinkPrefs) -> Option<u32> {
    (prefs.max_fps > 0 && fps > prefs.max_fps as f64 + 0.5).then_some(prefs.max_fps)
}

/// Busy with something a shrink should not compete with.
fn busy(app: &AppHandle) -> bool {
    let controller = &app.state::<AppState>().controller;
    controller.detected_game().is_some()
        || controller.is_file_recording()
        || crate::import::is_running()
}

/// Files to shrink, oldest first, each with its target bitrate.
fn candidates(app: &AppHandle, settings: &Settings) -> Vec<(ClipMeta, i64)> {
    let prefs = &settings.storage.shrink;
    let folder = PathBuf::from(&settings.storage.folder);
    let favorites: Vec<String> = app
        .state::<AppState>()
        .albums
        .list()
        .into_iter()
        .filter(|a| a.id == umbra_core::albums::FAVORITES)
        .flat_map(|a| a.clip_paths)
        .collect();
    let cutoff = SystemTime::now() - Duration::from_secs(prefs.after_days as u64 * 86_400);
    let mut busy_paths: Vec<PathBuf> = umbra_core::processing::list()
        .into_iter()
        .map(|p| p.path)
        .collect();
    busy_paths.extend(skipped());
    let mut found: Vec<(ClipMeta, i64)> = library::scan(&folder)
        .into_iter()
        .filter(|m| !m.is_image && m.size_bytes >= MIN_BYTES)
        .filter(|m| prefs.include_clips || m.details.iter().any(|(k, _)| k == "recording"))
        .filter(|m| !favorites.iter().any(|f| Path::new(f) == m.path))
        .filter(|m| !busy_paths.contains(&m.path))
        .filter(|m| library::parse_iso8601(&m.created_at).is_some_and(|t| t <= cutoff))
        .filter_map(|m| {
            let rate = target_bitrate(m.width, m.height, m.fps, prefs);
            let video_rate = m.bit_rate as f64;
            (m.bit_rate == 0
                || fps_cap(m.fps, prefs).is_some()
                || video_rate > rate as f64 * SMALL_ENOUGH)
                .then_some((m, rate))
        })
        .filter(|(m, _)| !ten_bit(&m.path))
        .collect();
    found.sort_by(|a, b| a.0.created_at.cmp(&b.0.created_at));
    found
}

/// HDR10 recordings are 10-bit; encoding them again as 8-bit would lose that, so they stay.
fn ten_bit(path: &Path) -> bool {
    library::is_ten_bit(path)
}

/// Half-written files from a shrink cut short when Umbra closed. Only these, Umbra's own.
fn remove_leftovers(folder: &Path) {
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            let path = item.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
            if name.as_deref().is_some_and(|n| n.ends_with(".mp4.shrink")) {
                // A finished, checked smaller file whose original already went but which was
                // not put in its place (the move failed, or Umbra stopped in between): it is
                // the clip now. Deleting it lost both.
                let original = path.with_extension("");
                if !original.exists() && std::fs::rename(&path, &original).is_ok() {
                    log::info!("put the smaller {} in place", original.display());
                    continue;
                }
                let _ = std::fs::remove_file(&path);
            } else if name.is_some_and(|n| n.ends_with(".mp4.shrink.part")) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn run_pass(app: &AppHandle) {
    let settings = app.state::<AppState>().controller.settings();
    if busy(app) || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    remove_leftovers(Path::new(&settings.storage.folder));
    run_queue(app, &settings);
    if !settings.storage.shrink.enabled || busy(app) {
        RUNNING.store(false, Ordering::SeqCst);
        return;
    }
    let list = candidates(app, &settings);
    if !list.is_empty() {
        log::info!("shrinking {} old recordings", list.len());
    }
    for (meta, rate) in list {
        if busy(app)
            || !app
                .state::<AppState>()
                .controller
                .settings()
                .storage
                .shrink
                .enabled
        {
            break;
        }
        match shrink_one(app, &settings, &meta, rate, &settings.storage.shrink) {
            Ok(saved) => {
                log::info!(
                    "shrank {} by {:.1} GB",
                    meta.path.display(),
                    saved as f64 / 1e9
                );
                let _ = app.emit("clips-changed", ());
            }
            Err(err) if is_cancelled(&err) && STOP.swap(false, Ordering::SeqCst) => {
                log::info!("compressing {} stopped", meta.path.display());
                send_result(app, &meta.path, None, None, true);
            }
            Err(err) if is_cancelled(&err) => {
                log::info!(
                    "shrinking {} stopped; tried again later",
                    meta.path.display()
                );
                break;
            }
            Err(err) => log::warn!("cannot shrink {}: {err:#}", meta.path.display()),
        }
    }
    RUNNING.store(false, Ordering::SeqCst);
}

/// Shrinks one file in place; returns the bytes saved.
fn shrink_one(
    app: &AppHandle,
    settings: &Settings,
    meta: &ClipMeta,
    rate: i64,
    prefs: &ShrinkPrefs,
) -> Result<u64> {
    let path = &meta.path;
    STOP.store(false, Ordering::SeqCst);
    *CURRENT.lock().unwrap() = Some(path.clone());
    let result = shrink_file(app, settings, meta, rate, prefs);
    *CURRENT.lock().unwrap() = None;
    result
}

fn shrink_file(
    app: &AppHandle,
    settings: &Settings,
    meta: &ClipMeta,
    rate: i64,
    prefs: &ShrinkPrefs,
) -> Result<u64> {
    let path = &meta.path;
    let temp = path.with_extension("mp4.shrink");
    let _ = std::fs::remove_file(&temp);
    let mut job = ExportJob::new(path, &temp, 0.0, meta.duration_secs + 1.0);
    job.audio_tracks = (0..meta.audio_tracks.len()).collect();
    job.codec = Some(umbra_core::encode::VideoCodec::Hevc);
    job.video_bitrate = Some(rate);
    job.threads = settings.export.threads;
    job.max_fps = fps_cap(meta.fps, prefs);
    // In the background on files of an hour or more: the encoder's faster setting, about
    // twice as quick, and the bitrate still decides the size.
    job.quick = true;
    // Still the same clip: its camera stays a stream of its own, movable in the editor.
    job.keep_camera = true;
    job.keep_logo = true;

    let game = meta
        .details
        .iter()
        .find(|(k, _)| k == "game")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "Desktop".into());
    let task =
        umbra_core::processing::begin_task(path, &game, RecordingKind::Manual, 1000, "shrink");
    let _ = app.emit("processing-started", ());
    let watch = app.clone();
    let mut last_check = std::time::Instant::now();
    let result = umbra_core::export::export(&job, &mut |fraction| {
        task.progress((fraction.clamp(0.0, 1.0) * 1000.0) as u64, 1000);
        if STOP.load(Ordering::SeqCst) {
            return false;
        }
        // A game starting stops it; it is done again later.
        if last_check.elapsed() > Duration::from_secs(2) {
            last_check = std::time::Instant::now();
            return !busy(&watch);
        }
        true
    });
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }

    // The new file must hold all of it before the original goes.
    let check = (|| -> Result<u64> {
        let new = library::probe(&temp).context("cannot read the smaller file")?;
        let tolerance = 1.0f64.max(meta.duration_secs * 0.01);
        if (new.duration_secs - meta.duration_secs).abs() > tolerance {
            bail!(
                "the smaller file is {:.1} s long, the original {:.1} s",
                new.duration_secs,
                meta.duration_secs
            );
        }
        if new.audio_tracks.len() != meta.audio_tracks.len()
            || (new.width, new.height) != (meta.width, meta.height)
        {
            bail!("the smaller file's tracks or picture size differ");
        }
        if new.size_bytes >= meta.size_bytes {
            bail!("it would not be smaller");
        }
        Ok(meta.size_bytes - new.size_bytes)
    })();
    let saved = match check {
        Ok(saved) => saved,
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            return Err(err);
        }
    };
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Err(err) = crate::clips::remove(app, std::slice::from_ref(path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(err).context("cannot remove the original (is it open?)");
    }
    std::fs::rename(&temp, path).context("cannot put the smaller file in place")?;
    // Same date as before, so sorting and the storage limits see it as the same age.
    if let Some(modified) = modified {
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
            let _ = file.set_modified(modified);
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::remove_leftovers;

    #[test]
    fn a_checked_smaller_file_whose_original_went_is_kept() {
        let dir = std::env::temp_dir().join(format!("umbra-shrink-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let put = |name: &str| std::fs::write(dir.join(name), name).unwrap();
        // The original went but the smaller file was not put in its place.
        put("a.mp4.shrink");
        // A shrink cut short: the original is still there.
        put("b.mp4");
        put("b.mp4.shrink");
        put("c.mp4.shrink.part");
        remove_leftovers(&dir);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["a.mp4", "b.mp4"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("a.mp4")).unwrap(),
            "a.mp4.shrink"
        );
        assert_eq!(std::fs::read_to_string(dir.join("b.mp4")).unwrap(), "b.mp4");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
