//! Keyboard and mouse input for the overlay. Raw Input, registered as an input sink on a
//! message-only window of its own thread: no hook, nothing injected, nothing polled. The
//! program in front comes from a WinEvent hook. Input waits `DELAY` before the rules
//! (`inputrules`) decide what is kept, so a switch between programs, or a password field the
//! guard (`inputguard`) tells of a moment late, still applies to it. What is kept goes out
//! in blocks to the recorders, whose replay buffers hold it like the video and audio.
//!
//! Raw Input is one registration per process and kind of device, and Tauri's window layer
//! registers too when it starts; Umbra uses none of its device events, so the capture takes
//! it over, and checks every few seconds that it still has it.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRegisteredRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
    RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    EnumChildWindows, GetForegroundWindow, GetWindowThreadProcessId, HWND_MESSAGE, MSG,
    MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RegisterClassW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT, WM_INPUT, WNDCLASSW,
};
use windows::core::{BOOL, w};

use crate::inputfile::{Block, Button, Event, Key, Pressed};
use crate::inputguard::Guard;
use crate::inputrules::{
    App, Context as RuleContext, FocusHistory, GuardHistory, Raw, context_at, keep_for, rule_for,
};
use crate::settings::{InputRule, KeyboardMode};

/// How long input waits before the rules decide on it.
pub const DELAY: f64 = 0.3;
/// How often the capture hands out what it decided.
const TICK: Duration = Duration::from_millis(50);
/// Mouse movement is summed over this long.
const BUCKET: f64 = 0.008;
/// While something is held, a block goes out at least this often, so a clip starting
/// anywhere knows what is held.
const HEARTBEAT: f64 = 0.25;
/// Input waiting for the rules, at most; movement goes first when there is more.
const MAX_PENDING: usize = 20_000;

/// What the capture is doing, for the settings and the recording panel.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputStatus {
    pub on: bool,
    /// The program in front, such as `chrome.exe`.
    pub app: Option<String>,
    /// The rule that applies to it; `None` when nothing is kept there (or for Umbra).
    pub rule: Option<String>,
    /// `password` while a password field has focus, `chat` while a game's chat is open.
    pub guard: Option<String>,
}

type Sink = Arc<dyn Fn(&Block) + Send + Sync>;

struct Shared {
    rules: Mutex<Vec<InputRule>>,
    /// The game Umbra detected; its rule is the "Games" one.
    game_pid: Arc<AtomicU32>,
    sinks: Mutex<Vec<(u64, Sink)>>,
    status: Mutex<InputStatus>,
    /// Input up to this QPC time (100 ns) has been handed out.
    committed: AtomicI64,
    stop: AtomicBool,
}

/// Keyboard and mouse input for every recorder that subscribes. Captures only while on.
pub struct InputHub {
    shared: Arc<Shared>,
    next: AtomicU64,
    thread: Mutex<Option<JoinHandle<()>>>,
}

/// Unsubscribes when dropped.
pub struct Subscription {
    shared: Arc<Shared>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.shared
            .sinks
            .lock()
            .unwrap()
            .retain(|(id, _)| *id != self.id);
    }
}

impl InputHub {
    pub fn new(game_pid: Arc<AtomicU32>) -> Self {
        Self {
            shared: Arc::new(Shared {
                rules: Mutex::new(Vec::new()),
                game_pid,
                sinks: Mutex::new(Vec::new()),
                status: Mutex::new(InputStatus::default()),
                committed: AtomicI64::new(0),
                stop: AtomicBool::new(false),
            }),
            next: AtomicU64::new(1),
            thread: Mutex::new(None),
        }
    }

    /// `sink` gets every block kept, its times in QPC seconds.
    pub fn subscribe(&self, sink: impl Fn(&Block) + Send + Sync + 'static) -> Subscription {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.shared.sinks.lock().unwrap().push((id, Arc::new(sink)));
        Subscription {
            shared: self.shared.clone(),
            id,
        }
    }

