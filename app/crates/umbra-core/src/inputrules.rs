//! What of the keyboard and mouse is kept, decided before anything is written: the rules per
//! program (first match wins), the stricter of two rules right at a switch between programs,
//! the password guard and the chat guard. Everything here is plain logic over times, so it is
//! tested without a keyboard; `input` feeds it.

use crate::inputfile::{Button, Event, Key};
use crate::settings::{InputRule, KeyboardMode, MouseMode, RuleMatch};

/// Around a switch between programs, input counts as both's: the stricter rule applies.
pub const SWITCH_WINDOW: f64 = 0.05;
/// The password guard learns of a focused field a little after it has focus; a password
/// field's guard applies from this long before it was told.
pub const GUARD_LEAD: f64 = 0.25;

/// The program in front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct App {
    pub pid: u32,
    /// File name, such as `chrome.exe`.
    pub exe: String,
    pub path: Option<String>,
    /// The game Umbra detected.
    pub game: bool,
    /// Umbra itself: nothing is kept while it is in front.
    pub umbra: bool,
}

/// What is kept of input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub keyboard: KeyboardMode,
    pub mouse: MouseMode,
}

impl Keep {
    pub const NOTHING: Keep = Keep {
        keyboard: KeyboardMode::Drop,
        mouse: MouseMode::Drop,
    };

    /// The stricter of both, for keyboard and mouse each.
    pub fn stricter(self, other: Keep) -> Keep {
        let keyboard = |m: KeyboardMode| match m {
            KeyboardMode::Full => 0,
            KeyboardMode::Redacted => 1,
            KeyboardMode::Drop => 2,
        };
        let mouse = |m: MouseMode| match m {
            MouseMode::Full => 0,
            MouseMode::ButtonsOnly => 1,
            MouseMode::Drop => 2,
        };
        Keep {
            keyboard: if keyboard(other.keyboard) > keyboard(self.keyboard) {
                other.keyboard
            } else {
                self.keyboard
            },
            mouse: if mouse(other.mouse) > mouse(self.mouse) {
                other.mouse
            } else {
                self.mouse
            },
        }
    }
}

/// The rule for `app`: the first that matches, with its place in the list. `None` for Umbra
/// itself and for a program no rule covers; both keep nothing.
pub fn rule_for<'a>(rules: &'a [InputRule], app: &App) -> Option<(usize, &'a InputRule)> {
    if app.umbra {
        return None;
    }
    rules
        .iter()
        .enumerate()
        .find(|(_, rule)| match rule.matches {
            RuleMatch::Any => true,
            RuleMatch::Game => app.game,
            RuleMatch::Apps => rule
                .apps
                .iter()
                .any(|entry| crate::apps::matches(entry, &app.exe, app.path.as_deref())),
        })
}

/// What is kept while `app` is in front, and the keys that open a chat there.
pub fn keep_for(rules: &[InputRule], app: &App) -> (Keep, Vec<u16>) {
    match rule_for(rules, app) {
        Some((_, rule)) => (
            Keep {
                keyboard: rule.keyboard,
                mouse: rule.mouse,
            },
            rule.chat_keys
                .iter()
                .filter_map(|name| crate::hotkey::key_code(name))
                .collect(),
        ),
        None => (Keep::NOTHING, Vec::new()),
    }
}

/// Keys that keep their name under a redacting rule: they type no text.
pub fn is_command_key(vk: u16) -> bool {
    matches!(vk,
        // Shift, Ctrl, Alt (either side), Caps Lock, Windows keys.
        0x10..=0x12 | 0xA0..=0xA5 | 0x14 | 0x5B | 0x5C
        // F1 to F24.
        | 0x70..=0x87
        // Arrows; Esc, Tab, Enter, Backspace, Delete; Home, End, Page Up, Page Down.
        | 0x25..=0x28 | 0x1B | 0x09 | 0x0D | 0x08 | 0x2E | 0x21..=0x24
        // Media and volume keys.
        | 0xAD..=0xB3)
}

fn is_ctrl(vk: u16) -> bool {
    matches!(vk, 0x11 | 0xA2 | 0xA3)
}

fn is_alt(vk: u16) -> bool {
    matches!(vk, 0x12 | 0xA4 | 0xA5)
}

fn is_win(vk: u16) -> bool {
    matches!(vk, 0x5B | 0x5C)
}

const ENTER: u16 = 0x0D;
const ESCAPE: u16 = 0x1B;

