//! Short messages over the game: a clip was saved, a recording started or was saved. They
//! appear in a small window that is always on top, never takes the focus and lets clicks
//! through, so the game keeps the keyboard and mouse. A game in exclusive fullscreen draws
//! over everything and hides it; borderless and windowed games show it.

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use umbra_core::controller::{ClipEvent, RecordingEvent};
use umbra_core::recordings::RecordingKind;
use umbra_core::settings::{CaptureChoice, Corner, StatusOverlay};

use crate::AppState;
use crate::dto::StatusDto;

const LABEL: &str = "overlay";
const WIDTH: f64 = 340.0;
const HEIGHT: f64 = 84.0;
const MARGIN: i32 = 24;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Message {
    title: String,
    detail: String,
    /// `clip`, `recording` or `error`, for the icon.
    tone: &'static str,
}

const STATUS_LABEL: &str = "status-overlay";
const STATUS_WIDTH: f64 = 168.0;
const STATUS_HEIGHT: f64 = 44.0;

/// Creates the message window and the status badge window, hidden. Both load the same page
/// (`?overlay`, `?status`).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = overlay_window(app, LABEL, "index.html?overlay", WIDTH, HEIGHT)?;
    exclude_from_capture(&window);
    let status = overlay_window(
        app,
        STATUS_LABEL,
        "index.html?status",
        STATUS_WIDTH,
        STATUS_HEIGHT,
    )?;
    exclude_from_capture(&status);
    Ok(())
}

fn overlay_window(
    app: &AppHandle,
    label: &str,
    url: &str,
    width: f64,
    height: f64,
) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title("Umbra overlay")
        .inner_size(width, height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .resizable(false)
        .visible(false)
        .build()?;
    let _ = window.set_ignore_cursor_events(true);
    Ok(window)
}

/// Keeps the window out of screen captures, Umbra's own included: the badge and the messages
/// are for the player, not for the clips.
fn exclude_from_capture(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };
    if let Ok(hwnd) = window.hwnd() {
        let hwnd = HWND(hwnd.0 as *mut _);
        if let Err(err) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
            log::warn!("cannot keep the overlay out of captures: {err}");
        }
    }
}

/// What the badge says, for the page.
#[derive(Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Badge {
    /// `recording`, `paused`, `buffer` or `off`.
    mode: &'static str,
    /// Seconds recorded (a recording) or held (the replay buffer).
    secs: f64,
}

/// Shows, moves or hides the status badge for the current status; called on every status
/// update (about once a second).
pub fn update_status(app: &AppHandle, status: &StatusDto) {
    let Some(window) = app.get_webview_window(STATUS_LABEL) else {
        return;
    };
    let state = app.state::<AppState>();
    let settings = state.controller.settings();
    let game = state.controller.detected_game();
    let wanted = match settings.overlay.status {
        StatusOverlay::Off => false,
        StatusOverlay::Always => true,
        StatusOverlay::Games => game.as_ref().is_some_and(|g| {
            settings.overlay.status_games.is_empty()
                || settings
                    .overlay
                    .status_games
                    .iter()
                    .any(|entry| umbra_core::apps::matches(entry, &g.exe, g.path.as_deref()))
        }),
    };
    if !wanted {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        }
        return;
    }
    let badge = match &status.file {
        Some(file) if file.paused => Badge {
            mode: "paused",
            secs: file.secs,
        },
        Some(file) => Badge {
            mode: "recording",
            secs: file.secs,
        },
        None if status.state == "recording" => Badge {
            mode: "buffer",
            secs: status.buffer_secs,
        },
        None => Badge {
            mode: "off",
            secs: 0.0,
        },
    };

    // On the game's screen, else the captured one, else the main one.
    let monitors = umbra_core::capture::list_monitors();
    let device = game
        .as_ref()
        .and_then(|g| g.screen)
        .and_then(|h| monitors.iter().find(|m| m.handle == h))
        .map(|m| m.device.clone())
        .or_else(|| match &settings.capture {
            CaptureChoice::Monitor { device } if !device.is_empty() => Some(device.clone()),
            _ => None,
        });
    let monitor = window.available_monitors().ok().and_then(|list| {
        list.into_iter().find(|m| {
            device
                .as_deref()
                .is_some_and(|d| m.name().is_some_and(|n| n.eq_ignore_ascii_case(d)))
        })
    });
    let monitor = monitor.or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let (w, h) = (
            (STATUS_WIDTH * scale) as i32,
            (STATUS_HEIGHT * scale) as i32,
        );
        let (left, top) = (monitor.position().x, monitor.position().y);
        let (right, bottom) = (
            left + monitor.size().width as i32,
            top + monitor.size().height as i32,
        );
        let margin = (12.0 * scale) as i32;
        let (x, y) = match settings.overlay.status_corner {
            Corner::TopLeft => (left + margin, top + margin),
            Corner::TopRight => (right - w - margin, top + margin),
            Corner::BottomLeft => (left + margin, bottom - h - margin),
            Corner::BottomRight => (right - w - margin, bottom - h - margin),
        };
        if window.outer_position().ok() != Some(PhysicalPosition::new(x, y)) {
            let _ = window.set_position(PhysicalPosition::new(x, y));
        }
    }
    let _ = window.emit("status-badge", badge);
    if !window.is_visible().unwrap_or(false) {
        let _ = window.show();
    }
    let _ = window.set_always_on_top(true);
}

