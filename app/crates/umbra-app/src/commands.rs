//! Tauri commands: the backend half of the `Api` interface in `app/ui/src/lib/api/types.ts`.
//! Errors go to the UI as plain strings.
//!
//! Anything that can take more than a moment is an `async` command that does its work on a
//! blocking thread. Plain commands run on the window's own thread and would freeze the UI.

use tauri::{AppHandle, Manager};
use umbra_core::capture::list_monitors as monitors;
use umbra_core::d3d::Gpu;
use umbra_core::devices;
use umbra_core::encode::{RateControl, VideoCodec, VideoEncoder, VideoSettings};
use umbra_core::settings::Settings;
use umbra_core::wnd;

use umbra_core::albums::Album;
use umbra_core::controller::resolve_target;
use umbra_core::settings::CaptureChoice;

use crate::AppState;
use crate::clips::{self, ClipDto, ExportRequest, TrackAudioDto};
use crate::dto::{
    AudioAppDto, CaptureSourceDto, MicrophoneDto, MonitorDto, RunningGameDto, StatusDto,
    SystemInfoDto, UsageEstimateDto, WindowDto,
};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("the task stopped unexpectedly: {e}"))
}

/// Lets the UI load thumbnails and play clips from the clip folder, and nothing else.
pub fn allow_clip_folders(app: &AppHandle, settings: &Settings) {
    let folder = std::path::PathBuf::from(&settings.storage.folder);
    let _ = std::fs::create_dir_all(&folder);
    let scope = app.asset_protocol_scope();
    if let Err(e) = scope.allow_directory(&folder, true) {
        log::warn!("cannot allow clip folder: {e}");
    }
    for cache in [
        clips::thumbs_dir(),
        clips::tracks_dir(),
        umbra_core::settings::logos_dir(),
    ] {
        let _ = std::fs::create_dir_all(&cache);
        if let Err(e) = scope.allow_directory(&cache, true) {
            log::warn!("cannot allow {}: {e}", cache.display());
        }
    }
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    app.state::<AppState>().controller.settings()
}

#[tauri::command]
pub async fn set_settings(app: AppHandle, settings: Settings) -> CmdResult<Settings> {
    blocking(move || {
        let kept = app
            .state::<AppState>()
            .controller
            .set_settings(settings)
            .map_err(err)?;
        allow_clip_folders(&app, &kept);
        // Tighter storage limits take effect right away.
        clips::enforce_limits_later(&app);
        // Shrinking turned on (or changed) looks for old recordings now.
        if kept.storage.shrink.enabled {
            crate::shrink::wake();
        }
        Ok(kept)
    })
    .await?
}

#[tauri::command]
pub fn get_status(app: AppHandle) -> StatusDto {
    app.state::<AppState>().controller.status().into()
}

#[tauri::command]
pub async fn get_system_info(app: AppHandle) -> CmdResult<SystemInfoDto> {
    blocking(move || {
        app.state::<AppState>()
            .system_info
            .get_or_init(|| {
                let mut gpu_name = "Unknown GPU".to_string();
                let mut codecs = Vec::new();
                if let Ok(gpu) = Gpu::new() {
                    gpu_name = gpu.adapter_name.clone();
                    // A codec is offered only if the hardware encoder really opens.
                    for (name, codec) in [
                        ("h264", VideoCodec::H264),
                        ("hevc", VideoCodec::Hevc),
                        ("av1", VideoCodec::Av1),
                    ] {
                        let probe = VideoSettings {
                            hdr10: false,
                            codec,
                            width: 1280,
                            height: 720,
                            fps: 30,
                            rate: RateControl::Cbr { kbps: 5000 },
                            preset: 1,
                            keyframe_interval_secs: 1,
                            tuning: Default::default(),
                        };
                        if VideoEncoder::new(&gpu, probe).is_ok() {
                            codecs.push(name);
                        }
                    }
                }
                SystemInfoDto {
                    app_version: env!("CARGO_PKG_VERSION").to_string(),
                    gpu: gpu_name,
                    codecs,
                    settings_folder: umbra_core::settings::data_dir()
                        .to_string_lossy()
                        .into_owned(),
                }
            })
            .clone()
    })
    .await
}