    /// Captures (or stops) with these rules; the rules change at once while it runs.
    pub fn set(&self, on: bool, rules: Vec<InputRule>) {
        *self.shared.rules.lock().unwrap() = rules;
        let mut thread = self.thread.lock().unwrap();
        let running = thread.as_ref().is_some_and(|t| !t.is_finished());
        if on && !running {
            self.shared.stop.store(false, Ordering::Relaxed);
            let shared = self.shared.clone();
            match std::thread::Builder::new()
                .name("umbra-input".into())
                .spawn(move || {
                    if let Err(err) = run(&shared) {
                        log::warn!("input capture stopped: {err:#}");
                    }
                    *shared.status.lock().unwrap() = InputStatus::default();
                }) {
                Ok(handle) => *thread = Some(handle),
                Err(err) => log::warn!("cannot start input capture: {err}"),
            }
        } else if !on && running {
            self.shared.stop.store(true, Ordering::Relaxed);
            if let Some(handle) = thread.take() {
                let _ = handle.join();
            }
        }
    }

    pub fn status(&self) -> InputStatus {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn is_on(&self) -> bool {
        self.thread
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|t| !t.is_finished())
    }

    /// Waits (up to `most`) until input up to now has reached the recorders, so a clip
    /// saved now has it to its end.
    pub fn wait_handed_out(&self, most: Duration) {
        if !self.is_on() {
            return;
        }
        let now = crate::clock::qpc_100ns();
        let deadline = Instant::now() + most;
        while self.shared.committed.load(Ordering::Relaxed) < now && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl std::fmt::Debug for InputHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputHub")
    }
}

impl Drop for InputHub {
    fn drop(&mut self) {
        self.set(false, Vec::new());
    }
}

/// Collects what the rules kept into blocks: mouse movement summed over `BUCKET`, each block
/// starting with what was held then.
#[derive(Default)]
pub struct BlockBuilder {
    pressed: Pressed,
    open: Option<Block>,
    last_out: f64,
}

impl BlockBuilder {
    pub fn push(&mut self, t: f64, event: Event) {
        let block = self.open.get_or_insert_with(|| Block {
            start: t,
            pressed: self.pressed.clone(),
            events: Vec::new(),
        });
        self.pressed.apply(&event);
        if let (Event::Move { dx, dy }, Some((at, Event::Move { dx: sx, dy: sy }))) =
            (event, block.events.last_mut())
        {
            if t - *at < BUCKET {
                *sx += dx;
                *sy += dy;
                return;
            }
        }
        block.events.push((t, event));
    }

    /// The block to hand out at `now`: what came since the last, or what is held when
    /// nothing came for `HEARTBEAT`.
    pub fn take(&mut self, now: f64) -> Option<Block> {
        if let Some(block) = self.open.take() {
            self.last_out = now;
            return Some(block);
        }
        if !self.pressed.is_empty() && now - self.last_out >= HEARTBEAT {
            self.last_out = now;
            return Some(Block {
                start: now,
                pressed: self.pressed.clone(),
                events: Vec::new(),
            });
        }
        None
    }
}

/// What the capture thread works with; its window procedure and WinEvent callback reach it
/// through here.
struct Capture {
    shared: Arc<Shared>,
    hwnd: HWND,
    pending: VecDeque<(f64, Raw)>,
    focus: FocusHistory,
    guard_said: GuardHistory,
    guard_rx: Receiver<(f64, crate::inputrules::GuardSays)>,
    guard: Option<Guard>,
    filter: crate::inputrules::Filter,
    builder: BlockBuilder,
    last_front: f64,
    last_registered: f64,
}

thread_local! {
    static CAPTURE: RefCell<Option<Capture>> = const { RefCell::new(None) };
}