/// Input as it comes from the devices, before any rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raw {
    Key { key: Key, down: bool },
    Button { button: Button, down: bool },
    Wheel { vertical: i32, horizontal: i32 },
    Move { dx: i32, dy: i32 },
}

/// Where input is going at one moment, as far as the rules care.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub keep: Keep,
    /// Keys that open a chat in the program in front (`InputRule::chat_keys`).
    pub chat_keys: Vec<u16>,
    /// Changes whenever another program comes to the front; a chat ends with it.
    pub focus: u64,
    /// A password field has focus, or the guard could not tell: no key at all is kept.
    pub password: bool,
}

/// How a key that went down was kept, so its release is kept the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kept {
    Named,
    Text,
}

/// Applies the rules to input, one event at a time, oldest first. Remembers what it kept
/// down, so a release matches its press however the rules changed in between (a key held
/// across a switch is never left down), and what is physically down, so a key held under a
/// rule that kept nothing is not picked up half way by its repeats.
#[derive(Default)]
pub struct Filter {
    /// Keys down on the keyboard, kept or not.
    physical: Vec<Key>,
    kept: Vec<(Key, Kept)>,
    buttons: u8,
    chat_open: bool,
    focus: u64,
}

impl Filter {
    pub fn apply(&mut self, raw: Raw, context: &Context) -> Option<Event> {
        if context.focus != self.focus {
            self.focus = context.focus;
            self.chat_open = false;
        }
        match raw {
            Raw::Key { key, down: true } => {
                if self.physical.contains(&key) {
                    // A repeat while held.
                    return None;
                }
                let combo = self.combo();
                self.physical.push(key);
                let event = self.press(key, combo, context)?;
                self.kept.push((
                    key,
                    if event == Event::TextDown {
                        Kept::Text
                    } else {
                        Kept::Named
                    },
                ));
                Some(event)
            }
            Raw::Key { key, down: false } => {
                self.physical.retain(|k| *k != key);
                let at = self.kept.iter().position(|(k, _)| *k == key)?;
                Some(match self.kept.remove(at).1 {
                    Kept::Named => Event::KeyUp(key),
                    Kept::Text => Event::TextUp,
                })
            }
            Raw::Button { button, down } => {
                let bit = 1u8 << button as u8;
                if down {
                    if context.keep.mouse == MouseMode::Drop || self.buttons & bit != 0 {
                        return None;
                    }
                    self.buttons |= bit;
                    Some(Event::ButtonDown(button))
                } else if self.buttons & bit != 0 {
                    self.buttons &= !bit;
                    Some(Event::ButtonUp(button))
                } else {
                    None
                }
            }
            Raw::Wheel {
                vertical,
                horizontal,
            } => (context.keep.mouse != MouseMode::Drop).then_some(Event::Wheel {
                vertical,
                horizontal,
            }),
            Raw::Move { dx, dy } => {
                (context.keep.mouse == MouseMode::Full).then_some(Event::Move { dx, dy })
            }
        }
    }

    /// A game's chat is open: keys typed are kept as for a redacting rule.
    pub fn in_chat(&self) -> bool {
        self.chat_open
    }

    /// Releases of everything kept down, for when keeping input stops.
    pub fn release_all(&mut self) -> Vec<Event> {
        let mut events: Vec<Event> = self
            .kept
            .drain(..)
            .map(|(key, kept)| match kept {
                Kept::Named => Event::KeyUp(key),
                Kept::Text => Event::TextUp,
            })
            .collect();
        for button in Button::ALL {
            if self.buttons & (1 << button as u8) != 0 {
                events.push(Event::ButtonUp(button));
            }
        }
        self.buttons = 0;
        self.physical.clear();
        events
    }

    /// A shortcut is being held: Ctrl or Alt (not both, which is AltGr and types text) or
    /// Windows. Right Alt is AltGr on many layouts, so it does not count.
    fn combo(&self) -> bool {
        let held = |test: fn(u16) -> bool| self.physical.iter().any(|k| test(k.vk));
        let alt_gr = self
            .physical
            .iter()
            .any(|k| k.vk == 0xA5 || (k.vk == 0x12 && k.ext));
        held(is_win) || (!alt_gr && (held(is_ctrl) != held(is_alt)))
    }