#[tauri::command]
pub fn start_recording(app: AppHandle) {
    app.state::<AppState>().controller.start_recording();
}

#[tauri::command]
pub fn stop_recording(app: AppHandle) {
    app.state::<AppState>().controller.stop_recording();
}

#[tauri::command]
pub fn list_monitors() -> Vec<MonitorDto> {
    monitors()
        .into_iter()
        .map(|m| MonitorDto {
            device: m.device,
            width: m.width,
            height: m.height,
            refresh_hz: m.refresh_hz,
            primary: m.primary,
            native_width: m.native_width,
            native_height: m.native_height,
        })
        .collect()
}

#[tauri::command]
pub async fn list_windows() -> CmdResult<Vec<WindowDto>> {
    blocking(|| {
        wnd::list_windows()
            .into_iter()
            .map(|w| WindowDto {
                exe: w.exe,
                title: w.title,
            })
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn list_audio_apps() -> CmdResult<Vec<AudioAppDto>> {
    blocking(|| {
        devices::list_audio_apps().map_err(err).map(|apps| {
            apps.into_iter()
                .map(|a| AudioAppDto {
                    exe: a.exe,
                    name: a.name,
                })
                .collect()
        })
    })
    .await?
}

#[tauri::command]
pub async fn list_microphones() -> CmdResult<Vec<MicrophoneDto>> {
    blocking(|| {
        devices::list_microphones().map_err(err).map(|mics| {
            mics.into_iter()
                .map(|m| MicrophoneDto {
                    id: m.id,
                    name: m.name,
                    is_default: m.is_default,
                })
                .collect()
        })
    })
    .await?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDto {
    id: String,
    name: String,
    /// The sizes it films at, smallest first.
    sizes: Vec<clips::SizeDto>,
    /// `Media Foundation` or `DirectShow`.
    api: &'static str,
}

#[tauri::command]
pub async fn list_cameras() -> CmdResult<Vec<CameraDto>> {
    blocking(|| {
        umbra_core::camera::list_cameras()
            .into_iter()
            .map(|c| CameraDto {
                id: c.id,
                name: c.name,
                sizes: c
                    .sizes
                    .into_iter()
                    .map(|(width, height)| clips::SizeDto { width, height })
                    .collect(),
                api: c.api,
            })
            .collect()
    })
    .await
}

/// The system file picker for a logo. The image is copied into Umbra's folder, so it stays
/// when the original moves, and its copy's path comes back; `None` if the user cancels.
#[tauri::command]
pub async fn pick_logo(app: AppHandle) -> CmdResult<Option<String>> {
    let Some(chosen) = pick(app, |dialog| {
        dialog
            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
            .set_title("Choose a logo")
            .pick_file()
    })
    .await?
    else {
        return Ok(None);
    };
    blocking(move || {
        let chosen = std::path::PathBuf::from(chosen);
        let extension = chosen
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let dir = umbra_core::settings::logos_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let copy = dir.join(format!("logo-{millis}.{extension}"));
        std::fs::copy(&chosen, &copy).map_err(|e| format!("cannot copy the image: {e}"))?;
        Ok(Some(copy.to_string_lossy().into_owned()))
    })
    .await?
}

/// A clip's camera as a file the player can show over the clip; `None` without one.
#[tauri::command]
pub async fn get_camera_video(
    app: AppHandle,
    path: String,
) -> CmdResult<Option<clips::CameraVideoDto>> {
    blocking(move || clips::camera_video(&app, &path).map_err(err)).await?
}

/// The system folder picker, in front of Umbra's window. `None` if the user cancels.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> CmdResult<Option<String>> {
    pick(app, |dialog| dialog.pick_folder()).await
}

/// The system file picker for a program (`.exe`), in front of Umbra's window. `None` if the
/// user cancels.
#[tauri::command]
pub async fn pick_program(app: AppHandle) -> CmdResult<Option<String>> {
    pick(app, |dialog| {
        dialog
            .add_filter("Programs", &["exe"])
            .set_title("Choose a program")
            .pick_file()
    })
    .await
}

/// The system file picker for videos to import, several at once. Empty if cancelled.
#[tauri::command]
pub async fn pick_videos(app: AppHandle) -> CmdResult<Vec<String>> {
    let picker = std::thread::spawn(move || {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Videos", &["mp4", "m4v", "mov", "mkv", "webm"])
            .set_title("Choose clips to import");
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        dialog
            .pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    });
    blocking(move || picker.join())
        .await?
        .map_err(|_| "the picker stopped unexpectedly".to_string())
}

/// A file Umbra is still working on, such as a long recording being made quick to open.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingDto {
    /// `finish` or `shrink`.
    task: &'static str,
    /// Where the file ends up.
    path: String,
    file_name: String,
    game: String,
    kind: umbra_core::recordings::RecordingKind,
    bytes_done: u64,
    bytes_total: u64,
    /// When the work started.
    started_at: String,
    /// Seconds left at the speed so far, once there is one.
    eta_secs: Option<f64>,
}

/// How much shrinking old recordings would save now (Settings, Storage).
#[tauri::command]
pub async fn shrink_estimate(app: AppHandle) -> CmdResult<crate::shrink::ShrinkEstimate> {
    blocking(move || crate::shrink::estimate(&app)).await
}

/// Compresses these clips now (right-click or the selection), at `quality` or the setting's.
#[tauri::command]
pub async fn compress_clips(
    app: AppHandle,
    paths: Vec<String>,
    quality: Option<umbra_core::settings::ShrinkQuality>,
) -> CmdResult<crate::shrink::ShrinkRequest> {
    blocking(move || crate::shrink::request(&app, &paths, quality).map_err(err)).await?
}

/// Queues clips to burn layers into (right-click, Burn in); returns how many were queued.
#[tauri::command]
pub async fn burn_in(
    app: AppHandle,
    paths: Vec<String>,
    layers: umbra_core::burnin::Layers,
) -> CmdResult<usize> {
    blocking(move || crate::burnin::request(&app, &paths, layers).map_err(err)).await?
}

/// Stops burning into these clips, the one at work and any waiting.
#[tauri::command]
pub async fn stop_burn_in(app: AppHandle, paths: Vec<String>) -> CmdResult<usize> {
    blocking(move || crate::burnin::stop(&app, &paths).map_err(err)).await?
}

/// Stops compressing these files, now and later.
#[tauri::command]
pub async fn stop_compressing(app: AppHandle, paths: Vec<String>) -> CmdResult<usize> {
    blocking(move || crate::shrink::stop(&app, &paths).map_err(err)).await?
}

/// Looks for old recordings to shrink right away.
#[tauri::command]
pub fn shrink_now() {
    crate::shrink::wake();
}

/// Files being worked on, for the library to show in progress.
#[tauri::command]
pub fn list_processing() -> Vec<ProcessingDto> {
    umbra_core::processing::list()
        .into_iter()
        .map(|p| {
            let elapsed = p.started.elapsed().as_secs_f64();
            let rate = if elapsed > 2.0 && p.done > 0 {
                p.done as f64 / elapsed
            } else {
                0.0
            };
            ProcessingDto {
                file_name: p
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: p.path.to_string_lossy().into_owned(),
                task: p.task,
                game: p.game,
                kind: p.kind,
                bytes_done: p.done,
                bytes_total: p.total,
                started_at: umbra_core::library::iso8601(p.started_at),
                eta_secs: (rate > 0.0).then(|| p.total.saturating_sub(p.done) as f64 / rate),
            }
        })
        .collect()
}

/// Other recorders' folders on this PC that hold videos.
#[tauri::command]
pub async fn list_import_sources(app: AppHandle) -> CmdResult<Vec<crate::import::ImportSource>> {
    blocking(move || crate::import::sources(&app)).await
}

/// What importing these files or folders would bring in.
#[tauri::command]
pub async fn scan_import(
    app: AppHandle,
    paths: Vec<String>,
) -> CmdResult<Vec<crate::import::ImportFile>> {
    blocking(move || crate::import::scan(&app, &paths)).await
}

/// Copies the files into the library in the background (`import-progress`, `import-done`).
#[tauri::command]
pub fn start_import(
    app: AppHandle,
    files: Vec<crate::import::ImportRequestFile>,
    move_originals: bool,
) -> CmdResult<()> {
    crate::import::start(&app, files, move_originals).map_err(err)
}

/// Shows a system picker owned by Umbra's window and returns the chosen path.
async fn pick(
    app: AppHandle,
    show: impl FnOnce(rfd::FileDialog) -> Option<std::path::PathBuf> + Send + 'static,
) -> CmdResult<Option<String>> {
    // A thread of its own: the dialog needs a single-threaded COM apartment, and the shared
    // worker threads may already be in the multithreaded one (capture sets that up), where
    // the dialog fails without showing.
    let picker = std::thread::spawn(move || {
        let mut dialog = rfd::FileDialog::new();
        if let Some(window) = app.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        show(dialog).map(|p| p.to_string_lossy().into_owned())
    });
    blocking(move || picker.join())
        .await?
        .map_err(|_| "the picker stopped unexpectedly".to_string())
}

#[tauri::command]
pub async fn save_clip(app: AppHandle) -> CmdResult<ClipDto> {
    blocking(move || clips::save(&app).map_err(err)).await?
}

/// A clip's keyboard and mouse input for the player: blocks of what was held and the events
/// after, mouse movement summed over thirtieths of a second. Events are
/// `[seconds, kind, a, b, c]`: 1 key down and 2 up (`a` virtual key, `b` scan code, `c` 1 for
/// an extended key), 3 and 4 a text key down and up, 5 and 6 a mouse button down and up (`a`:
/// 0 left, 1 right, 2 middle, 3 back, 4 forward), 7 the wheel (`a` up, `b` right, 120 a
/// notch), 8 movement (`a` right, `b` down).
#[tauri::command]
pub async fn get_input_track(app: AppHandle, path: String) -> CmdResult<serde_json::Value> {
    blocking(move || -> anyhow::Result<serde_json::Value> {
        let video = clips::validate(&app, &path)?;
        let Some(file) = umbra_core::inputfile::existing_sidecar(&video) else {
            return Ok(serde_json::json!({ "blocks": [] }));
        };
        let blocks = umbra_core::inputfile::read(&file)?;
        Ok(serde_json::json!({ "blocks": blocks.iter().map(view_block).collect::<Vec<_>>() }))
    })
    .await?
    .map_err(err)
}

fn view_block(block: &umbra_core::inputfile::Block) -> serde_json::Value {
    use umbra_core::inputfile::Event;
    const STEP: f64 = 1.0 / 30.0;
    let mut events: Vec<serde_json::Value> = Vec::new();
    let mut moved: Option<(f64, i32, i32)> = None;
    let round = |t: f64| (t * 1000.0).round() / 1000.0;
    for (t, event) in &block.events {
        if let Event::Move { dx, dy } = event {
            match &mut moved {
                Some((at, x, y)) if t - *at < STEP => {
                    *x += dx;
                    *y += dy;
                }
                _ => {
                    if let Some((at, x, y)) = moved.take() {
                        events.push(serde_json::json!([round(at), 8, x, y, 0]));
                    }
                    moved = Some((*t, *dx, *dy));
                }
            }
            continue;
        }
        let (kind, a, b, c) = match *event {
            Event::KeyDown(k) => (1, k.vk as i32, k.scan as i32, k.ext as i32),
            Event::KeyUp(k) => (2, k.vk as i32, k.scan as i32, k.ext as i32),
            Event::TextDown => (3, 0, 0, 0),
            Event::TextUp => (4, 0, 0, 0),
            Event::ButtonDown(b) => (5, b as i32, 0, 0),
            Event::ButtonUp(b) => (6, b as i32, 0, 0),
            Event::Wheel {
                vertical,
                horizontal,
            } => (7, vertical, horizontal, 0),
            Event::Move { .. } => unreachable!(),
        };
        events.push(serde_json::json!([round(*t), kind, a, b, c]));
    }
    if let Some((at, x, y)) = moved {
        events.push(serde_json::json!([round(at), 8, x, y, 0]));
    }
    events.sort_by(|a, b| {
        a[0].as_f64()
            .unwrap_or(0.0)
            .total_cmp(&b[0].as_f64().unwrap_or(0.0))
    });
    let p = &block.pressed;
    serde_json::json!({
        "start": round(block.start),
        "keys": p.keys.iter().map(|k| [k.vk as i32, k.scan as i32, k.ext as i32]).collect::<Vec<_>>(),
        "text": p.text,
        "buttons": p.buttons,
        "events": events,
    })
}

/// Switches recording keyboard and mouse input on or off (`null` switches it over).
#[tauri::command]
pub fn set_input_capture(app: AppHandle, on: Option<bool>) -> CmdResult<()> {
    app.state::<AppState>()
        .controller
        .set_input_capture(on)
        .map_err(err)
}

/// The auto-clip log, oldest first.
#[tauri::command]
pub fn get_auto_clip_log(app: AppHandle) -> Vec<umbra_core::autoclip::LogEntry> {
    app.state::<AppState>().controller.auto_clip_log()
}

/// Writes Counter-Strike 2's config file into the game's folder; returns where.
#[tauri::command]
pub async fn install_cs2_config(app: AppHandle) -> CmdResult<String> {
    blocking(move || {
        app.state::<AppState>()
            .controller
            .install_cs2_config()
            .map(|path| path.display().to_string())
            .map_err(err)
    })
    .await?
}

/// The config file's text, for copying by hand, and the folder it goes in when CS2 is found.
#[tauri::command]
pub async fn get_cs2_config(app: AppHandle) -> CmdResult<serde_json::Value> {
    blocking(move || {
        let cs2 = app.state::<AppState>().controller.settings().auto_clip.cs2;
        serde_json::json!({
            "text": umbra_core::cs2::config_text(cs2.port, &cs2.token),
            "fileName": umbra_core::cs2::CONFIG_NAME,
            "folder": umbra_core::cs2::config_dir().map(|d| d.display().to_string()),
        })
    })
    .await
}

#[tauri::command]
pub async fn list_clips(app: AppHandle) -> CmdResult<Vec<ClipDto>> {
    blocking(move || clips::list(&app)).await
}

#[tauri::command]
pub async fn delete_clip(app: AppHandle, path: String) -> CmdResult<()> {
    blocking(move || clips::delete_many(&app, &[path]).map_err(err)).await?
}

#[tauri::command]
pub async fn rename_clip(app: AppHandle, path: String, new_name: String) -> CmdResult<ClipDto> {
    blocking(move || clips::rename(&app, &path, &new_name).map_err(err)).await?
}

#[tauri::command]
pub async fn reveal_clip(app: AppHandle, path: String) -> CmdResult<()> {
    blocking(move || clips::reveal(&app, &path).map_err(err)).await?
}

#[tauri::command]
pub async fn export_clip(
    app: AppHandle,
    request: ExportRequest,
) -> CmdResult<clips::ExportResultDto> {
    blocking(move || clips::export(&app, request).map_err(err)).await?
}

/// Puts clips on the clipboard as files, as Explorer's Copy does.
#[tauri::command]
pub async fn copy_clips_to_clipboard(app: AppHandle, paths: Vec<String>) -> CmdResult<()> {
    blocking(move || clips::copy_to_clipboard(&app, &paths).map_err(err)).await?
}

/// Drags clips out of the window into another program (Discord, Explorer). Not async: a
/// drag has to run on the window's thread. True when they were dropped somewhere.
#[tauri::command]
pub fn drag_clips_out(
    app: AppHandle,
    window: tauri::WebviewWindow,
    paths: Vec<String>,
) -> CmdResult<bool> {
    crate::dragout::drag(&app, &window, &paths).map_err(err)
}

#[tauri::command]
pub async fn take_screenshot(app: AppHandle) -> CmdResult<ClipDto> {
    blocking(move || clips::screenshot(&app).map_err(err)).await?
}

#[tauri::command]
pub async fn delete_clips(app: AppHandle, paths: Vec<String>) -> CmdResult<()> {
    blocking(move || clips::delete_many(&app, &paths).map_err(err)).await?
}

#[tauri::command]
pub async fn get_track_audio(app: AppHandle, path: String) -> CmdResult<Vec<TrackAudioDto>> {
    blocking(move || clips::track_audio(&app, &path).map_err(err)).await?
}

#[tauri::command]
pub async fn get_waveforms(app: AppHandle, path: String) -> CmdResult<clips::WaveformsDto> {
    blocking(move || clips::waveforms(&app, &path).map_err(err)).await?
}

#[tauri::command]
pub async fn open_settings_folder() -> CmdResult<()> {
    blocking(|| clips::open_settings_folder().map_err(err)).await?
}

/// The games running now, for the lists of games in the settings.
#[tauri::command]
pub async fn list_running_games(app: AppHandle) -> CmdResult<Vec<RunningGameDto>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.controller.settings();
        let mut listed = settings.recording.games.clone();
        listed.extend(
            settings
                .presets
                .iter()
                .flat_map(|p| p.games.iter().cloned()),
        );
        umbra_core::games::running_games(&listed, state.controller.detected_game().as_ref())
            .into_iter()
            .map(|g| RunningGameDto {
                exe: g.exe,
                name: g.name,
            })
            .collect()
    })
    .await
}

