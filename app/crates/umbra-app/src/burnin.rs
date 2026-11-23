//! Burn in, from the library (right-click a clip): its camera, keyboard and mouse or logo
//! drawn into the picture for good (see `umbra_core::burnin`), one clip after another on a
//! thread of its own. The new file goes next to the original as `<name> (burned).mp4`, or,
//! when Settings say so, takes the original's place once it checks out; the original then
//! goes like a deleted clip (to the Recycle Bin unless Storage says otherwise).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use umbra_core::burnin::{self, Layers};
use umbra_core::export::is_cancelled;
use umbra_core::inputfile::sidecar;
use umbra_core::library;
use umbra_core::recordings::RecordingKind;

use crate::AppState;

/// Clips waiting, with the layers to burn in; and whether the worker runs. One lock, so a
/// clip queued just as the worker finds the queue empty is not left behind.
static QUEUE: Mutex<(Vec<(PathBuf, Layers)>, bool)> = Mutex::new((Vec::new(), false));
/// The clip being burned in now, and where it goes.
static CURRENT: Mutex<Option<(PathBuf, PathBuf)>> = Mutex::new(None);
/// The user asked to stop the current one.
static STOP: AtomicBool = AtomicBool::new(false);

/// What happened to a clip, for a message in the window.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct BurnResult {
    path: String,
    file_name: String,
    /// The new file, when it worked: beside the original, or the original's own path when it
    /// took its place.
    output: Option<String>,
    problem: Option<String>,
    stopped: bool,
}

/// Queues clips to burn `which` into; returns how many were queued (not those already in
/// line for the same).
pub fn request(app: &AppHandle, paths: &[String], which: Layers) -> Result<usize> {
    let mut queue = QUEUE.lock().unwrap();
    let mut queued = 0;
    for p in paths {
        let path = crate::clips::inside_clip_folder(app, p)?;
        if library::is_image_path(&path) || queue.0.contains(&(path.clone(), which)) {
            continue;
        }
        queue.0.push((path, which));
        queued += 1;
    }
    if queued > 0 && !queue.1 {
        queue.1 = true;
        let app = app.clone();
        let started = std::thread::Builder::new()
            .name("umbra-burn-in".into())
            .spawn(move || work(&app));
        if started.is_err() {
            queue.1 = false;
            queue.0.clear();
            anyhow::bail!("cannot start burning in");
        }
    }
    Ok(queued)
}

/// Stops these clips: the one being burned in (by its own path or the new file's) and any
/// waiting. Returns how many were stopped.
pub fn stop(app: &AppHandle, paths: &[String]) -> Result<usize> {
    let mut stopped = 0;
    for p in paths {
        let path = crate::clips::inside_clip_folder(app, p)?;
        let mut queue = QUEUE.lock().unwrap();
        let before = queue.0.len();
        queue.0.retain(|(q, _)| *q != path);
        let removed = before - queue.0.len();
        drop(queue);
        let current = CURRENT
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(from, to)| *from == path || *to == path);
        if current {
            STOP.store(true, Ordering::SeqCst);
        }
        if removed > 0 {
            send(app, &path, None, None, true);
        }
        stopped += removed + usize::from(current);
    }
    Ok(stopped)
}

fn send(
    app: &AppHandle,
    path: &Path,
    output: Option<&Path>,
    problem: Option<String>,
    stopped: bool,
) {
    let _ = app.emit(
        "burn-result",
        BurnResult {
            path: path.to_string_lossy().into_owned(),
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            output: output.map(|o| o.to_string_lossy().into_owned()),
            problem,
            stopped,
        },
    );
}

fn work(app: &AppHandle) {
    loop {
        let (path, which) = {
            let mut queue = QUEUE.lock().unwrap();
            if queue.0.is_empty() {
                queue.1 = false;
                return;
            }
            queue.0.remove(0)
        };
        STOP.store(false, Ordering::SeqCst);
        let result = burn_one(app, &path, which);
        *CURRENT.lock().unwrap() = None;
        match result {
            Ok(output) => {
                log::info!("burned {which:?} into {}", path.display());
                let _ = app.emit("clips-changed", ());
                send(app, &path, Some(&output), None, false);
            }
            Err(err) if is_cancelled(&err) => {
                log::info!("burning in {} stopped", path.display());
                send(app, &path, None, None, true);
            }
            Err(err) => {
                log::warn!("cannot burn into {}: {err:#}", path.display());
                send(app, &path, None, Some(format!("{err:#}")), false);
            }
        }
    }
}

/// Burns one clip; returns where the new file is.
fn burn_one(app: &AppHandle, path: &Path, which: Layers) -> Result<PathBuf> {
    let controller = &app.state::<AppState>().controller;
    let settings = controller.settings();
    let meta = library::probe(path).context("it cannot be read")?;
    let output = burnin::output_for(path);
    *CURRENT.lock().unwrap() = Some((path.to_path_buf(), output.clone()));
    let root = PathBuf::from(&settings.storage.folder);
    let game = library::game_from_path(&root, path).unwrap_or_else(|| "Desktop".into());
    let task =
        umbra_core::processing::begin_task(&output, &game, RecordingKind::Manual, 1000, "burn");
    let _ = app.emit("processing-started", ());
    burnin::burn(&meta, which, &settings, &output, &mut |fraction| {
        task.progress((fraction.clamp(0.0, 1.0) * 1000.0) as u64, 1000);
        !STOP.load(Ordering::SeqCst)
    })?;
    if !settings.export.burn_replace_original {
        return Ok(output);
    }

    // In the original's place, so albums and bookmarks keep pointing at it; only once the
    // new file holds all of it.
    let new_input = sidecar(&output);
    let give_up = |err: anyhow::Error| {
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_file(&new_input);
        err
    };
    burnin::check(&meta, which, &output).map_err(give_up)?;
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    // The original's input file goes with it; the new file's (when the input stays a layer,
    // a copy) takes its place.
    crate::clips::remove(
        app,
        &umbra_core::inputfile::with_sidecars(&[path.to_path_buf()]),
    )
    .map_err(|err| give_up(err.context("cannot remove the original (is it open?)")))?;
    std::fs::rename(&output, path).context("cannot put the new file in place")?;
    if new_input.exists() {
        std::fs::rename(&new_input, sidecar(path))
            .context("cannot put the keyboard and mouse input in place")?;
    }
    // Same date as before, so it sorts where it did.
    if let Some(modified) = modified
        && let Ok(file) = std::fs::OpenOptions::new().write(true).open(path)
    {
        let _ = file.set_modified(modified);
    }
    Ok(path.to_path_buf())
}
