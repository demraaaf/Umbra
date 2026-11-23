//! Global hotkeys. The keyboard and mouse state is polled a few hundred times a second, which
//! works while a game runs in fullscreen, accepts mouse buttons, and needs no input hooks.
//!
//! Polling replaced low-level hooks because hooks installed inside the Tauri window process
//! were never called (they worked in a plain console program), and some anti-cheat systems
//! look at installed input hooks.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MBUTTON, VK_MENU, VK_RWIN,
    VK_SHIFT, VK_XBUTTON1, VK_XBUTTON2,
};

const CTRL: u8 = 1;
const ALT: u8 = 2;
const SHIFT: u8 = 4;
const WIN: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Trigger {
    /// Windows virtual-key code.
    Key(u32),
    /// 3 middle, 4 back, 5 forward.
    Mouse(u8),
}

/// One key or button with its modifiers, such as `Alt+F10` or `Mouse4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Combo {
    modifiers: u8,
    trigger: Trigger,
}

/// An action's bindings: one or more combos, any of which triggers it. Written as
/// `Mouse4, Mouse5` in the settings; a single combo is the common case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    combos: Vec<Combo>,
}

impl FromStr for Hotkey {
    type Err = anyhow::Error;

    /// Parses `Alt+F10`, `Ctrl+Shift+KeyG`, `Mouse4`, or several of them separated by
    /// commas. The key part is a `KeyboardEvent.code` value, so the settings screen can
    /// record it directly.
    fn from_str(text: &str) -> Result<Self> {
        let mut combos: Vec<Combo> = Vec::new();
        for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let combo: Combo = part.parse()?;
            if !combos.contains(&combo) {
                combos.push(combo);
            }
        }
        if combos.is_empty() {
            bail!("hotkey `{text}` has no key");
        }
        Ok(Self { combos })
    }
}

impl FromStr for Combo {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        let mut modifiers = 0;
        let mut trigger = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers |= CTRL,
                "alt" => modifiers |= ALT,
                "shift" => modifiers |= SHIFT,
                "win" | "meta" | "super" => modifiers |= WIN,
                _ => {
                    if trigger.is_some() {
                        bail!("hotkey `{text}` has more than one key");
                    }
                    trigger = Some(parse_trigger(part)?);
                }
            }
        }
        let trigger = trigger.ok_or_else(|| anyhow!("hotkey `{text}` has no key"))?;
        Ok(Self { modifiers, trigger })
    }
}

/// The virtual-key code of a key named as in the settings (`KeyY`, `Enter`, `F5`); `None`
/// for a mouse button or an unknown name.
pub fn key_code(name: &str) -> Option<u16> {
    match parse_trigger(name.trim()).ok()? {
        Trigger::Key(vk) => Some(vk as u16),
        Trigger::Mouse(_) => None,
    }
}

fn parse_trigger(name: &str) -> Result<Trigger> {
    let lower = name.to_ascii_lowercase();
    if let Some(button) = lower.strip_prefix("mouse") {
        return match button {
            "3" => Ok(Trigger::Mouse(3)),
            "4" => Ok(Trigger::Mouse(4)),
            "5" => Ok(Trigger::Mouse(5)),
            _ => bail!("mouse button `{name}` is not supported (use Mouse3, Mouse4 or Mouse5)"),
        };
    }
    if let Some(letter) = lower.strip_prefix("key") {
        if let [c @ b'a'..=b'z'] = letter.as_bytes() {
            return Ok(Trigger::Key(c.to_ascii_uppercase() as u32));
        }
    }
    if let Some(digit) = lower.strip_prefix("digit") {
        if let [c @ b'0'..=b'9'] = digit.as_bytes() {
            return Ok(Trigger::Key(*c as u32));
        }
    }
    if let Some(digit) = lower.strip_prefix("numpad") {
        if let [c @ b'0'..=b'9'] = digit.as_bytes() {
            return Ok(Trigger::Key(0x60 + (c - b'0') as u32));
        }
        let vk = match digit {
            "multiply" => 0x6A,
            "add" => 0x6B,
            "subtract" => 0x6D,
            "decimal" => 0x6E,
            "divide" => 0x6F,
            _ => bail!("unknown key `{name}`"),
        };
        return Ok(Trigger::Key(vk));
    }
    if let Some(number) = lower.strip_prefix('f') {
        if let Ok(n @ 1..=24) = number.parse::<u32>() {
            return Ok(Trigger::Key(0x70 + n - 1));
        }
    }
    let vk = match lower.as_str() {
        "space" => 0x20,
        "enter" | "numpadenter" => 0x0D,
        "escape" => 0x1B,
        "tab" => 0x09,
        "backspace" => 0x08,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "arrowleft" => 0x25,
        "arrowup" => 0x26,
        "arrowright" => 0x27,
        "arrowdown" => 0x28,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "pause" => 0x13,
        "scrolllock" => 0x91,
        "printscreen" => 0x2C,
        "backquote" => 0xC0,
        "minus" => 0xBD,
        "equal" => 0xBB,
        _ => bail!("unknown key `{name}`"),
    };
    Ok(Trigger::Key(vk))
}

