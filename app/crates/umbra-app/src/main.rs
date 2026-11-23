// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod burnin;
mod clipboard;
mod clips;
mod commands;
mod dragout;
mod dto;
mod import;
mod media;
mod overlay;
mod shrink;
mod tray;

use std::sync::OnceLock;

use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use umbra_core::controller::{Controller, Event};
use umbra_core::settings;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Threading::CreateMutexW;

use crate::dto::{StatusDto, SystemInfoDto};

pub struct AppState {
    pub controller: Controller,
    pub system_info: OnceLock<SystemInfoDto>,
    pub albums: umbra_core::albums::AlbumStore,
    /// No settings file existed when Umbra started: show the first-start screen.
    pub first_run: std::sync::atomic::AtomicBool,
    /// Set to stop a running benchmark.
    pub benchmark_cancel: std::sync::atomic::AtomicBool,
}

pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn on_tray_menu(app: &AppHandle, id: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match id {
        "open" => show_main_window(app),
        "save" => {
            // Off the window thread: writing the file takes a moment.
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(err) = app.state::<AppState>().controller.save_clip() {
                    log::warn!("save from tray failed: {err:#}");
                }
            });
        }
        "toggle" => {
            if state.controller.status().state == umbra_core::controller::State::Recording {
                state.controller.stop_recording();
            } else {
                state.controller.start_recording();
            }
        }
        "camera" => {
            if let Err(err) = state.controller.set_camera_on(None) {
                log::warn!("cannot switch the camera: {err:#}");
            }
        }
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Only one Umbra at a time: two would both grab the hotkey and record the same screen.
/// One Umbra per settings folder. A copy pointed at another folder (`UMBRA_SETTINGS_DIR`, as
/// the test version is) may run next to the installed one.
fn already_running() -> bool {
    let mut name = String::from("Local\\UmbraClipperSingleInstance");
    if std::env::var_os("UMBRA_SETTINGS_DIR").is_some() {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        settings::data_dir().hash(&mut hasher);
        name.push_str(&format!("-{:016x}", hasher.finish()));
    }
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        // Kept for the life of the process on purpose.
        let _ = CreateMutexW(None, false, windows::core::PCWSTR(wide.as_ptr()));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

/// Writes to the console and to `logs/umbra.log` next to the cache folder, so what went wrong
/// can be looked up after the fact. The file starts over past 5 MB (the previous one is kept
/// as `umbra.old.log`).
struct Logger {
    folder: std::path::PathBuf,
    file: std::sync::Mutex<Option<std::fs::File>>,
}

const LOG_LIMIT: u64 = 5 << 20;

impl Logger {
    fn open() -> Self {
        let folder = settings::cache_dir().with_file_name("logs");
        let file = std::fs::create_dir_all(&folder)
            .ok()
            .and_then(|()| Self::open_file(&folder));
        Self {
            folder,
            file: std::sync::Mutex::new(file),
        }
    }

    fn open_file(folder: &std::path::Path) -> Option<std::fs::File> {
        let path = folder.join("umbra.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
            let _ = std::fs::rename(&path, folder.join("umbra.old.log"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
            let mut slot = self.file.lock().unwrap();
            if let Some(file) = slot.as_mut() {
                use std::io::Write;
                let when = umbra_core::library::iso8601(std::time::SystemTime::now());
                let _ = writeln!(file, "{when} [{}] {}", record.level(), record.args());
                // Checked as it grows, not only at launch: Umbra runs for days.
                if file.metadata().is_ok_and(|m| m.len() > LOG_LIMIT) {
                    *slot = None;
                    *slot = Self::open_file(&self.folder);
                }
            }
        }
    }
    fn flush(&self) {}
}

fn main() {
    let _ = log::set_logger(Box::leak(Box::new(Logger::open())));
    log::set_max_level(log::LevelFilter::Info);
    umbra_core::util::set_dpi_aware();
    umbra_core::quiet_ffmpeg_logs();

    if already_running() {
        return;
    }
    let minimized_flag = std::env::args().any(|a| a == "--minimized");

    // Settings moved from %APPDATA% to Documents; bring an older install's files along.
    for moved in settings::migrate_legacy() {
        log::info!("moved to {}", moved.display());
    }
    let first_run = !settings::settings_path().exists();

    // The controller (and its input hooks) is created before the window system starts, and
    // gets the app handle once it exists.
    let handle_cell: std::sync::Arc<OnceLock<AppHandle>> = std::sync::Arc::new(OnceLock::new());
    let sink_cell = handle_cell.clone();
    let controller = match Controller::start(settings::settings_path(), move |event| {
        let Some(handle) = sink_cell.get() else {
            return;
        };
        match event {
            Event::Status(status) => {
                let dto = StatusDto::from(status);
                tray::update(handle, &dto);
                overlay::update_status(handle, &dto);
                let _ = handle.emit("status", dto);
            }
            Event::ClipSaved(saved) => {
                overlay::clip_saved(handle, &saved);
                clips::on_clip_saved(handle, saved)
            }
            Event::ScreenshotSaved(saved) => clips::on_screenshot_saved(handle, saved),
            Event::RecordingSaved(saved) => {
                overlay::recording_saved(handle, &saved);
                clips::on_recording_saved(handle, saved)
            }
            Event::RecordingStarted { kind, game } => {
                overlay::recording_started(handle, kind, &game)
            }
            Event::ClipPending {
                secs,
                game,
                length_secs,
            } => {
                if secs > 0 {
                    overlay::clip_pending(handle, secs);
                }
                // The library shows a placeholder until the clip is there.
                let _ = handle.emit(
                    "clip-saving",
                    serde_json::json!({ "game": game, "waitSecs": secs, "lengthSecs": length_secs }),
                );
            }
            Event::ClipFailed { error } => {
                let _ = handle.emit("clip-failed", serde_json::json!({ "error": error }));
            }
            Event::LowDisk {
                free_bytes,
                limit_bytes,
                stopped,
            } => overlay::low_disk(handle, free_bytes, limit_bytes, stopped),
            Event::RecordingPaused { paused, kind, game } => {
                overlay::recording_paused(handle, paused, kind, &game)
            }
            Event::RecordingFinishing { kind, game } => {
                let _ = handle.emit(
                    "recording-finishing",
                    serde_json::json!({ "kind": kind, "game": game }),
                );
            }
            Event::LibraryChanged => {
                let _ = handle.emit("clips-changed", ());
            }
            Event::ProcessingStarted => {
                let _ = handle.emit("processing-started", ());
            }
            Event::AutoClipLog(entry) => {
                let _ = handle.emit("auto-clip-log", entry);
            }
        }
    }) {
        Ok(controller) => controller,
        Err(err) => {
            eprintln!("Umbra cannot start: {err:#}");
            return;
        }
    };
    // Windows' startup entry, put right if it went missing or points at another copy.
    if let Err(err) = umbra_core::controller::apply_startup(&controller.settings()) {
        log::warn!("cannot update launch at startup: {err:#}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_settings,
            commands::get_status,
            commands::get_system_info,
            commands::start_recording,
            commands::stop_recording,
            commands::save_clip,
            commands::get_input_track,
            commands::set_input_capture,
            commands::get_auto_clip_log,
            commands::install_cs2_config,
            commands::get_cs2_config,
            commands::take_screenshot,
            commands::list_monitors,
            commands::list_windows,
            commands::list_audio_apps,
            commands::list_microphones,
            commands::list_cameras,
            commands::pick_logo,
            commands::get_camera_video,
            commands::list_capture_sources,
            commands::get_preview,
            commands::estimate_usage,
            commands::run_benchmark,
            commands::run_load_test,
            commands::start_file_recording,
            commands::get_default_settings,
            commands::get_storage_usage,
            commands::clean_up_storage,
            overlay::hide_overlay,
            commands::is_first_run,
            commands::finish_onboarding,
            commands::stop_file_recording,
            commands::pause_file_recording,
            commands::pick_program,
            commands::copy_clips_to_clipboard,
            commands::drag_clips_out,
            commands::pick_videos,
            commands::list_import_sources,
            commands::scan_import,
            commands::start_import,
            commands::list_processing,
            commands::shrink_estimate,
            commands::shrink_now,
            commands::compress_clips,
            commands::stop_compressing,
            commands::burn_in,
            commands::stop_burn_in,
            commands::list_running_games,
            commands::cancel_benchmark,
            commands::video_settings_code,
            commands::read_video_settings_code,
            commands::open_settings_folder,
            commands::list_clips,
            commands::delete_clip,
            commands::delete_clips,
            commands::rename_clip,
            commands::reveal_clip,
            commands::export_clip,
            commands::get_track_audio,
            commands::get_waveforms,
            commands::list_albums,
            commands::create_album,
            commands::rename_album,
            commands::delete_album,
            commands::set_album_hidden,
            commands::add_to_album,
            commands::remove_from_album,
            commands::pick_folder,
        ])
        .on_window_event(|window, event| {
            // Closing the window hides it; recording carries on from the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let _ = handle_cell.set(handle.clone());

            let current = controller.settings();
            commands::allow_clip_folders(&handle, &current);
            media::start(&handle);
            shrink::start(&handle);
            // Windows starts Umbra with --minimized at login; a normal launch always shows the window.
            let start_hidden = minimized_flag && current.recording.start_minimized;
            let initial = StatusDto::from(controller.status());
            app.manage(AppState {
                controller,
                system_info: OnceLock::new(),
                albums: umbra_core::albums::AlbumStore::load(
                    settings::data_dir().join("albums.json"),
                ),
                benchmark_cancel: Default::default(),
                first_run: std::sync::atomic::AtomicBool::new(first_run),
            });

            if let Err(err) = overlay::create(&handle) {
                log::warn!("cannot create the overlay window: {err}");
            }
            // Storage limits: once now, then every hour.
            let hourly = handle.clone();
            std::thread::spawn(move || {
                loop {
                    clips::enforce_limits_later(&hourly);
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            });
            tray::create(&handle)?;
            tray::update(&handle, &initial);
            if !start_hidden {
                show_main_window(&handle);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Umbra");
}