    fn press(&mut self, key: Key, combo: bool, context: &Context) -> Option<Event> {
        if context.password {
            return None;
        }
        let named = Event::KeyDown(key);
        let redacted = || {
            if is_command_key(key.vk) || combo {
                named
            } else {
                Event::TextDown
            }
        };
        match context.keep.keyboard {
            KeyboardMode::Drop => None,
            KeyboardMode::Redacted => Some(redacted()),
            KeyboardMode::Full if self.chat_open => {
                if matches!(key.vk, ENTER | ESCAPE) {
                    self.chat_open = false;
                }
                Some(redacted())
            }
            KeyboardMode::Full => {
                if context.chat_keys.contains(&key.vk) {
                    self.chat_open = true;
                }
                Some(named)
            }
        }
    }
}

/// The programs in front over time, to tell which rule applied at a moment.
#[derive(Default)]
pub struct FocusHistory {
    /// When each came to the front, oldest first; the first is in front since before any
    /// input still waiting.
    changes: Vec<(f64, App, u64)>,
    next_focus: u64,
}

impl FocusHistory {
    pub fn front(&mut self, at: f64, app: App) {
        if self.changes.last().is_some_and(|(_, last, _)| *last == app) {
            return;
        }
        self.next_focus += 1;
        self.changes.push((at, app, self.next_focus));
    }

    /// The program in front at `t`, when it came there, and its focus number.
    pub fn at(&self, t: f64) -> Option<(f64, &App, u64)> {
        let index = self.changes.partition_point(|(at, ..)| *at <= t);
        let (since, app, focus) = self.changes.get(index.saturating_sub(1))?;
        Some((*since, app, *focus))
    }

    /// Every program in front at some moment of `from..=to`.
    fn during(&self, from: f64, to: f64) -> impl Iterator<Item = &App> {
        let first = self
            .changes
            .partition_point(|(at, ..)| *at <= from)
            .saturating_sub(1);
        self.changes[first..]
            .iter()
            .take_while(move |(at, ..)| *at <= to)
            .map(|(_, app, _)| app)
    }

    /// Forgets what came before `t`, keeping the program in front then.
    pub fn forget_before(&mut self, t: f64) {
        let keep_from = self
            .changes
            .partition_point(|(at, ..)| *at <= t)
            .saturating_sub(1);
        self.changes.drain(..keep_from);
    }
}

/// What the password guard said, over time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardSays {
    /// The focused element of process `pid` is (or is not) a password field.
    Focus { pid: u32, password: bool },
    /// The guard could not tell for `pid` (an error or a timeout).
    Unknown { pid: u32 },
}

#[derive(Default)]
pub struct GuardHistory {
    said: Vec<(f64, GuardSays)>,
}

impl GuardHistory {
    pub fn push(&mut self, at: f64, says: GuardSays) {
        self.said.push((at, says));
    }

    pub fn forget_before(&mut self, t: f64) {
        let keep_from = self
            .said
            .partition_point(|(at, _)| *at <= t - GUARD_LEAD)
            .saturating_sub(1);
        self.said.drain(..keep_from);
    }

    /// Whether keys typed at `t` into `pid`, in front since `since`, must not be kept: the
    /// guard said its focus is a password field (from a little before it said so), or said
    /// nothing about it since it came to the front, or could not tell. A rule that keeps the
    /// keyboard in full (a game) trusts it when it cannot tell.
    pub fn password_at(&self, t: f64, pid: u32, since: f64, full: bool) -> bool {
        // What it said about `pid` last, as of `t`.
        let mut verdict = None;
        for (at, says) in &self.said {
            let (about, password) = match *says {
                GuardSays::Focus { pid, password } => (pid, Some(password)),
                GuardSays::Unknown { pid } => (pid, None),
            };
            let from = if password == Some(true) {
                at - GUARD_LEAD
            } else {
                *at
            };
            // A password record reaches back before it was said, past ones said before it.
            if from > t {
                continue;
            }
            // Said about another program: whatever it said of this one before is old news.
            verdict = (about == pid && *at >= since - GUARD_LEAD).then_some(password);
        }
        match verdict {
            Some(Some(password)) => password,
            // Nothing said since it came to the front, or could not tell.
            _ => !full,
        }
    }
}