struct PollState {
    hotkey: Hotkey,
    /// Per combo: its trigger is held; ignored until released so one press fires once.
    held: Vec<bool>,
}

fn key_down(vk: u16) -> bool {
    // The top bit is set while the key is down.
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

fn modifiers_down() -> u8 {
    let down = |vk: VIRTUAL_KEY| key_down(vk.0);
    let mut mask = 0;
    if down(VK_CONTROL) {
        mask |= CTRL;
    }
    if down(VK_MENU) {
        mask |= ALT;
    }
    if down(VK_SHIFT) {
        mask |= SHIFT;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        mask |= WIN;
    }
    mask
}

fn trigger_down(trigger: Trigger) -> bool {
    match trigger {
        Trigger::Key(vk) => key_down(vk as u16),
        Trigger::Mouse(3) => key_down(VK_MBUTTON.0),
        Trigger::Mouse(4) => key_down(VK_XBUTTON1.0),
        Trigger::Mouse(_) => key_down(VK_XBUTTON2.0),
    }
}

pub struct HotkeyListener {
    state: Arc<Mutex<PollState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HotkeyListener {
    /// Starts listening. `on_press` runs on the polling thread, so it should only signal
    /// another thread.
    pub fn start(hotkey: Hotkey, on_press: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        let held = vec![false; hotkey.combos.len()];
        let state = Arc::new(Mutex::new(PollState { hotkey, held }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = state.clone();
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("umbra-hotkey".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    {
                        let mut state = thread_state.lock().unwrap();
                        let state = &mut *state;
                        let mut fire = false;
                        for (combo, held) in state.hotkey.combos.iter().zip(state.held.iter_mut()) {
                            if !trigger_down(combo.trigger) {
                                *held = false;
                            } else if !*held {
                                // Latch even when the modifiers are wrong, so holding the key
                                // and then adding a modifier does not fire.
                                *held = true;
                                fire |= modifiers_down() == combo.modifiers;
                            }
                        }
                        // Two bindings pressed together still fire once.
                        if fire {
                            on_press();
                        }
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
            })
            .context("cannot start hotkey thread")?;
        Ok(Self {
            state,
            stop,
            thread: Some(thread),
        })
    }

    /// Switches to another hotkey.
    pub fn set_hotkey(&self, hotkey: Hotkey) {
        let mut state = self.state.lock().unwrap();
        // Whatever is held right now belongs to the old keys.
        state.held = vec![true; hotkey.combos.len()];
        state.hotkey = hotkey;
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Keeps a flag true while a key is held, for push-to-talk. Extra modifiers are allowed, so
/// talking while also holding Shift or Ctrl in a game still works.
pub struct KeyHoldWatcher {
    key: Arc<Mutex<Hotkey>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl KeyHoldWatcher {
    pub fn start(hotkey: Hotkey, held: Arc<AtomicBool>) -> Result<Self> {
        let key = Arc::new(Mutex::new(hotkey));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_key = key.clone();
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("umbra-push-to-talk".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    let down = {
                        let key = thread_key.lock().unwrap();
                        let modifiers = modifiers_down();
                        key.combos.iter().any(|c| {
                            trigger_down(c.trigger) && modifiers & c.modifiers == c.modifiers
                        })
                    };
                    held.store(down, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(4));
                }
                held.store(false, Ordering::Relaxed);
            })
            .context("cannot start push-to-talk thread")?;
        Ok(Self {
            key,
            stop,
            thread: Some(thread),
        })
    }

    pub fn set_key(&self, hotkey: Hotkey) {
        *self.key.lock().unwrap() = hotkey;
    }
}

impl Drop for KeyHoldWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Types the hotkey's first binding on the keyboard, for self-tests. Only works for keyboard
/// hotkeys.
pub fn inject(hotkey: &Hotkey) -> Result<()> {
    let hotkey = hotkey.combos[0];
    let Trigger::Key(vk) = hotkey.trigger else {
        bail!("only keyboard hotkeys can be injected");
    };
    let mut keys = Vec::new();
    for (mask, key) in [
        (CTRL, VK_CONTROL),
        (ALT, VK_MENU),
        (SHIFT, VK_SHIFT),
        (WIN, VK_LWIN),
    ] {
        if hotkey.modifiers & mask != 0 {
            keys.push(key);
        }
    }
    keys.push(VIRTUAL_KEY(vk as u16));

    let event = |key: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let downs: Vec<INPUT> = keys.iter().map(|&k| event(k, false)).collect();
    let ups: Vec<INPUT> = keys.iter().rev().map(|&k| event(k, true)).collect();
    // Held for a moment, like a real key press, so the poller can see it.
    for (batch, pause) in [(&downs, Some(Duration::from_millis(60))), (&ups, None)] {
        let sent = unsafe { SendInput(batch, size_of::<INPUT>() as i32) };
        if sent as usize != batch.len() {
            bail!("SendInput sent {sent} of {} events", batch.len());
        }
        if let Some(pause) = pause {
            std::thread::sleep(pause);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Combo {
        let hotkey: Hotkey = text.parse().unwrap();
        assert_eq!(hotkey.combos.len(), 1);
        hotkey.combos[0]
    }

    #[test]
    fn parses_keys_and_modifiers() {
        let h = one("Alt+F10");
        assert_eq!(h.modifiers, ALT);
        assert_eq!(h.trigger, Trigger::Key(0x79));

        let h = one("Ctrl+Shift+KeyG");
        assert_eq!(h.modifiers, CTRL | SHIFT);
        assert_eq!(h.trigger, Trigger::Key(b'G' as u32));

        assert_eq!(one("Mouse4").trigger, Trigger::Mouse(4));
        assert_eq!(one("F13").trigger, Trigger::Key(0x7C));
        assert_eq!(one("Numpad5").trigger, Trigger::Key(0x65));
        assert_eq!(one("Digit1").trigger, Trigger::Key(b'1' as u32));
    }

    #[test]
    fn several_bindings() {
        let h: Hotkey = "Mouse4, Mouse5".parse().unwrap();
        assert_eq!(h.combos.len(), 2);
        assert_eq!(h.combos[1].trigger, Trigger::Mouse(5));
        // The same binding twice counts once; blank parts are ignored.
        let h: Hotkey = "Alt+F10,alt+f10, ".parse().unwrap();
        assert_eq!(h.combos.len(), 1);
        // One bad binding makes the whole setting invalid, so it falls back to the default.
        assert!("Mouse4, Nonsense".parse::<Hotkey>().is_err());
        assert!(", ".parse::<Hotkey>().is_err());
    }

    #[test]
    fn rejects_bad_hotkeys() {
        assert!("Alt".parse::<Hotkey>().is_err());
        assert!("F1+F2".parse::<Hotkey>().is_err());
        assert!("Mouse1".parse::<Hotkey>().is_err());
        assert!("F25".parse::<Hotkey>().is_err());
        assert!("Nonsense".parse::<Hotkey>().is_err());
    }
}