fn run(shared: &Arc<Shared>) -> Result<()> {
    let (tx, guard_rx) = channel();
    let guard = match Guard::start(tx) {
        Ok(guard) => Some(guard),
        Err(err) => {
            // Without a guard, keys are kept only where a rule keeps them in full.
            log::warn!("{err:#}");
            None
        }
    };
    let hwnd = unsafe { create_window()? };
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    let now = crate::clock::qpc_secs();
    let mut capture = Capture {
        shared: shared.clone(),
        hwnd,
        pending: VecDeque::new(),
        focus: FocusHistory::default(),
        guard_said: GuardHistory::default(),
        guard_rx,
        guard,
        filter: Default::default(),
        builder: BlockBuilder::default(),
        last_front: now,
        last_registered: now,
    };
    capture.focus.front(now, front_app(&shared.game_pid));
    if let Err(err) = unsafe { register(hwnd, false) } {
        unsafe {
            let _ = UnhookWinEvent(hook);
            let _ = DestroyWindow(hwnd);
        }
        return Err(err);
    }
    log::info!("input capture started");
    CAPTURE.with(|c| *c.borrow_mut() = Some(capture));

    while !shared.stop.load(Ordering::Relaxed) {
        unsafe {
            MsgWaitForMultipleObjects(None, false, TICK.as_millis() as u32, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
        }
        CAPTURE.with(|c| {
            if let Some(capture) = c.borrow_mut().as_mut() {
                capture.tick(crate::clock::qpc_secs(), false);
            }
        });
    }

    let capture = CAPTURE.with(|c| c.borrow_mut().take());
    if let Some(mut capture) = capture {
        capture.tick(crate::clock::qpc_secs(), true);
    }
    unsafe {
        let _ = UnhookWinEvent(hook);
        let _ = register(hwnd, true);
        let _ = DestroyWindow(hwnd);
    }
    log::info!("input capture stopped");
    Ok(())
}

impl Capture {
    fn tick(&mut self, now: f64, last: bool) {
        for (at, says) in self.guard_rx.try_iter() {
            self.guard_said.push(at, says);
        }
        // The WinEvent hook times switches; this catches any it missed, and a game found
        // while its window was already in front.
        if now - self.last_front >= 0.25 {
            self.last_front = now;
            self.focus.front(now, front_app(&self.shared.game_pid));
        }
        if now - self.last_registered >= 2.0 {
            self.last_registered = now;
            unsafe { self.keep_registration() };
        }
        let rules = self.shared.rules.lock().unwrap().clone();
        let front = self.focus.at(now).map(|(_, app, _)| app.clone());
        if let Some(guard) = &self.guard {
            let keeps_keys = front
                .as_ref()
                .is_some_and(|app| keep_for(&rules, app).0.keyboard != KeyboardMode::Drop);
            guard.want(keeps_keys);
        }

        // Decide on what has waited long enough (everything, when stopping).
        let cutoff = if last { f64::INFINITY } else { now - DELAY };
        let mut decided: Option<RuleContext> = None;
        while self.pending.front().is_some_and(|(t, _)| *t <= cutoff) {
            let (t, raw) = self.pending.pop_front().unwrap();
            let context = context_at(&rules, &self.focus, &self.guard_said, t);
            if let Some(event) = self.filter.apply(raw, &context) {
                self.builder.push(t, event);
            }
            decided = Some(context);
        }
        let handed = if last { now } else { cutoff };
        if last {
            for event in self.filter.release_all() {
                self.builder.push(now, event);
            }
        }
        if let Some(block) = self.builder.take(handed) {
            let sinks: Vec<Sink> = self
                .shared
                .sinks
                .lock()
                .unwrap()
                .iter()
                .map(|(_, s)| s.clone())
                .collect();
            for sink in sinks {
                sink(&block);
            }
        }
        self.shared
            .committed
            .store((handed.min(now) * 1e7) as i64, Ordering::Relaxed);
        self.focus.forget_before(handed - 1.0);
        self.guard_said.forget_before(handed - 1.0);

        // What applies now, for the settings' live view.
        let guard = if self.filter.in_chat() {
            Some("chat".to_string())
        } else if decided.is_some_and(|c| c.password)
            || context_at(&rules, &self.focus, &self.guard_said, now).password
        {
            Some("password".to_string())
        } else {
            None
        };
        let status = InputStatus {
            on: !last,
            app: front
                .as_ref()
                .map(|a| a.exe.clone())
                .filter(|e| !e.is_empty()),
            rule: front
                .as_ref()
                .and_then(|app| rule_for(&rules, app))
                .map(|(_, rule)| rule.name.clone()),
            guard,
        };
        let mut shared = self.shared.status.lock().unwrap();
        if *shared != status {
            *shared = status;
        }
    }

    unsafe fn keep_registration(&self) {
        let mut devices = [RAWINPUTDEVICE::default(); 8];
        let mut count = devices.len() as u32;
        let found = unsafe {
            GetRegisteredRawInputDevices(
                Some(devices.as_mut_ptr()),
                &mut count,
                size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        let ours = found != u32::MAX
            && [2u16, 6].iter().all(|usage| {
                devices[..found as usize]
                    .iter()
                    .any(|d| d.usUsagePage == 1 && d.usUsage == *usage && d.hwndTarget == self.hwnd)
            });
        if !ours {
            log::info!("input capture registered again (something else took Raw Input)");
            if let Err(err) = unsafe { register(self.hwnd, false) } {
                log::warn!("cannot register for Raw Input: {err:#}");
            }
        }
    }

    fn add(&mut self, t: f64, raw: Raw) {
        if let (Raw::Move { dx, dy }, Some((at, Raw::Move { dx: sx, dy: sy }))) =
            (raw, self.pending.back_mut())
        {
            if t - *at < BUCKET {
                *sx += dx;
                *sy += dy;
                return;
            }
        }
        if self.pending.len() >= MAX_PENDING {
            if matches!(raw, Raw::Move { .. }) || self.pending.len() >= 2 * MAX_PENDING {
                return;
            }
        }
        self.pending.push_back((t, raw));
    }
}

unsafe fn create_window() -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("UmbraInputCapture");
        // A second capture in the same process finds it registered already.
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .context("cannot make the input window")?;
        Ok(hwnd)
    }
}

/// Mice and keyboards, delivered to `hwnd` whatever is in front; or none (`remove`).
unsafe fn register(hwnd: HWND, remove: bool) -> Result<()> {
    let device = |usage| RAWINPUTDEVICE {
        usUsagePage: 1,
        usUsage: usage,
        dwFlags: if remove {
            RIDEV_REMOVE
        } else {
            RIDEV_INPUTSINK
        },
        hwndTarget: if remove { HWND::default() } else { hwnd },
    };
    unsafe {
        RegisterRawInputDevices(&[device(2), device(6)], size_of::<RAWINPUTDEVICE>() as u32)
            .context("cannot register for Raw Input")
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_INPUT {
        let t = crate::clock::qpc_secs();
        if let Some(raw) = unsafe { read_raw(lparam) } {
            CAPTURE.with(|c| {
                if let Ok(mut slot) = c.try_borrow_mut() {
                    if let Some(capture) = slot.as_mut() {
                        for r in raw {
                            capture.add(t, r);
                        }
                    }
                }
            });
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let t = crate::clock::qpc_secs();
    CAPTURE.with(|c| {
        if let Ok(mut slot) = c.try_borrow_mut() {
            if let Some(capture) = slot.as_mut() {
                let app = front_app(&capture.shared.game_pid);
                capture.focus.front(t, app);
            }
        }
    });
}

/// The input in a `WM_INPUT` message: a key, or what a mouse report says (buttons, wheel and
/// movement can come together).
unsafe fn read_raw(lparam: LPARAM) -> Option<Vec<Raw>> {
    let mut data = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut c_void),
            RID_INPUT,
            Some(&mut data as *mut RAWINPUT as *mut c_void),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == u32::MAX || read == 0 {
        return None;
    }
    let mut out = Vec::new();
    if data.header.dwType == RIM_TYPEKEYBOARD.0 {
        let k = unsafe { data.data.keyboard };
        // 255 is a filler some keys send around their real code.
        if k.VKey == 0xFF {
            return None;
        }
        let ext = k.Flags & 2 != 0;
        out.push(Raw::Key {
            key: Key {
                vk: sided(k.VKey, k.MakeCode, ext),
                scan: k.MakeCode,
                ext,
            },
            down: k.Flags & 1 == 0,
        });
    } else if data.header.dwType == RIM_TYPEMOUSE.0 {
        let m = unsafe { data.data.mouse };
        let (flags, value) = unsafe {
            (
                m.Anonymous.Anonymous.usButtonFlags,
                m.Anonymous.Anonymous.usButtonData as i16 as i32,
            )
        };
        for (i, button) in Button::ALL.into_iter().enumerate() {
            if flags & (1 << (2 * i)) != 0 {
                out.push(Raw::Button { button, down: true });
            }
            if flags & (2 << (2 * i)) != 0 {
                out.push(Raw::Button {
                    button,
                    down: false,
                });
            }
        }
        if flags & 0x400 != 0 {
            out.push(Raw::Wheel {
                vertical: value,
                horizontal: 0,
            });
        }
        if flags & 0x800 != 0 {
            out.push(Raw::Wheel {
                vertical: 0,
                horizontal: value,
            });
        }
        // Absolute positions (a tablet, remote desktop) say nothing about a mouse's moves.
        if m.usFlags.0 & 1 == 0 && (m.lLastX != 0 || m.lLastY != 0) {
            out.push(Raw::Move {
                dx: m.lLastX,
                dy: m.lLastY,
            });
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Raw Input names Shift, Ctrl and Alt without a side; the side matters (right Alt is AltGr).
fn sided(vk: u16, scan: u16, ext: bool) -> u16 {
    match vk {
        0x10 if scan == 0x36 => 0xA1,
        0x10 => 0xA0,
        0x11 if ext => 0xA3,
        0x11 => 0xA2,
        0x12 if ext => 0xA5,
        0x12 => 0xA4,
        _ => vk,
    }
}

/// The program in front. A Store app's frame belongs to ApplicationFrameHost; the app itself
/// is the child window of another process.
fn front_app(game_pid: &AtomicU32) -> App {
    let window = unsafe { GetForegroundWindow() };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    let mut path = crate::games::image_path(pid);
    let exe_of = |path: &Option<String>| {
        path.as_deref()
            .and_then(|p| p.rsplit('\\').next())
            .unwrap_or("")
            .to_string()
    };
    if exe_of(&path).eq_ignore_ascii_case("ApplicationFrameHost.exe") {
        struct Search {
            host: u32,
            found: u32,
        }
        unsafe extern "system" fn child(window: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
            if pid != 0 && pid != search.host {
                search.found = pid;
                return false.into();
            }
            true.into()
        }
        let mut search = Search {
            host: pid,
            found: 0,
        };
        unsafe {
            let _ = EnumChildWindows(
                Some(window),
                Some(child),
                LPARAM(&mut search as *mut Search as isize),
            );
        }
        if search.found != 0 {
            pid = search.found;
            path = crate::games::image_path(pid);
        }
    }
    let game = game_pid.load(Ordering::Relaxed);
    App {
        pid,
        exe: exe_of(&path),
        path,
        game: pid != 0 && pid == game,
        umbra: pid == std::process::id(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_is_summed_and_blocks_start_with_what_is_held() {
        let mut b = BlockBuilder::default();
        b.push(1.000, Event::ButtonDown(Button::Left));
        b.push(1.001, Event::Move { dx: 2, dy: 1 });
        b.push(1.004, Event::Move { dx: 3, dy: -1 });
        b.push(1.010, Event::Move { dx: 1, dy: 0 });
        let block = b.take(1.05).unwrap();
        assert_eq!(block.start, 1.0);
        assert!(block.pressed.is_empty());
        assert_eq!(
            block.events,
            vec![
                (1.000, Event::ButtonDown(Button::Left)),
                (1.001, Event::Move { dx: 5, dy: 0 }),
                (1.010, Event::Move { dx: 1, dy: 0 }),
            ]
        );
        // Nothing new, button still held: a block saying so, at most every heartbeat.
        assert!(b.take(1.1).is_none());
        let beat = b.take(1.31).unwrap();
        assert!(beat.events.is_empty() && beat.pressed.holds(Button::Left));
        b.push(1.4, Event::ButtonUp(Button::Left));
        let block = b.take(1.45).unwrap();
        assert!(block.pressed.holds(Button::Left));
        // Nothing held, nothing new: nothing at all.
        assert!(b.take(5.0).is_none());
    }

    #[test]
    fn sides_of_modifiers_are_told_apart() {
        assert_eq!(sided(0x10, 0x2A, false), 0xA0);
        assert_eq!(sided(0x10, 0x36, false), 0xA1);
        assert_eq!(sided(0x11, 0x1D, true), 0xA3);
        assert_eq!(sided(0x12, 0x38, true), 0xA5);
        assert_eq!(sided(0x41, 0x1E, false), 0x41);
    }
}
