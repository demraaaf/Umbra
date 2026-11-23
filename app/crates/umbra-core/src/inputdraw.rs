//! Drawing the input overlay for export: the keys and mouse of a moment, as the player and
//! editor draw them (`ui/src/lib/input/overlay.ts`). Both read their geometry from the same
//! file (`ui/src/lib/input/layouts.json`), so they cannot drift apart. Pictures are drawn
//! with Direct2D and DirectWrite into memory, only when what shows changes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::Deserialize;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_LINE_JOIN_ROUND, D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE, D2D1CreateFactory, ID2D1Factory, ID2D1GeometrySink,
    ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWriteCreateFactory, IDWriteFactory,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmap, IWICImagingFactory,
    WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::w;
use windows_numerics::Vector2;

use crate::inputfile::{Block, Button, Event, Key, Pressed};
use crate::settings::{InputOverlayPrefs, OverlayKeys};

// ---------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LayoutKey {
    pub label: String,
    pub vk: u16,
    pub scan: u16,
    pub ext: bool,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl LayoutKey {
    fn matches(&self, key: &Key) -> bool {
        (key.scan != 0 && key.scan == self.scan && key.ext == self.ext) || key.vk == self.vk
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeysLayout {
    pub width: f64,
    pub height: f64,
    pub keys: Vec<LayoutKey>,
    /// Where text keys (kept without saying which) light up.
    pub text: Rect,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Circle {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// An egg-shaped body, its top and bottom rounded by `top` and `bottom` high; the left and
/// right buttons are its top above `split`, either side of the middle.
#[derive(Clone, Debug, Deserialize)]
pub struct MouseLayout {
    pub width: f64,
    pub height: f64,
    pub body: Rect,
    pub top: f64,
    pub bottom: f64,
    pub split: f64,
    pub wheel: Rect,
    pub x1: Rect,
    pub x2: Rect,
    pub movement: Circle,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Ticker {
    pub height: f64,
    pub chips: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Layouts {
    pub gap: f64,
    pub radius: f64,
    pub font: f64,
    pub wasd: KeysLayout,
    pub keyboard: KeysLayout,
    pub mouse: MouseLayout,
    pub ticker: Ticker,
    pub spacing: f64,
    /// Short names of keys by virtual-key code, for the ticker.
    pub names: HashMap<String, String>,
}

pub fn layouts() -> &'static Layouts {
    static LAYOUTS: OnceLock<Layouts> = OnceLock::new();
    LAYOUTS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../ui/src/lib/input/layouts.json"))
            .expect("the overlay layouts are valid")
    })
}

/// What the overlay is made of, placed side by side (keys, then the mouse) with the ticker
/// under them, in key widths.
pub struct Composition {
    pub keys: Option<&'static KeysLayout>,
    pub mouse: Option<(f64, &'static MouseLayout)>,
    pub ticker: Option<f64>,
    pub width: f64,
    pub height: f64,
}

impl Composition {
    pub fn new(prefs: &InputOverlayPrefs) -> Option<Self> {
        let l = layouts();
        let keys = match prefs.keys {
            OverlayKeys::None => None,
            OverlayKeys::Wasd => Some(&l.wasd),
            OverlayKeys::Keyboard => Some(&l.keyboard),
        };
        let mut width = keys.map_or(0.0, |k| k.width);
        let mut height = keys.map_or(0.0, |k| k.height);
        let mouse = prefs.mouse.then(|| {
            let x = if keys.is_some() {
                width + l.spacing
            } else {
                0.0
            };
            width = x + l.mouse.width;
            height = height.max(l.mouse.height);
            (x, &l.mouse)
        });
        let ticker = prefs.ticker.then(|| {
            let y = if height > 0.0 {
                height + l.spacing
            } else {
                0.0
            };
            height = y + l.ticker.height;
            width = width.max(6.0);
            y
        });
        (width > 0.0).then_some(Self {
            keys,
            mouse,
            ticker,
            width,
            height,
        })
    }
}

// ---------------------------------------------------------------------------------------
// What shows at a moment
// ---------------------------------------------------------------------------------------

/// What the overlay shows at a moment; two equal ones draw the same picture.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Visual {
    /// Per key of the layout, held or not.
    pub keys: Vec<bool>,
    pub text: bool,
    pub buttons: u8,
    /// 1 turned up, -1 down, 0 still.
    pub wheel: i8,
    /// Which way the mouse goes, -8 to 8 on each axis.
    pub movement: (i8, i8),
    pub ticker: Vec<String>,
}

/// How long a wheel turn lights up, and the stretch movement is summed over.
const WHEEL_SECS: f64 = 0.15;
const MOVE_SECS: f64 = 0.1;
/// Movement (mouse counts) over `MOVE_SECS` that moves the dot all the way out.
const MOVE_FULL: f64 = 300.0;
/// How long a key stays in the ticker.
const TICKER_SECS: f64 = 2.0;

/// A clip's input, ready to tell what shows at any moment.
pub struct Timeline {
    blocks: Vec<Block>,
    events: Vec<(f64, Event)>,
}

impl Timeline {
    pub fn new(blocks: Vec<Block>) -> Self {
        let mut events: Vec<(f64, Event)> = blocks
            .iter()
            .flat_map(|b| b.events.iter().copied())
            .collect();
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { blocks, events }
    }

    pub fn load(path: &Path) -> Result<Self> {
        Ok(Self::new(crate::inputfile::read(path)?))
    }

    fn between(&self, from: f64, to: f64) -> &[(f64, Event)] {
        let a = self.events.partition_point(|(t, _)| *t <= from);
        let b = self.events.partition_point(|(t, _)| *t <= to);
        &self.events[a..b]
    }

    pub fn visual(&self, t: f64, composition: &Composition) -> Visual {
        let pressed = crate::inputfile::pressed_at(&self.blocks, t);
        let keys = composition.keys.map_or_else(Vec::new, |layout| {
            layout
                .keys
                .iter()
                .map(|k| pressed.keys.iter().any(|key| k.matches(key)))
                .collect()
        });
        let mut wheel = 0;
        let (mut dx, mut dy) = (0i64, 0i64);
        if composition.mouse.is_some() {
            for (_, event) in self.between(t - WHEEL_SECS, t) {
                if let Event::Wheel { vertical, .. } = event {
                    wheel = vertical.signum() as i8;
                }
            }
            for (_, event) in self.between(t - MOVE_SECS, t) {
                if let Event::Move { dx: x, dy: y } = event {
                    dx += i64::from(*x);
                    dy += i64::from(*y);
                }
            }
        }
        let movement = {
            let length = (dx as f64).hypot(dy as f64);
            if length < 1.0 {
                (0, 0)
            } else {
                let reach = (length / MOVE_FULL).min(1.0) * 8.0 / length;
                (
                    (dx as f64 * reach).round() as i8,
                    (dy as f64 * reach).round() as i8,
                )
            }
        };
        let ticker = if composition.ticker.is_some() {
            self.ticker(t)
        } else {
            Vec::new()
        };
        Visual {
            keys,
            text: pressed.text > 0,
            buttons: pressed.buttons,
            wheel,
            movement,
            ticker,
        }
    }

    /// The last keys pressed: `Ctrl+C`, `W`, `•` for a text key. Shift, Ctrl, Alt and the
    /// Windows key show only with another.
    fn ticker(&self, t: f64) -> Vec<String> {
        let mut labels = Vec::new();
        for (at, event) in self.between(t - TICKER_SECS, t) {
            match event {
                Event::TextDown => labels.push("•".to_string()),
                Event::KeyDown(key) if !is_modifier(key.vk) => {
                    let held = crate::inputfile::pressed_at(&self.blocks, *at - 1e-6);
                    labels.push(combo_label(&held, key));
                }
                _ => {}
            }
        }
        let chips = layouts().ticker.chips;
        labels.split_off(labels.len().saturating_sub(chips))
    }
}

fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

pub fn key_name(vk: u16) -> String {
    layouts()
        .names
        .get(&vk.to_string())
        .cloned()
        .unwrap_or_else(|| format!("#{vk}"))
}

fn combo_label(held: &Pressed, key: &Key) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let has = |test: fn(u16) -> bool| held.keys.iter().any(|k| test(k.vk));
    if has(|vk| matches!(vk, 0x11 | 0xA2 | 0xA3)) {
        parts.push("Ctrl");
    }
    if has(|vk| matches!(vk, 0x10 | 0xA0 | 0xA1)) {
        parts.push("Shift");
    }
    if has(|vk| matches!(vk, 0x12 | 0xA4 | 0xA5)) {
        parts.push("Alt");
    }
    if has(|vk| matches!(vk, 0x5B | 0x5C)) {
        parts.push("Win");
    }
    let name = key_name(key.vk);
    parts.push(&name);
    parts.join("+")
}

// ---------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------

/// The overlay's colours: the app's dark style, keys held in the chosen highlight.
struct Palette {
    fill: [f32; 4],
    edge: [f32; 4],
    /// The mouse wheel's slot, and the scroll arrow's rim.
    slot: [f32; 4],
    /// The wheel, the side buttons and the resting movement dot, when not lit.
    wheel: [f32; 4],
    label: [f32; 4],
    held: [f32; 4],
    held_label: [f32; 4],
}

impl Palette {
    fn new(prefs: &InputOverlayPrefs) -> Self {
        let held = parse_hex(&prefs.highlight);
        // Dark text on a light highlight, light text on a dark one.
        let light = 0.2126 * held[0] + 0.7152 * held[1] + 0.0722 * held[2] > 0.5;
        Self {
            fill: [0.094, 0.094, 0.106, 0.82],
            edge: [1.0, 1.0, 1.0, 0.14],
            slot: [0.0, 0.0, 0.0, 0.5],
            wheel: [1.0, 1.0, 1.0, 0.22],
            label: [0.83, 0.83, 0.85, 1.0],
            held: [held[0], held[1], held[2], 1.0],
            held_label: if light {
                [0.094, 0.094, 0.106, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            },
        }
    }
}

fn parse_hex(css: &str) -> [f32; 3] {
    let hex = css.trim().trim_start_matches('#');
    let value = u32::from_str_radix(hex, 16).unwrap_or(0xffffff);
    match hex.len() {
        3 => [(value >> 8) & 0xf, (value >> 4) & 0xf, value & 0xf].map(|d| (d * 17) as f32 / 255.0),
        _ => [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff].map(|d| d as f32 / 255.0),
    }
}

/// Draws overlays into memory. One per export; not shared between threads.
pub struct Painter {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    /// Text formats by size, in pixels.
    formats: HashMap<u32, IDWriteTextFormat>,
}

/// A drawn overlay: straight (not premultiplied) RGBA.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Painter {
    pub fn new() -> Result<Self> {
        unsafe {
            // WIC needs COM; a thread that has it already keeps its own.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            Ok(Self {
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                    .context("cannot start Direct2D")?,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
                    .context("cannot start DirectWrite")?,
                wic: CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .context("cannot start WIC")?,
                formats: HashMap::new(),
            })
        }
    }

    fn format(&mut self, px: f32) -> Result<IDWriteTextFormat> {
        let key = (px * 4.0).round() as u32;
        if let Some(format) = self.formats.get(&key) {
            return Ok(format.clone());
        }
        let format = unsafe {
            let format = self.dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                key as f32 / 4.0,
                w!("en-us"),
            )?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format
        };
        self.formats.insert(key, format.clone());
        Ok(format)
    }

    fn text_width(&mut self, text: &str, px: f32) -> Result<f32> {
        let format = self.format(px)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self
                .dwrite
                .CreateTextLayout(&wide, &format, 10_000.0, 10_000.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok(metrics.width)
        }
    }

    /// The overlay `visual` of `composition`, `width` pixels wide, at `prefs`' look.
    pub fn paint(
        &mut self,
        composition: &Composition,
        visual: &Visual,
        width: u32,
        prefs: &InputOverlayPrefs,
    ) -> Result<Picture> {
        let unit = width as f64 / composition.width;
        let height = ((composition.height * unit).round() as u32).max(1);
        let width = width.max(1);
        let l = layouts();
        let palette = Palette::new(prefs);
        let font_px = (l.font * unit) as f32;
        // Ticker chips are measured before drawing starts.
        // Ticker chips, measured before drawing starts: the newest at the right edge, older
        // ones to its left as far as they fit.
        let mut chips = Vec::new();
        if composition.ticker.is_some() {
            let pad = (0.3 * unit) as f32;
            let mut right = width as f32;
            for label in visual.ticker.iter().rev() {
                let w = self.text_width(label, font_px * 0.95)? + 2.0 * pad;
                if right - w < 0.0 {
                    break;
                }
                chips.push((label.clone(), right - w, w));
                right -= w + (0.15 * unit) as f32;
            }
        }
        let format = self.format(font_px)?;
        let small = self.format(font_px * 0.7)?;
        let ticker_format = self.format(font_px * 0.95)?;
        unsafe {
            let bitmap: IWICBitmap = self.wic.CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let target: ID2D1RenderTarget =
                self.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
            let brush = |c: [f32; 4]| -> Result<ID2D1SolidColorBrush> {
                let color = D2D1_COLOR_F {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                    a: c[3],
                };
                Ok(target.CreateSolidColorBrush(&color, None)?)
            };
            let fill = brush(palette.fill)?;
            let edge = brush(palette.edge)?;
            let label = brush(palette.label)?;
            let held = brush(palette.held)?;
            let held_label = brush(palette.held_label)?;
            let px = |v: f64| (v * unit) as f32;
            let rect = |x: f64, y: f64, w: f64, h: f64| D2D_RECT_F {
                left: px(x + l.gap / 2.0),
                top: px(y + l.gap / 2.0),
                right: px(x + w - l.gap / 2.0),
                bottom: px(y + h - l.gap / 2.0),
            };
            let radius = px(l.radius);
            let key_shape = |r: D2D_RECT_F, lit: bool, rounding: f32| {
                let rounded = D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: rounding,
                    radiusY: rounding,
                };
                target.FillRoundedRectangle(&rounded, if lit { &held } else { &fill });
                target.DrawRoundedRectangle(&rounded, &edge, px(0.03).max(1.0), None);
            };
            let text = |s: &str, r: D2D_RECT_F, lit: bool, f: &IDWriteTextFormat| {
                let wide: Vec<u16> = s.encode_utf16().collect();
                target.DrawText(
                    &wide,
                    f,
                    &r,
                    if lit { &held_label } else { &label },
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            };

            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F::default()));
            if let Some(keys) = composition.keys {
                for (k, lit) in keys.keys.iter().zip(&visual.keys) {
                    let r = rect(k.x, k.y, k.w, k.h);
                    key_shape(r, *lit, radius);
                    let f = if k.label.chars().count() > 2 {
                        &small
                    } else {
                        &format
                    };
                    text(&k.label, r, *lit, f);
                }
                let t = keys.text;
                let r = rect(t.x, t.y, t.w, t.h);
                key_shape(r, visual.text, (r.bottom - r.top) / 2.0);
                text("•", r, visual.text, &format);
            }
            if let Some((x0, m)) = composition.mouse {
                // As `drawMouse` in overlay.ts.
                let left = px(x0 + m.body.x);
                let right = px(x0 + m.body.x + m.body.w);
                let top = px(m.body.y);
                let bottom = px(m.body.y + m.body.h);
                let cx = (left + right) / 2.0;
                let rx = (right - left) / 2.0;
                let (rt, rb) = (px(m.top), px(m.bottom));
                let split = px(m.split);
                let line = px(0.035).max(1.0);
                let lit = |button: Button| visual.buttons & (1 << button as u8) != 0;
                let pt = |x: f32, y: f32| Vector2 { X: x, Y: y };
                let arc = |to: Vector2, ry: f32, clockwise: bool| D2D1_ARC_SEGMENT {
                    point: to,
                    size: D2D_SIZE_F {
                        width: rx,
                        height: ry,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: if clockwise {
                        D2D1_SWEEP_DIRECTION_CLOCKWISE
                    } else {
                        D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                    },
                    arcSize: D2D1_ARC_SIZE_SMALL,
                };
                let path = |build: &dyn Fn(&ID2D1GeometrySink)| -> Result<ID2D1PathGeometry> {
                    let geometry = self.d2d.CreatePathGeometry()?;
                    let sink = geometry.Open()?;
                    build(&sink);
                    sink.Close()?;
                    Ok(geometry)
                };
                let round = self.d2d.CreateStrokeStyle(
                    &D2D1_STROKE_STYLE_PROPERTIES {
                        startCap: D2D1_CAP_STYLE_ROUND,
                        endCap: D2D1_CAP_STYLE_ROUND,
                        dashCap: D2D1_CAP_STYLE_ROUND,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        miterLimit: 10.0,
                        dashStyle: D2D1_DASH_STYLE_SOLID,
                        dashOffset: 0.0,
                    },
                    None,
                )?;
                let slot = brush(palette.slot)?;
                let wheel = brush(palette.wheel)?;

                let body = path(&|sink| {
                    sink.BeginFigure(pt(left, top + rt), D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddArc(&arc(pt(cx, top), rt, true));
                    sink.AddArc(&arc(pt(right, top + rt), rt, true));
                    sink.AddLine(pt(right, bottom - rb));
                    sink.AddArc(&arc(pt(cx, bottom), rb, true));
                    sink.AddArc(&arc(pt(left, bottom - rb), rb, true));
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                })?;
                target.FillGeometry(&body, &fill, None);
                // A held button: its half of the top, up to the body's edge.
                for (button, side) in [(Button::Left, -1.0f32), (Button::Right, 1.0)] {
                    if !lit(button) {
                        continue;
                    }
                    let half = path(&|sink| {
                        sink.BeginFigure(pt(cx, split), D2D1_FIGURE_BEGIN_FILLED);
                        sink.AddLine(pt(cx, top));
                        sink.AddArc(&arc(pt(cx + side * rx, top + rt), rt, side > 0.0));
                        sink.AddLine(pt(cx + side * rx, split));
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    })?;
                    target.FillGeometry(&half, &held, None);
                }
                let w = D2D_RECT_F {
                    left: px(x0 + m.wheel.x),
                    top: px(m.wheel.y),
                    right: px(x0 + m.wheel.x + m.wheel.w),
                    bottom: px(m.wheel.y + m.wheel.h),
                };
                let pad = px(0.07);
                // The seams: across under the buttons, and between them, broken by the
                // wheel's slot.
                target.DrawLine(pt(left, split), pt(right, split), &edge, line, None);
                target.DrawLine(pt(cx, top), pt(cx, w.top - pad), &edge, line, None);
                target.DrawLine(pt(cx, w.bottom + pad), pt(cx, split), &edge, line, None);
                target.DrawGeometry(&body, &edge, line, None);

                let pill = |r: D2D_RECT_F, b: &ID2D1SolidColorBrush| {
                    let radius = (r.right - r.left) / 2.0;
                    target.FillRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect: r,
                            radiusX: radius,
                            radiusY: radius,
                        },
                        b,
                    );
                };
                pill(
                    D2D_RECT_F {
                        left: w.left - pad,
                        top: w.top - pad,
                        right: w.right + pad,
                        bottom: w.bottom + pad,
                    },
                    &slot,
                );
                pill(w, if lit(Button::Middle) { &held } else { &wheel });
                if visual.wheel != 0 {
                    // Up above the wheel, down below it, with a dark rim so it shows on a
                    // held button too.
                    let up = visual.wheel > 0;
                    let s = px(0.15);
                    let tip = if up {
                        w.top - pad - px(0.1) - s * 0.7
                    } else {
                        w.bottom + pad + px(0.1) + s * 0.7
                    };
                    let arms = if up { tip + s * 0.7 } else { tip - s * 0.7 };
                    let chevron = path(&|sink| {
                        sink.BeginFigure(pt(cx - s, arms), D2D1_FIGURE_BEGIN_HOLLOW);
                        sink.AddLine(pt(cx, tip));
                        sink.AddLine(pt(cx + s, arms));
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    })?;
                    target.DrawGeometry(&chevron, &slot, px(0.13), &round);
                    target.DrawGeometry(&chevron, &held, px(0.07), &round);
                }
                for (button, r) in [(Button::X1, m.x1), (Button::X2, m.x2)] {
                    let r = D2D_RECT_F {
                        left: px(x0 + r.x),
                        top: px(r.y),
                        right: px(x0 + r.x + r.w),
                        bottom: px(r.y + r.h),
                    };
                    pill(r, if lit(button) { &held } else { &wheel });
                }

                if prefs.movement {
                    let c = m.movement;
                    let center = pt(px(x0 + c.x), px(c.y));
                    let circle = |at: Vector2, r: f32| D2D1_ELLIPSE {
                        point: at,
                        radiusX: r,
                        radiusY: r,
                    };
                    target.DrawEllipse(&circle(center, px(c.r)), &edge, line, None);
                    let (dx, dy) = visual.movement;
                    if (dx, dy) == (0, 0) {
                        target.FillEllipse(&circle(center, px(0.08)), &wheel);
                    } else {
                        let end = pt(
                            center.X + px(c.r) * 0.78 * f32::from(dx) / 8.0,
                            center.Y + px(c.r) * 0.78 * f32::from(dy) / 8.0,
                        );
                        target.DrawLine(center, end, &held, px(0.07), &round);
                        target.FillEllipse(&circle(end, px(0.13)), &held);
                    }
                }
            }
            if let Some(y) = composition.ticker {
                for (s, x, w) in &chips {
                    let r = D2D_RECT_F {
                        left: *x,
                        top: px(y),
                        right: x + w,
                        bottom: px(y + l.ticker.height),
                    };
                    key_shape(r, false, radius);
                    text(s, r, false, &ticker_format);
                }
            }
            target.EndDraw(None, None)?;
            drop(target);

            let area = WICRect {
                X: 0,
                Y: 0,
                Width: width as i32,
                Height: height as i32,
            };
            let lock = bitmap.Lock(&area, WICBitmapLockRead.0 as u32)?;
            let stride = lock.GetStride()? as usize;
            let mut size = 0u32;
            let mut data = std::ptr::null_mut();
            lock.GetDataPointer(&mut size, &mut data)?;
            let source = std::slice::from_raw_parts(data, size as usize);
            let opacity = prefs.opacity.clamp(0.0, 1.0) as f32;
            let mut rgba = vec![0u8; (width * height * 4) as usize];
            for row in 0..height as usize {
                for col in 0..width as usize {
                    let p = &source[row * stride + col * 4..][..4];
                    let a = p[3];
                    let o = &mut rgba[(row * width as usize + col) * 4..][..4];
                    if a == 0 {
                        continue;
                    }
                    // Premultiplied BGRA to straight RGBA.
                    let un = |c: u8| {
                        ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8
                    };
                    o[0] = un(p[2]);
                    o[1] = un(p[1]);
                    o[2] = un(p[0]);
                    o[3] = (f32::from(a) * opacity).round() as u8;
                }
            }
            Ok(Picture {
                width,
                height,
                rgba,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputfile::Pressed;

    fn prefs(keys: OverlayKeys, mouse: bool, ticker: bool) -> InputOverlayPrefs {
        InputOverlayPrefs {
            keys,
            mouse,
            ticker,
            ..InputOverlayPrefs::default()
        }
    }

    const W: Key = Key {
        vk: 0x57,
        scan: 0x11,
        ext: false,
    };

    #[test]
    fn the_layouts_load_and_compose() {
        let l = layouts();
        assert!(l.wasd.keys.iter().any(|k| k.label == "W"));
        assert_eq!(l.keyboard.keys.len(), 61);
        let c = Composition::new(&prefs(OverlayKeys::Wasd, true, false)).unwrap();
        assert!((c.width - (l.wasd.width + l.spacing + l.mouse.width)).abs() < 1e-9);
        assert!(Composition::new(&prefs(OverlayKeys::None, false, false)).is_none());
        let ticker = Composition::new(&prefs(OverlayKeys::None, false, true)).unwrap();
        assert!(ticker.width >= 6.0 && ticker.height == l.ticker.height);
    }

    #[test]
    fn what_shows_follows_the_input() {
        let blocks = vec![Block {
            start: 0.0,
            pressed: Pressed::default(),
            events: vec![
                (
                    1.0,
                    Event::KeyDown(Key {
                        vk: 0xA2,
                        scan: 0x1D,
                        ext: false,
                    }),
                ),
                (
                    1.1,
                    Event::KeyDown(Key {
                        vk: 0x43,
                        scan: 0x2E,
                        ext: false,
                    }),
                ),
                (
                    1.2,
                    Event::KeyUp(Key {
                        vk: 0x43,
                        scan: 0x2E,
                        ext: false,
                    }),
                ),
                (
                    1.3,
                    Event::KeyUp(Key {
                        vk: 0xA2,
                        scan: 0x1D,
                        ext: false,
                    }),
                ),
                (2.0, Event::KeyDown(W)),
                (2.0, Event::TextDown),
                (2.05, Event::Move { dx: 400, dy: 0 }),
                (
                    2.06,
                    Event::Wheel {
                        vertical: -120,
                        horizontal: 0,
                    },
                ),
                (2.1, Event::ButtonDown(Button::Right)),
            ],
        }];
        let timeline = Timeline::new(blocks);
        let c = Composition::new(&prefs(OverlayKeys::Wasd, true, true)).unwrap();
        let v = timeline.visual(2.1, &c);
        let w_index = layouts()
            .wasd
            .keys
            .iter()
            .position(|k| k.label == "W")
            .unwrap();
        assert!(v.keys[w_index]);
        assert_eq!(v.keys.iter().filter(|k| **k).count(), 1);
        assert!(v.text);
        assert_eq!(v.buttons, 1 << Button::Right as u8);
        assert_eq!(v.wheel, -1);
        assert_eq!(v.movement, (8, 0));
        assert_eq!(v.ticker, vec!["Ctrl+C", "W", "•"]);
        // Later, the wheel and the movement have settled; the ticker has emptied.
        let later = timeline.visual(4.5, &c);
        assert_eq!((later.wheel, later.movement), (0, (0, 0)));
        assert!(later.ticker.is_empty());
        // A key without a scan code (sent by a program) matches by its virtual key.
        let blind = Timeline::new(vec![Block {
            start: 0.0,
            pressed: Pressed::default(),
            events: vec![(
                0.5,
                Event::KeyDown(Key {
                    vk: 0x57,
                    scan: 0,
                    ext: false,
                }),
            )],
        }]);
        assert!(blind.visual(0.6, &c).keys[w_index]);
    }

    #[test]
    fn a_picture_is_drawn_with_held_keys_in_the_highlight() {
        let c = Composition::new(&prefs(OverlayKeys::Wasd, true, true)).unwrap();
        let mut held = Visual {
            keys: vec![false; layouts().wasd.keys.len()],
            ticker: vec!["Ctrl+C".into(), "W".into()],
            ..Visual::default()
        };
        let w_index = layouts()
            .wasd
            .keys
            .iter()
            .position(|k| k.label == "W")
            .unwrap();
        held.keys[w_index] = true;
        let mut prefs = prefs(OverlayKeys::Wasd, true, true);
        prefs.highlight = "#ff0000".into();
        prefs.opacity = 1.0;
        let mut painter = Painter::new().unwrap();
        let picture = painter.paint(&c, &held, 400, &prefs).unwrap();
        assert_eq!(picture.width, 400);
        let unit = 400.0 / c.width;
        assert_eq!(picture.height, (c.height * unit).round() as u32);
        // The middle of W is red; the middle of S (not held) is dark.
        let k = &layouts().wasd.keys[w_index];
        let at = |x: f64, y: f64| {
            let i = (((y * unit) as usize) * picture.width as usize + (x * unit) as usize) * 4;
            &picture.rgba[i..i + 4]
        };
        let w = at(k.x + 0.2, k.y + 0.15);
        assert!(w[0] > 200 && w[1] < 60 && w[3] > 200, "{w:?}");
        let s = layouts().wasd.keys.iter().find(|k| k.label == "S").unwrap();
        let s = at(s.x + 0.2, s.y + 0.15);
        assert!(s[0] < 60 && s[3] > 150, "{s:?}");
        // Outside every key: clear.
        assert_eq!(at(0.05, 0.5)[3], 0);
    }

    /// The left button's half of the body, a side button and the arrow over the wheel light
    /// up; the right half, the wheel and the corners outside the rounded body do not.
    #[test]
    fn the_mouse_lights_what_is_held() {
        let mut prefs = prefs(OverlayKeys::None, true, false);
        prefs.highlight = "#ff0000".into();
        prefs.opacity = 1.0;
        let c = Composition::new(&prefs).unwrap();
        let visual = Visual {
            buttons: 1 << Button::Left as u8 | 1 << Button::X1 as u8,
            wheel: 1,
            movement: (8, 0),
            ..Visual::default()
        };
        let picture = Painter::new()
            .unwrap()
            .paint(&c, &visual, 260, &prefs)
            .unwrap();
        let unit = 260.0 / c.width;
        let at = |x: f64, y: f64| {
            let i = (((y * unit) as usize) * picture.width as usize + (x * unit) as usize) * 4;
            picture.rgba[i..i + 4].to_vec()
        };
        let red = |p: &[u8]| p[0] > 200 && p[1] < 60 && p[3] > 200;
        let m = &layouts().mouse;
        let (left, right) = (m.body.x + m.body.w * 0.25, m.body.x + m.body.w * 0.75);
        assert!(
            red(&at(left, m.split - 0.3)),
            "left held: {:?}",
            at(left, m.split - 0.3)
        );
        let unheld = at(right, m.split - 0.3);
        assert!(!red(&unheld) && unheld[3] > 150, "right not: {unheld:?}");
        assert!(
            red(&at(m.x1.x + m.x1.w / 2.0, m.x1.y + m.x1.h / 2.0)),
            "back held"
        );
        assert!(
            !red(&at(m.x2.x + m.x2.w / 2.0, m.x2.y + m.x2.h / 2.0)),
            "forward not"
        );
        let wheel = at(m.wheel.x + m.wheel.w / 2.0, m.wheel.y + m.wheel.h / 2.0);
        assert!(!red(&wheel), "no middle click: {wheel:?}");
        // The arrow's tip, above the wheel.
        let mid = m.body.x + m.body.w / 2.0;
        assert!(red(&at(mid, m.wheel.y - 0.07 - 0.1 - 0.105)), "up arrow");
        // The movement dot, to the right in the palm.
        let dot = at(m.movement.x + m.movement.r * 0.78, m.movement.y);
        assert!(red(&dot), "moving right: {dot:?}");
        // The rounded corners are outside the body.
        assert_eq!(at(m.body.x + 0.05, m.body.y + 0.05)[3], 0);
        assert_eq!(
            at(m.body.x + m.body.w - 0.05, m.body.y + m.body.h - 0.05)[3],
            0
        );
    }
}