/// Monitors first, then windows. Umbra's own window is left out.
#[tauri::command]
pub async fn list_capture_sources(app: AppHandle) -> CmdResult<Vec<CaptureSourceDto>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let games = state.controller.settings().recording.games;
        let detected = state.controller.detected_game().map(|g| g.exe);
        let mut sources: Vec<CaptureSourceDto> = monitors()
            .into_iter()
            .enumerate()
            .map(|(n, m)| CaptureSourceDto {
                label: format!("Display {}", n + 1),
                detail: format!("{}\u{d7}{} \u{b7} {} Hz", m.width, m.height, m.refresh_hz),
                target: CaptureChoice::Monitor { device: m.device },
                is_game: false,
            })
            .collect();
        let own = std::process::id();
        for window in wnd::list_windows() {
            if window.pid == own || window.exe.eq_ignore_ascii_case("explorer.exe") {
                continue;
            }
            let is_game = games.iter().any(|g| g.eq_ignore_ascii_case(&window.exe))
                || detected
                    .as_deref()
                    .is_some_and(|d| d.eq_ignore_ascii_case(&window.exe));
            sources.push(CaptureSourceDto {
                label: window.title.clone(),
                detail: window.exe.clone(),
                target: CaptureChoice::Window {
                    exe: window.exe,
                    title: window.title,
                },
                is_game,
            });
        }
        sources
    })
    .await
}