fn show(app: &AppHandle, message: Message) {
    let settings = app.state::<AppState>().controller.settings();
    if !settings.overlay.enabled {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    // In the chosen corner of the monitor the cursor is on, which is where the game is.
    let monitor = window
        .cursor_position()
        .ok()
        .and_then(|p| window.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let (w, h) = ((WIDTH * scale) as i32, (HEIGHT * scale) as i32);
        let area = monitor.work_area();
        let (left, top) = (area.position.x, area.position.y);
        let (right, bottom) = (left + area.size.width as i32, top + area.size.height as i32);
        let margin = (MARGIN as f64 * scale) as i32;
        let (x, y) = match settings.overlay.corner {
            Corner::TopLeft => (left + margin, top + margin),
            Corner::TopRight => (right - w - margin, top + margin),
            Corner::BottomLeft => (left + margin, bottom - h - margin),
            Corner::BottomRight => (right - w - margin, bottom - h - margin),
        };
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
    let _ = window.emit("overlay-message", message);
    let _ = window.show();
    let _ = window.set_always_on_top(true);
}

/// Hides the overlay; the page calls this when its message has faded.
#[tauri::command]
pub fn hide_overlay(app: AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

pub fn clip_saved(app: &AppHandle, saved: &ClipEvent) {
    show(
        app,
        Message {
            title: "Clip saved".into(),
            detail: format!("{} · {:.0} s", saved.game, saved.clip.duration_secs),
            tone: "clip",
        },
    );
}

pub fn clip_pending(app: &AppHandle, secs: u32) {
    show(
        app,
        Message {
            title: "Saving the clip".into(),
            detail: format!("Recording {secs} s more, then it is saved"),
            tone: "clip",
        },
    );
}

/// The clip folder's drive is nearly full: said in the overlay, in a Windows notification
/// (seen even when Umbra is in the tray) and in the window.
pub fn low_disk(
    app: &AppHandle,
    free_bytes: u64,
    limit_bytes: u64,
    stopped: Option<RecordingKind>,
) {
    use tauri_plugin_notification::NotificationExt;
    let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
    let title = match stopped {
        Some(RecordingKind::Session) => "Session stopped: disk almost full",
        Some(RecordingKind::Manual) => "Recording stopped: disk almost full",
        None => "Not recording: disk almost full",
    };
    let detail = format!(
        "{:.1} GB left, under the {:.0} GB set in Storage. {}",
        gb(free_bytes),
        gb(limit_bytes),
        if stopped.is_some() {
            "What was recorded is saved."
        } else {
            "Free up space to record again."
        }
    );
    show(
        app,
        Message {
            title: title.into(),
            detail: detail.clone(),
            tone: "error",
        },
    );
    if let Err(err) = app
        .notification()
        .builder()
        .title(title)
        .body(&detail)
        .show()
    {
        log::warn!("cannot show a notification: {err}");
    }
    let _ = app.emit(
        "low-disk",
        serde_json::json!({ "title": title, "detail": detail }),
    );
}

pub fn recording_started(app: &AppHandle, kind: RecordingKind, game: &str) {
    show(
        app,
        Message {
            title: match kind {
                RecordingKind::Session => "Recording this session".into(),
                RecordingKind::Manual => "Recording started".into(),
            },
            detail: game.to_string(),
            tone: "recording",
        },
    );
}

pub fn recording_paused(app: &AppHandle, paused: bool, kind: RecordingKind, game: &str) {
    let what = match kind {
        RecordingKind::Session => "Session",
        RecordingKind::Manual => "Recording",
    };
    show(
        app,
        Message {
            title: format!("{what} {}", if paused { "paused" } else { "resumed" }),
            detail: game.to_string(),
            tone: "recording",
        },
    );
}

pub fn recording_saved(app: &AppHandle, event: &RecordingEvent) {
    let what = match event.kind {
        RecordingKind::Session => "Session",
        RecordingKind::Manual => "Recording",
    };
    let message = match &event.saved {
        Some(saved) => Message {
            title: format!("{what} saved"),
            detail: format!("{} · {}", event.game, duration(saved.duration_secs)),
            tone: "clip",
        },
        None => Message {
            title: format!("{what} not saved"),
            detail: event.error.clone().unwrap_or_default(),
            tone: "error",
        },
    };
    show(app, message);
}

fn duration(secs: f64) -> String {
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
