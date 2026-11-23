//! Tray icons: one shows whether the replay buffer is recording, a second appears while the
//! microphone is live. Both are drawn in code so the app ships no extra image files.

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Wry};

use crate::dto::StatusDto;

const SIZE: u32 = 32;
const GREEN: [u8; 3] = [34, 197, 94];
const RED: [u8; 3] = [239, 68, 68];
const GREY: [u8; 3] = [170, 170, 170];

/// The menu's camera switch, ticked while the camera is on.
static CAMERA: std::sync::OnceLock<CheckMenuItem<Wry>> = std::sync::OnceLock::new();

#[derive(Clone, Copy)]
enum Shape {
    Disc,
    Ring,
    Square,
}

/// A simple filled or outlined shape on a transparent background.
fn glyph(shape: Shape, colour: [u8; 3]) -> Image<'static> {
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let centre = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
            let distance = (dx * dx + dy * dy).sqrt();
            let coverage = match shape {
                Shape::Disc => (11.0 - distance + 0.5).clamp(0.0, 1.0),
                Shape::Ring => {
                    let outer = (11.0 - distance + 0.5).clamp(0.0, 1.0);
                    let inner = (distance - 7.0 + 0.5).clamp(0.0, 1.0);
                    outer.min(inner)
                }
                Shape::Square => {
                    let edge = dx.abs().max(dy.abs());
                    (9.0 - edge + 0.5).clamp(0.0, 1.0)
                }
            };
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i..i + 3].copy_from_slice(&colour);
            rgba[i + 3] = (coverage * 255.0) as u8;
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Umbra", true, None::<&str>)?;
    let save = MenuItem::with_id(app, "save", "Save clip", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start recording", true, None::<&str>)?;
    let camera = CheckMenuItem::with_id(app, "camera", "Camera", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Umbra", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &save,
            &toggle,
            &camera,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let _ = CAMERA.set(camera);
    TrayIconBuilder::<Wry>::with_id("main")
        .icon(glyph(Shape::Ring, GREY))
        .tooltip("Umbra: not recording")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| crate::on_tray_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    // Hidden until the microphone is live.
    let mic = TrayIconBuilder::<Wry>::with_id("mic")
        .icon(glyph(Shape::Square, GREEN))
        .tooltip("Umbra: microphone is live")
        .build(app)?;
    mic.set_visible(false)?;
    Ok(())
}

/// Reflects the recording state in the tray.
pub fn update(app: &AppHandle, status: &StatusDto) {
    if let Some(tray) = app.tray_by_id("main") {
        let (shape, colour, text) = match status.state {
            "recording" => (
                Shape::Disc,
                GREEN,
                match &status.game {
                    Some(game) => format!("Umbra: recording ({game})"),
                    None => "Umbra: recording".to_string(),
                },
            ),
            "error" => (Shape::Disc, RED, "Umbra: recording problem".to_string()),
            _ => (Shape::Ring, GREY, "Umbra: not recording".to_string()),
        };
        let _ = tray.set_icon(Some(glyph(shape, colour)));
        let _ = tray.set_tooltip(Some(text));
    }
    if let Some(camera) = CAMERA.get() {
        let _ = camera.set_checked(status.camera_on);
    }
    if let Some(mic) = app.tray_by_id("mic") {
        let _ = mic.set_visible(status.mic_live && status.state == "recording");
    }
}
