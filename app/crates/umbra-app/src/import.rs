//! Bringing clips from other recorders (NVIDIA, Medal, AMD, Xbox Game Bar and others) or any
//! folder into the library. Each file is copied into the clip folder under its game's name as
//! an MP4 (MKV, MOV and WebM are rewrapped without re-encoding where the codecs allow, and
//! re-encoded where not); the originals stay where they are.
//!
//! Moving instead of copying removes each original only after its copy has been checked: a
//! rewrapped copy must hold exactly as many video and audio packets as the original, a
//! re-encoded one must be as long and readable to the end. Originals go to the Recycle Bin
//! unless the storage settings say to skip it. Files are imported on all processor threads.
//!
//! An imported file is dated the day it was imported, so storage limits count from then and
//! never delete old clips the moment they arrive; when it was recorded goes in its
//! `umbra_recorded` tag, which the library sorts by.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use umbra_core::library;
use umbra_core::naming::sanitize_segment;
use umbra_core::render::{OutputFormat, RenderJob, RenderPart};

use crate::clips::clip_folder;

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm"];

/// Folders under Videos where recorders save, with the name shown for them.
const KNOWN: &[(&str, &str)] = &[
    ("NVIDIA", "NVIDIA"),
    ("Medal", "Medal"),
    ("AMD ReLive", "Radeon ReLive"),
    ("SteelSeries Moments", "SteelSeries Moments"),
    ("Xbox Game Bar", "Captures"),
    ("Discord clips", "Discord Clips"),
    ("Outplayed", "Overwolf\\Outplayed"),
    ("Steam", "Steam"),
];