/// A small still of a source, or `None` when it can't be captured (a minimised window).
#[tauri::command]
pub async fn get_preview(target: CaptureChoice, max_width: u32) -> CmdResult<Option<String>> {
    blocking(move || {
        let Ok((resolved, _)) = resolve_target(&target) else {
            return None;
        };
        match umbra_core::snapshot::preview_data_url(resolved, max_width) {
            Ok(url) => Some(url),
            Err(e) => {
                log::info!("no preview: {e:#}");
                None
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn estimate_usage(app: AppHandle, settings: Settings) -> CmdResult<UsageEstimateDto> {
    blocking(move || {
        let settings = settings.normalized();
        // The size of what would be captured, for settings that keep the source size.
        let source = match &settings.capture {
            CaptureChoice::Monitor { device } => {
                let all = monitors();
                all.iter()
                    .find(|m| !device.is_empty() && m.device.eq_ignore_ascii_case(device))
                    .or_else(|| all.iter().find(|m| m.primary))
                    .map(|m| (m.width, m.height))
            }
            CaptureChoice::Window { exe, title } => {
                wnd::find_window(exe, title).and_then(|w| wnd::window_size(w.handle))
            }
        }
        .unwrap_or((1920, 1080));
        let gpu = app
            .state::<AppState>()
            .system_info
            .get()
            .map(|i| i.gpu.clone());
        let estimate = umbra_core::estimate::estimate(&settings, source, gpu.as_deref());
        UsageEstimateDto {
            buffer_bytes: estimate.buffer_bytes,
            clip_bytes: estimate.clip_bytes,
            encoder_load: (estimate.encoder_load * 100.0).round() / 100.0,
            cpu_percent: (estimate.cpu_percent * 10.0).round() / 10.0,
            level: estimate.level.as_str(),
            notes: estimate.notes,
            breakdown: estimate.breakdown,
        }
    })
    .await
}

#[tauri::command]
pub fn list_albums(app: AppHandle) -> Vec<Album> {
    clips::list_albums(&app)
}

#[tauri::command]
pub fn create_album(app: AppHandle, name: String) -> CmdResult<Album> {
    clips::album_change(&app, |albums| albums.create(&name)).map_err(err)
}

#[tauri::command]
pub fn rename_album(app: AppHandle, id: String, name: String) -> CmdResult<Album> {
    clips::album_change(&app, |albums| albums.rename(&id, &name)).map_err(err)
}

#[tauri::command]
pub fn delete_album(app: AppHandle, id: String) -> CmdResult<()> {
    clips::delete_album(&app, &id).map_err(err)
}

#[tauri::command]
pub fn set_album_hidden(app: AppHandle, id: String, hide_from_clips: bool) -> CmdResult<Album> {
    clips::album_change(&app, |albums| albums.set_hidden(&id, hide_from_clips)).map_err(err)
}

#[tauri::command]
pub async fn add_to_album(app: AppHandle, id: String, paths: Vec<String>) -> CmdResult<Album> {
    blocking(move || {
        let paths = clips::checked_paths(&app, &paths).map_err(err)?;
        clips::album_change(&app, |albums| albums.add(&id, &paths)).map_err(err)
    })
    .await?
}

/// Paths are not checked here: a clip that no longer exists can still be taken out.
#[tauri::command]
pub fn remove_from_album(app: AppHandle, id: String, paths: Vec<String>) -> CmdResult<Album> {
    clips::album_change(&app, |albums| albums.remove(&id, &paths)).map_err(err)
}

/// Measures the encoder and recommends video settings. Recording pauses while it runs, and
/// `benchmark-progress` events report each step. Applies nothing.
#[tauri::command]
pub async fn run_benchmark(
    app: AppHandle,
    answers: umbra_core::benchmark::Answers,
) -> CmdResult<umbra_core::benchmark::Report> {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    use umbra_core::benchmark::{self, Screen};

    blocking(move || {
        let state = app.state::<AppState>();
        if state.controller.is_file_recording() {
            return Err("Stop the recording first: the test pauses recording.".to_string());
        }
        state.benchmark_cancel.store(false, Ordering::Relaxed);
        let settings = state.controller.settings();
        // The recorded monitor, or the primary one when a window is recorded.
        let list = monitors();
        let monitor = match &settings.capture {
            CaptureChoice::Monitor { device } if !device.is_empty() => list
                .iter()
                .find(|m| m.device.eq_ignore_ascii_case(device))
                .or_else(|| list.iter().find(|m| m.primary)),
            _ => list.iter().find(|m| m.primary),
        }
        .or(list.first());
        let screen = monitor.map_or(
            Screen {
                width: 1920,
                height: 1080,
                refresh_hz: 60,
            },
            |m| Screen {
                width: m.native_width.max(16),
                height: m.native_height.max(16),
                refresh_hz: m.refresh_hz,
            },
        );

        state.controller.pause(true);
        // Let the recorder's encoder close before measuring.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let result = benchmark::run(
            &answers,
            &screen,
            &settings.video,
            settings.clip.buffer_secs(),
            umbra_core::estimate::total_ram(),
            &state.benchmark_cancel,
            &mut |progress| {
                let _ = app.emit("benchmark-progress", progress);
            },
        );
        state.controller.pause(false);
        result.map_err(err)
    })
    .await?
}

/// Starts recording what is being captured to a file. Clips keep working meanwhile.
#[tauri::command]
pub fn start_file_recording(
    app: AppHandle,
    kind: umbra_core::recordings::RecordingKind,
    source: Option<umbra_core::settings::CaptureChoice>,
) {
    let controller = &app.state::<AppState>().controller;
    match source {
        Some(source) => controller.start_file_recording_of(source),
        None => controller.start_file_recording(kind),
    }
}

/// Pauses (`true`) or resumes (`false`) the recording to a file.
#[tauri::command]
pub fn pause_file_recording(app: AppHandle, paused: bool) {
    app.state::<AppState>()
        .controller
        .pause_file_recording(paused);
}

/// Stops the recording to a file and keeps it; `recording-saved` follows.
#[tauri::command]
pub async fn stop_file_recording(app: AppHandle) -> CmdResult<()> {
    // Finishing writes the file's index, which can take a moment for a long recording.
    blocking(move || app.state::<AppState>().controller.stop_file_recording()).await
}

#[tauri::command]
pub fn cancel_benchmark(app: AppHandle) {
    app.state::<AppState>()
        .benchmark_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The share code for a set of video settings.
#[tauri::command]
pub fn video_settings_code(video: umbra_core::settings::VideoPrefs) -> String {
    umbra_core::share::encode(&video)
}

/// Video settings from a share code, brought into the supported ranges.
#[tauri::command]
pub fn read_video_settings_code(code: String) -> CmdResult<umbra_core::settings::VideoPrefs> {
    let video = umbra_core::share::decode(&code).map_err(err)?;
    let settings = Settings {
        video,
        ..Settings::default()
    };
    Ok(settings.normalized().video)
}

/// True until the first-start screen is finished, on a PC Umbra has not run on before.
#[tauri::command]
pub fn is_first_run(app: AppHandle) -> bool {
    app.state::<AppState>()
        .first_run
        .load(std::sync::atomic::Ordering::Relaxed)
}

#[tauri::command]
pub fn finish_onboarding(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    // Storing the settings is what marks Umbra as set up, even if nothing was changed.
    state
        .controller
        .set_settings(state.controller.settings())
        .map_err(err)?;
    state
        .first_run
        .store(false, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// How much the clip folder holds.
#[tauri::command]
pub async fn get_storage_usage(app: AppHandle) -> CmdResult<clips::StorageUsageDto> {
    blocking(move || clips::storage_usage(&app)).await
}

/// Applies the storage limits now.
#[tauri::command]
pub async fn clean_up_storage(app: AppHandle) -> CmdResult<clips::CleanupDto> {
    blocking(move || clips::enforce_limits(&app).map_err(err)).await?
}

/// The settings a new install starts with.
#[tauri::command]
pub fn get_default_settings() -> Settings {
    Settings::default()
}

/// Records with the default video settings (or a preset's) while a stand-in game loads the
/// graphics card, processor and memory, and says what to change if it costs the game too
/// much or drops frames. Normal recording pauses meanwhile; `loadtest-progress` reports.
#[tauri::command]
pub async fn run_load_test(
    app: AppHandle,
    preset_id: Option<String>,
) -> CmdResult<umbra_core::loadtest::LoadReport> {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    use umbra_core::loadtest;

    blocking(move || {
        let state = app.state::<AppState>();
        if state.controller.is_file_recording() {
            return Err("Stop the recording first: the test pauses recording.".to_string());
        }
        let settings = state.controller.settings();
        let video = preset_id
            .as_deref()
            .and_then(|id| settings.presets.iter().find(|p| p.id == id))
            .map_or(settings.video.clone(), |p| p.video.clone());

        // The screen the game would be on: the recorded monitor, or the primary one.
        let list = monitors();
        let monitor = match &settings.capture {
            CaptureChoice::Monitor { device } if !device.is_empty() => list
                .iter()
                .find(|m| m.device.eq_ignore_ascii_case(device))
                .or_else(|| list.iter().find(|m| m.primary)),
            _ => list.iter().find(|m| m.primary),
        }
        .or(list.first());
        let screen = monitor.map_or((1920, 1080), |m| (m.native_width, m.native_height));
        let output_height = video.height.unwrap_or(screen.1);
        let total = umbra_core::estimate::total_ram().unwrap_or(16 << 30);
        let mut with_video = settings.clone();
        with_video.video = video.clone();
        let buffer = with_video.estimated_buffer_bytes();
        let spec = loadtest::default_spec(screen, total);

        state.benchmark_cancel.store(false, Ordering::Relaxed);
        state.controller.pause(true);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let controller = &state.controller;
        let result = loadtest::run(
            spec,
            &video,
            output_height,
            settings.clip.buffer_secs(),
            buffer,
            total,
            &mut || controller.recorder_for_test(&video),
            &state.benchmark_cancel,
            &mut |p| {
                let _ = app.emit("loadtest-progress", p);
            },
        );
        state.controller.pause(false);
        result.map_err(err)
    })
    .await?
}