/// The rules at `t`: the program in front then, made stricter by any other in front within
/// `SWITCH_WINDOW`, and the password guard's word.
pub fn context_at(
    rules: &[InputRule],
    focus: &FocusHistory,
    guard: &GuardHistory,
    t: f64,
) -> Context {
    let Some((since, front, number)) = focus.at(t) else {
        return Context {
            keep: Keep::NOTHING,
            chat_keys: Vec::new(),
            focus: 0,
            password: false,
        };
    };
    let (mut keep, chat_keys) = keep_for(rules, front);
    for app in focus.during(t - SWITCH_WINDOW, t + SWITCH_WINDOW) {
        keep = keep.stricter(keep_for(rules, app).0);
    }
    let password = keep.keyboard != KeyboardMode::Drop
        && guard.password_at(t, front.pid, since, keep.keyboard == KeyboardMode::Full);
    Context {
        keep,
        chat_keys,
        focus: number,
        password,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::default_input_rules;

    fn app(exe: &str, pid: u32) -> App {
        App {
            pid,
            exe: exe.into(),
            path: Some(format!("C:\\Programs\\{exe}")),
            ..App::default()
        }
    }

    fn game(exe: &str, pid: u32) -> App {
        App {
            game: true,
            ..app(exe, pid)
        }
    }

    fn key(vk: u16) -> Key {
        Key {
            vk,
            scan: vk,
            ext: false,
        }
    }

    fn down(vk: u16) -> Raw {
        Raw::Key {
            key: key(vk),
            down: true,
        }
    }

    fn up(vk: u16) -> Raw {
        Raw::Key {
            key: key(vk),
            down: false,
        }
    }

    fn context(keyboard: KeyboardMode, mouse: MouseMode) -> Context {
        Context {
            keep: Keep { keyboard, mouse },
            chat_keys: vec![ENTER, 0x59],
            focus: 1,
            password: false,
        }
    }

    const A: u16 = 0x41;
    const CTRL: u16 = 0xA2;

    #[test]
    fn the_first_matching_rule_wins_and_the_defaults_are_as_asked() {
        let rules = default_input_rules();
        let keep = |a: &App| keep_for(&rules, a).0;
        let full = Keep {
            keyboard: KeyboardMode::Full,
            mouse: MouseMode::Full,
        };
        // A game is a game even if it is also listed lower down.
        assert_eq!(keep(&game("chrome.exe", 1)), full);
        assert_eq!(keep_for(&rules, &game("cs2.exe", 1)).1, vec![ENTER]);
        assert_eq!(keep(&app("Discord.exe", 2)).keyboard, KeyboardMode::Drop);
        assert_eq!(keep(&app("Discord.exe", 2)).mouse, MouseMode::Full);
        assert_eq!(keep(&app("KeePassXC.exe", 3)), Keep::NOTHING);
        assert_eq!(keep(&app("msedge.exe", 4)).keyboard, KeyboardMode::Redacted);
        let fallback = keep(&app("explorer.exe", 5));
        assert_eq!(
            (fallback.keyboard, fallback.mouse),
            (KeyboardMode::Redacted, MouseMode::Full)
        );
        // Umbra itself keeps nothing, whatever the rules say.
        let umbra = App {
            umbra: true,
            ..game("umbra.exe", 6)
        };
        assert_eq!(keep(&umbra), Keep::NOTHING);
        // Reordered by the user: the first match wins.
        let mut reordered = rules.clone();
        reordered.rotate_right(1);
        assert_eq!(
            keep_for(&reordered, &app("Discord.exe", 2)).0.keyboard,
            KeyboardMode::Redacted
        );
    }

    #[test]
    fn redacted_keys_lose_their_name_but_commands_and_shortcuts_keep_it() {
        let mut filter = Filter::default();
        let redacted = context(KeyboardMode::Redacted, MouseMode::Full);
        assert_eq!(filter.apply(down(A), &redacted), Some(Event::TextDown));
        assert_eq!(filter.apply(up(A), &redacted), Some(Event::TextUp));
        for vk in [
            0x10, 0x70, 0x87, 0x25, 0x1B, 0x09, 0x0D, 0x08, 0x2E, 0x24, 0x22, 0xB3,
        ] {
            assert_eq!(
                filter.apply(down(vk), &redacted),
                Some(Event::KeyDown(key(vk))),
                "{vk:#x}"
            );
            filter.apply(up(vk), &redacted);
        }
        // Digits, space and the number pad's digits type text.
        for vk in [0x31, 0x20, 0x61, 0xBA] {
            assert_eq!(filter.apply(down(vk), &redacted), Some(Event::TextDown));
            filter.apply(up(vk), &redacted);
        }
        // Ctrl+A is a shortcut; Shift+A is text; AltGr (right Alt, or Ctrl+Alt) types text.
        filter.apply(down(CTRL), &redacted);
        assert_eq!(
            filter.apply(down(A), &redacted),
            Some(Event::KeyDown(key(A)))
        );
        filter.apply(up(A), &redacted);
        filter.apply(down(0xA4), &redacted);
        assert_eq!(filter.apply(down(0x51), &redacted), Some(Event::TextDown));
        filter.apply(up(0x51), &redacted);
        filter.apply(up(0xA4), &redacted);
        filter.apply(up(CTRL), &redacted);
        filter.apply(down(0xA5), &redacted);
        assert_eq!(filter.apply(down(0x51), &redacted), Some(Event::TextDown));
        filter.release_all();
        filter.apply(down(0xA0), &redacted);
        assert_eq!(filter.apply(down(A), &redacted), Some(Event::TextDown));
    }

    #[test]
    fn a_release_matches_its_press_and_repeats_are_ignored() {
        let mut filter = Filter::default();
        let full = context(KeyboardMode::Full, MouseMode::Full);
        let dropped = context(KeyboardMode::Drop, MouseMode::Drop);
        assert_eq!(filter.apply(down(A), &full), Some(Event::KeyDown(key(A))));
        assert_eq!(filter.apply(down(A), &full), None, "a repeat");
        // Released under a rule that keeps nothing: the press was kept, so its release is.
        assert_eq!(filter.apply(up(A), &dropped), Some(Event::KeyUp(key(A))));
        // Pressed where nothing is kept, then the rules change while it is held: its repeats
        // do not start it half way, and its release says nothing.
        assert_eq!(filter.apply(down(A), &dropped), None);
        assert_eq!(filter.apply(down(A), &full), None);
        assert_eq!(filter.apply(up(A), &full), None);
        let button = |down| Raw::Button {
            button: Button::Left,
            down,
        };
        assert_eq!(
            filter.apply(button(true), &full),
            Some(Event::ButtonDown(Button::Left))
        );
        assert_eq!(filter.release_all(), vec![Event::ButtonUp(Button::Left)]);
    }

    #[test]
    fn the_mouse_keeps_what_its_rule_says() {
        let mut filter = Filter::default();
        let moved = Raw::Move { dx: 3, dy: -1 };
        let wheel = Raw::Wheel {
            vertical: 120,
            horizontal: 0,
        };
        let buttons_only = context(KeyboardMode::Full, MouseMode::ButtonsOnly);
        assert_eq!(filter.apply(moved, &buttons_only), None);
        assert!(filter.apply(wheel, &buttons_only).is_some());
        let dropped = context(KeyboardMode::Full, MouseMode::Drop);
        assert_eq!(filter.apply(wheel, &dropped), None);
        let right = Raw::Button {
            button: Button::Right,
            down: true,
        };
        assert_eq!(filter.apply(right, &dropped), None);
        let full = context(KeyboardMode::Full, MouseMode::Full);
        assert_eq!(
            filter.apply(moved, &full),
            Some(Event::Move { dx: 3, dy: -1 })
        );
    }

    #[test]
    fn a_chat_is_redacted_until_enter_or_escape() {
        let mut filter = Filter::default();
        let game = context(KeyboardMode::Full, MouseMode::Full);
        assert_eq!(filter.apply(down(A), &game), Some(Event::KeyDown(key(A))));
        filter.apply(up(A), &game);
        // Y opens the chat: Y itself is a game key, what follows is text.
        assert_eq!(
            filter.apply(down(0x59), &game),
            Some(Event::KeyDown(key(0x59)))
        );
        filter.apply(up(0x59), &game);
        assert_eq!(filter.apply(down(A), &game), Some(Event::TextDown));
        filter.apply(up(A), &game);
        assert_eq!(
            filter.apply(down(ENTER), &game),
            Some(Event::KeyDown(key(ENTER)))
        );
        filter.apply(up(ENTER), &game);
        assert_eq!(filter.apply(down(A), &game), Some(Event::KeyDown(key(A))));
        filter.apply(up(A), &game);
        // Enter both opens and sends.
        filter.apply(down(ENTER), &game);
        filter.apply(up(ENTER), &game);
        assert_eq!(filter.apply(down(A), &game), Some(Event::TextDown));
        filter.apply(up(A), &game);
        filter.apply(down(ESCAPE), &game);
        filter.apply(up(ESCAPE), &game);
        assert_eq!(filter.apply(down(A), &game), Some(Event::KeyDown(key(A))));
        filter.apply(up(A), &game);
        // Another program in front ends the chat.
        filter.apply(down(0x59), &game);
        filter.apply(up(0x59), &game);
        let back = Context {
            focus: 2,
            ..game.clone()
        };
        assert_eq!(filter.apply(down(A), &back), Some(Event::KeyDown(key(A))));
    }

    #[test]
    fn a_password_field_keeps_no_key_at_all() {
        let mut filter = Filter::default();
        let password = Context {
            password: true,
            ..context(KeyboardMode::Redacted, MouseMode::Full)
        };
        assert_eq!(filter.apply(down(A), &password), None);
        assert_eq!(filter.apply(down(ENTER), &password), None);
        assert!(
            filter
                .apply(Raw::Move { dx: 1, dy: 1 }, &password)
                .is_some()
        );
    }

    #[test]
    fn right_at_a_switch_the_stricter_rule_applies() {
        let rules = default_input_rules();
        let guard = GuardHistory::default();
        let mut focus = FocusHistory::default();
        focus.front(0.0, game("cs2.exe", 1));
        focus.front(10.0, app("KeePassXC.exe", 2));
        focus.front(20.0, game("cs2.exe", 1));
        let keep = |t: f64| context_at(&rules, &focus, &guard, t).keep;
        assert_eq!(keep(5.0).keyboard, KeyboardMode::Full);
        // Just before and just after the password manager came to the front: nothing kept.
        assert_eq!(keep(9.96), Keep::NOTHING);
        assert_eq!(keep(10.04), Keep::NOTHING);
        assert_eq!(keep(19.97), Keep::NOTHING);
        assert_eq!(keep(20.04), Keep::NOTHING);
        assert_eq!(keep(9.9).keyboard, KeyboardMode::Full);
        assert_eq!(keep(20.1).keyboard, KeyboardMode::Full);
        // A new program in front is a new focus.
        assert_ne!(
            context_at(&rules, &focus, &guard, 5.0).focus,
            context_at(&rules, &focus, &guard, 25.0).focus
        );
        focus.forget_before(15.0);
        assert_eq!(context_at(&rules, &focus, &guard, 16.0).keep, Keep::NOTHING);
    }

    #[test]
    fn the_password_guard_fails_closed_except_for_games() {
        let rules = default_input_rules();
        let mut focus = FocusHistory::default();
        focus.front(0.0, app("chrome.exe", 7));
        let mut guard = GuardHistory::default();
        let password = |guard: &GuardHistory, focus: &FocusHistory, t| {
            context_at(&rules, focus, guard, t).password
        };
        // Nothing heard from the guard yet: a browser's keys are not kept.
        assert!(password(&guard, &focus, 1.0));
        guard.push(
            2.0,
            GuardSays::Focus {
                pid: 7,
                password: false,
            },
        );
        assert!(password(&guard, &focus, 1.9));
        assert!(!password(&guard, &focus, 2.1));
        // A password field: from a little before the guard said so.
        guard.push(
            5.0,
            GuardSays::Focus {
                pid: 7,
                password: true,
            },
        );
        assert!(!password(&guard, &focus, 4.7));
        assert!(password(&guard, &focus, 4.8));
        guard.push(
            8.0,
            GuardSays::Focus {
                pid: 7,
                password: false,
            },
        );
        assert!(!password(&guard, &focus, 8.1));
        // The guard could not tell.
        guard.push(9.0, GuardSays::Unknown { pid: 7 });
        assert!(password(&guard, &focus, 9.1));
        // Another program came to the front: what was said of the browser does not carry over.
        focus.front(12.0, app("notepad.exe", 9));
        assert!(password(&guard, &focus, 12.1));
        guard.push(
            12.2,
            GuardSays::Focus {
                pid: 9,
                password: false,
            },
        );
        assert!(!password(&guard, &focus, 12.3));
        // A game trusts it when the guard is silent or cannot tell.
        focus.front(20.0, game("cs2.exe", 11));
        assert!(!password(&guard, &focus, 20.2));
        guard.push(21.0, GuardSays::Unknown { pid: 11 });
        assert!(!password(&guard, &focus, 21.1));
        guard.push(
            22.0,
            GuardSays::Focus {
                pid: 11,
                password: true,
            },
        );
        assert!(password(&guard, &focus, 22.1));
        // A program whose keyboard is dropped needs no guard.
        focus.front(30.0, app("KeePassXC.exe", 12));
        assert!(!password(&guard, &focus, 30.1));
    }
}