/// Folder names that say where clips are kept, not which game they are of.
const GENERIC: &[&str] = &[
    "clips",
    "exported clips",
    "shared clips",
    "captures",
    "videos",
    "medal",
    "nvidia",
    "radeon relive",
    "steelseries moments",
    "discord clips",
    "outplayed",
    "replays",
    "highlights",
    "recordings",
];

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    pub label: String,
    pub path: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportFile {
    pub path: String,
    pub name: String,
    pub bytes: u64,
    pub game: String,
    /// The same name is already in the library under that game; it is left out.
    pub duplicate: bool,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequestFile {
    pub path: String,
    pub game: String,
}

/// A file being imported right now: what is happening to it and how far it is.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ActiveFile {
    name: String,
    bytes: u64,
    /// `copying`, `converting`, `checking` or `removing`.
    stage: &'static str,
    /// Of this stage, 0 to 1.
    fraction: f32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ImportProgress {
    done: usize,
    total: usize,
    /// The last file that finished.
    current: String,
    /// Work done and to do, in bytes read (copying, and checking when moving).
    bytes_done: u64,
    bytes_total: u64,
    /// Bytes per second over the last few seconds.
    rate: f64,
    /// Seconds left at that rate, once it is known.
    eta_secs: Option<f64>,
    active: Vec<ActiveFile>,
    /// Originals are removed once their copy checks out.
    moving: bool,
}

/// Progress of every file, shared by the workers and the reporter.
struct Tracker {
    /// Per file being worked on: what is shown, and the bytes of work done on it.
    active: Mutex<BTreeMap<usize, (ActiveFile, u64)>>,
    /// Bytes of work done on files that are finished.
    finished: AtomicU64,
    total: u64,
}

impl Tracker {
    fn update(
        &self,
        index: usize,
        name: &str,
        bytes: u64,
        stage: &'static str,
        fraction: f32,
        work: u64,
    ) {
        let file = ActiveFile {
            name: name.to_string(),
            bytes,
            stage,
            fraction: fraction.clamp(0.0, 1.0),
        };
        self.active.lock().unwrap().insert(index, (file, work));
    }

    fn finish(&self, index: usize, work: u64) {
        self.active.lock().unwrap().remove(&index);
        self.finished.fetch_add(work, Ordering::SeqCst);
    }

    fn bytes_done(&self) -> u64 {
        let active: u64 = self.active.lock().unwrap().values().map(|(_, w)| *w).sum();
        (self.finished.load(Ordering::SeqCst) + active).min(self.total)
    }
}

/// Bytes being written by imports right now, so two big files do not both count on the
/// same free space.
static RESERVED: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ImportDone {
    imported: usize,
    skipped: usize,
    /// Originals removed after their copy was checked (moving).
    removed: usize,
    failed: Vec<String>,
}

/// What became of one file.
enum Outcome {
    /// In the library at `target`. `counts` are the original's video and audio packets when
    /// they were copied (none when it was re-encoded); `meta` the original's details.
    Imported {
        target: PathBuf,
        counts: Option<(usize, usize)>,
        meta: library::ClipMeta,
    },
    /// A file of that name was already there; nothing was done.
    AlreadyThere,
}

static IMPORTING: AtomicBool = AtomicBool::new(false);

/// An import is running.
pub fn is_running() -> bool {
    IMPORTING.load(Ordering::SeqCst)
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

/// Every video under `root`, skipping hidden and cache folders and the clip folder itself.
fn videos_under(root: &Path, skip: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if dir.starts_with(skip) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !name.starts_with('.') && name != "thumbs" && !name.contains("cache") {
                    stack.push(path);
                }
            } else if kind.is_file() && is_video(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn videos_folder() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos"))
}

/// The other recorders' folders on this PC that hold videos.
pub fn sources(app: &AppHandle) -> Vec<ImportSource> {
    let Some(videos) = videos_folder() else {
        return Vec::new();
    };
    let ours = clip_folder(app);
    KNOWN
        .iter()
        .filter_map(|(label, folder)| {
            let path = videos.join(folder);
            if !path.is_dir() || ours.starts_with(&path) && ours == path {
                return None;
            }
            let files = videos_under(&path, &ours);
            if files.is_empty() {
                return None;
            }
            let bytes = files
                .iter()
                .filter_map(|f| f.metadata().ok())
                .map(|m| m.len())
                .sum();
            Some(ImportSource {
                label: label.to_string(),
                path: path.to_string_lossy().into_owned(),
                files: files.len(),
                bytes,
            })
        })
        .collect()
}

/// The game a clip is of: its folder's name, unless that only says where clips are kept,
/// else the start of its file name before the date (`Battlefield V 2025.12.17 - 23.27.58`).
pub fn guess_game(file: &Path, root: &Path) -> String {
    if let Some(parent) = file.parent() {
        if parent != root {
            if let Some(name) = parent.file_name().map(|n| n.to_string_lossy().into_owned()) {
                if !GENERIC.contains(&name.to_lowercase().as_str()) && !name.trim().is_empty() {
                    return name;
                }
            }
        }
    }
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The first year-like number (19xx or 20xx) after a separator ends the game's name.
    let bytes = stem.as_bytes();
    let mut cut = None;
    for i in 1..bytes.len().saturating_sub(3) {
        let separator = matches!(bytes[i - 1], b' ' | b'_' | b'-' | b'.');
        let year = bytes[i..i + 4].iter().all(u8::is_ascii_digit)
            && (bytes[i] == b'1' || bytes[i] == b'2');
        if separator && year {
            cut = Some(i);
            break;
        }
    }
    let name = cut
        .map_or("", |i| &stem[..i])
        .trim_matches([' ', '_', '-', '.'])
        .replace('_', " ");
    if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
        "Imported".to_string()
    } else {
        name
    }
}

fn destination(app: &AppHandle, source: &Path, game: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".into());
    let game = sanitize_segment(game);
    let game = if game.is_empty() {
        "Imported".to_string()
    } else {
        game
    };
    clip_folder(app)
        .join(game)
        .join(format!("{}.mp4", sanitize_segment(&stem)))
}

/// What importing these files or folders would bring in.
pub fn scan(app: &AppHandle, paths: &[String]) -> Vec<ImportFile> {
    let ours = clip_folder(app);
    let mut files = Vec::new();
    for raw in paths {
        let path = PathBuf::from(raw);
        let (root, found) = if path.is_dir() {
            (path.clone(), videos_under(&path, &ours))
        } else if path.is_file() && is_video(&path) && !path.starts_with(&ours) {
            (
                path.parent().map(Path::to_path_buf).unwrap_or_default(),
                vec![path.clone()],
            )
        } else {
            continue;
        };
        for file in found {
            if files
                .iter()
                .any(|f: &ImportFile| f.path == file.to_string_lossy())
            {
                continue;
            }
            let game = guess_game(&file, &root);
            let duplicate = destination(app, &file, &game).exists();
            files.push(ImportFile {
                path: file.to_string_lossy().into_owned(),
                name: file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes: file.metadata().map_or(0, |m| m.len()),
                game,
                duplicate,
            });
        }
    }
    files
}

/// Imports one file. `report` gets the stage and how far it is (0 to 1).
fn import_one(
    app: &AppHandle,
    file: &ImportRequestFile,
    report: &dyn Fn(&'static str, f32),
) -> Result<Outcome> {
    let source = PathBuf::from(&file.path);
    if !source.is_file() {
        bail!("it is gone");
    }
    let target = destination(app, &source, &file.game);
    if target.exists() {
        return Ok(Outcome::AlreadyThere);
    }
    let folder = target.parent().context("no folder")?.to_path_buf();
    std::fs::create_dir_all(&folder)?;
    let size = source.metadata().map_or(0, |m| m.len());

    // Room for it, keeping the free space Storage asks for, and counting the other files
    // being imported at the same time.
    let keep_gb = app
        .state::<crate::AppState>()
        .controller
        .settings()
        .storage
        .min_free_gb as u64;
    let keep = keep_gb * (1u64 << 30);
    let others = RESERVED.fetch_add(size, Ordering::SeqCst);
    struct Reserved(u64);
    impl Drop for Reserved {
        fn drop(&mut self) {
            RESERVED.fetch_sub(self.0, Ordering::SeqCst);
        }
    }
    let _reserved = Reserved(size);
    if let Some(free) = umbra_core::disk::free_bytes(&folder) {
        if free < size + others + keep {
            let gb = |b: u64| b as f64 / 1e9;
            bail!(
                "not enough room on the clip drive: {:.1} GB free, it needs {:.1} GB{}",
                gb(free),
                gb(size + others),
                if keep > 0 {
                    format!(" plus the {keep_gb} GB Storage keeps free")
                } else {
                    String::new()
                }
            );
        }
    }

    // When it was recorded, for sorting: the file's own creation time, or when it was
    // written less its length.
    let meta = library::probe(&source).context("it is not a video Umbra can read")?;
    let tags = vec![
        ("recorded".to_string(), meta.created_at.clone()),
        ("game".to_string(), file.game.clone()),
        ("imported_from".to_string(), file.path.clone()),
    ];
    let partial = target.with_extension("mp4.part");
    report("copying", 0.0);
    let remuxed = umbra_core::recorder::remux_to_mp4(&source, &partial, &tags, &mut |read| {
        report("copying", read as f32 / size.max(1) as f32)
    });
    let counts = match remuxed {
        Ok(counts) => Some(counts),
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            log::info!("re-encoding {} to import it: {err:#}", source.display());
            // A codec MP4 cannot hold: re-encode it, picture and sound.
            let job = RenderJob {
                parts: vec![RenderPart {
                    path: source.clone(),
                    speeds: Vec::new(),
                }],
                output: partial.clone(),
                format: OutputFormat::Mp4,
                codec: None,
                size: None,
                fps: None,
                video_bitrate: None,
                audio_kbps: 192,
                gif_width: 480,
                fades: Default::default(),
                threads: 0,
            };
            umbra_core::render::render(&job, &mut |f| {
                report("converting", f);
                true
            })
            .map_err(|e| anyhow!("cannot convert it: {e:#}"))?;
            None
        }
    };
    std::fs::rename(&partial, &target).context("cannot put it in the clip folder")?;
    Ok(Outcome::Imported {
        target,
        counts,
        meta,
    })
}

/// Whether the copy holds everything the original did, before the original goes. `counts`
/// are the original's video and audio packets, counted while it was copied (none when it was
/// re-encoded). Only the copy is read again.
fn verify(
    original: &library::ClipMeta,
    copy: &Path,
    counts: Option<(usize, usize)>,
    report: &mut dyn FnMut(f32),
) -> Result<()> {
    let imported = library::probe(copy).context("cannot read the copy")?;
    let tolerance = 0.5f64.max(original.duration_secs * 0.01);
    if (original.duration_secs - imported.duration_secs).abs() > tolerance {
        bail!(
            "the copy is {:.1} s long, the original {:.1} s",
            imported.duration_secs,
            original.duration_secs
        );
    }
    if (imported.width, imported.height) != (original.width, original.height) {
        bail!("the copy's picture size differs from the original's");
    }
    let size = imported.size_bytes.max(1);
    let (copy_video, copy_audio) =
        library::packet_counts(copy, &mut |read| report(read as f32 / size as f32))
            .context("the copy cannot be read to the end")?;
    match counts {
        Some((video, audio)) if (copy_video, copy_audio) != (video, audio) => bail!(
            "the copy has {copy_video} video and {copy_audio} audio packets, the original {video} and {audio}"
        ),
        None if copy_video == 0 => bail!("the copy has no picture"),
        _ => Ok(()),
    }
}

/// Starts importing in the background on all processor threads: `import-progress` events
/// twice a second, then `import-done`. With `move_originals`, each original is removed once
/// its copy checks out.
pub fn start(app: &AppHandle, files: Vec<ImportRequestFile>, move_originals: bool) -> Result<()> {
    if IMPORTING.swap(true, Ordering::SeqCst) {
        bail!("an import is already running");
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("umbra-import".into())
        .spawn(move || {
            let total = files.len();
            let sizes: Vec<u64> = files
                .iter()
                .map(|f| std::fs::metadata(&f.path).map_or(0, |m| m.len()))
                .collect();
            // Copying reads a file once; checking reads its copy once more.
            let passes = if move_originals { 2 } else { 1 };
            let tracker = Tracker {
                active: Mutex::new(BTreeMap::new()),
                finished: AtomicU64::new(0),
                total: sizes.iter().sum::<u64>() * passes,
            };
            let next = AtomicUsize::new(0);
            let finished = AtomicUsize::new(0);
            let imported = AtomicUsize::new(0);
            let skipped = AtomicUsize::new(0);
            let removed = AtomicUsize::new(0);
            let failed = Mutex::new(Vec::new());
            let last = Mutex::new(String::new());
            let all_done = AtomicBool::new(false);
            // The Recycle Bin one file at a time.
            let removing = Mutex::new(());
            let workers = std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(total.max(1));
            log::info!(
                "importing {total} files on {workers} threads{}",
                if move_originals { ", moving" } else { "" }
            );
            std::thread::scope(|scope| {
                // Reports progress twice a second, with the speed of the last few seconds.
                scope.spawn(|| {
                    let mut samples: Vec<(Instant, u64)> = Vec::new();
                    while !all_done.load(Ordering::SeqCst) {
                        let now = Instant::now();
                        let done_bytes = tracker.bytes_done();
                        samples.push((now, done_bytes));
                        samples.retain(|(at, _)| now.duration_since(*at) <= Duration::from_secs(8));
                        let (t0, b0) = samples[0];
                        let secs = now.duration_since(t0).as_secs_f64();
                        let rate = if secs > 0.5 { done_bytes.saturating_sub(b0) as f64 / secs } else { 0.0 };
                        let left = tracker.total.saturating_sub(done_bytes) as f64;
                        let _ = app.emit(
                            "import-progress",
                            ImportProgress {
                                done: finished.load(Ordering::SeqCst),
                                total,
                                current: last.lock().unwrap().clone(),
                                bytes_done: done_bytes,
                                bytes_total: tracker.total,
                                rate,
                                eta_secs: (rate > 1_000_000.0).then(|| left / rate),
                                active: tracker.active.lock().unwrap().values().map(|(f, _)| f.clone()).collect(),
                                moving: move_originals,
                            },
                        );
                        std::thread::sleep(Duration::from_millis(500));
                    }
                });
                let work = scope.spawn(|| {
                    std::thread::scope(|inner| {
                        for _ in 0..workers {
                            inner.spawn(|| {
                                loop {
                                    let i = next.fetch_add(1, Ordering::SeqCst);
                                    let Some(file) = files.get(i) else { break };
                                    let size = sizes[i];
                                    let source = PathBuf::from(&file.path);
                                    let name = source
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_default();
                                    let report = |stage: &'static str, fraction: f32| {
                                        // Copying or converting is the first pass, checking the second;
                                        // removing comes after both.
                                        let (pass, fraction_done) = match stage {
                                            "checking" => (1.0, fraction.clamp(0.0, 1.0) as f64),
                                            "removing" => (passes as f64, 0.0),
                                            _ => (0.0, fraction.clamp(0.0, 1.0) as f64),
                                        };
                                        let work = ((pass + fraction_done) * size as f64) as u64;
                                        tracker.update(i, &name, size, stage, fraction, work);
                                    };
                                    // One file going wrong (even a crash in a decoder's wrapper)
                                    // must not stop the others.
                                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        import_one(&app, file, &report).and_then(|outcome| match outcome {
                                            Outcome::AlreadyThere => {
                                                skipped.fetch_add(1, Ordering::SeqCst);
                                                Ok(())
                                            }
                                            Outcome::Imported { target, counts, meta } => {
                                                imported.fetch_add(1, Ordering::SeqCst);
                                                if move_originals {
                                                    verify(&meta, &target, counts, &mut |f| report("checking", f))
                                                        .map_err(|e| anyhow!("copied, but the original was kept: {e:#}"))?;
                                                    report("removing", 0.0);
                                                    let _guard = removing.lock().unwrap();
                                                    crate::clips::remove(&app, std::slice::from_ref(&source)).map_err(|e| {
                                                        anyhow!("copied, but the original could not be removed: {e:#}")
                                                    })?;
                                                    removed.fetch_add(1, Ordering::SeqCst);
                                                }
                                                Ok(())
                                            }
                                        })
                                    }))
                                    .unwrap_or_else(|_| Err(anyhow!("Umbra hit an error reading it; it was left as it was")));
                                    if let Err(err) = result {
                                        log::warn!("import of {}: {err:#}", file.path);
                                        failed.lock().unwrap().push(format!("{name}: {err:#}"));
                                    }
                                    tracker.finish(i, size * passes);
                                    *last.lock().unwrap() = name;
                                    let done = finished.fetch_add(1, Ordering::SeqCst) + 1;
                                    if done % 5 == 0 {
                                        let _ = app.emit("clips-changed", ());
                                    }
                                }
                            });
                        }
                    });
                    all_done.store(true, Ordering::SeqCst);
                });
                let _ = work.join();
                all_done.store(true, Ordering::SeqCst);
            });
            let _ = app.emit("clips-changed", ());
            let _ = app.emit(
                "import-done",
                ImportDone {
                    imported: imported.load(Ordering::SeqCst),
                    skipped: skipped.load(Ordering::SeqCst),
                    removed: removed.load(Ordering::SeqCst),
                    failed: failed.into_inner().unwrap_or_default(),
                },
            );
            IMPORTING.store(false, Ordering::SeqCst);
        })
        .context("cannot start the import")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_come_from_folders_and_names() {
        let root = Path::new(r"C:\Users\a\Videos\NVIDIA");
        assert_eq!(
            guess_game(
                &root
                    .join("Battlefield V")
                    .join("Battlefield V 2025.12.17 - 23.27.58.01.mp4"),
                root
            ),
            "Battlefield V"
        );
        // A folder that only says "clips" does not name the game; the file name does.
        let medal = Path::new(r"C:\Users\a\Videos\Medal");
        assert_eq!(
            guess_game(
                &medal.join("Clips").join("Valorant_2024-01-15_20-30-45.mp4"),
                medal
            ),
            "Valorant"
        );
        assert_eq!(
            guess_game(
                &root.join("World of Warships 2025.10.09 - 18.37.36.01.mp4"),
                root
            ),
            "World of Warships"
        );
        assert_eq!(
            guess_game(&root.join("20240115_203045.mp4"), root),
            "Imported"
        );
        assert_eq!(guess_game(&root.join("my clip.mp4"), root), "Imported");
    }
}
