//! User settings: the shape the UI edits, validation, and the file they are stored in.
//! The JSON field names match `app/ui/src/lib/api/types.ts`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodecChoice {
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RateMode {
    Cbr,
    Vbr,
    /// Constant quality: the bitrate follows the picture, up to the peak bitrate.
    Cq,
    Cqp,
}

/// What the encoder is tuned for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tune {
    /// High quality.
    Hq,
    /// Low latency.
    Ll,
    /// Ultra low latency.
    Ull,
}

/// A second look at each frame before it is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Multipass {
    Off,
    /// The first pass at quarter resolution.
    Quarter,
    /// The first pass at full resolution.
    Full,
}

/// How a picture of a different shape than the video is fitted into it, for example a game at
/// a stretched 1440×1080 recorded into a 1920×1080 video.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scaling {
    /// Fill the whole video, the way a stretched game fills the screen.
    Stretch,
    /// Keep the shape and fill the rest with black bars.
    Fit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordingMode {
    Always,
    Games,
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    Split,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CaptureChoice {
    /// `device` is a name such as `\\.\DISPLAY1`. Empty means the primary monitor.
    Monitor {
        device: String,
    },
    Window {
        exe: String,
        title: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoPrefs {
    pub codec: CodecChoice,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: u32,
    pub rate_mode: RateMode,
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub qp: u8,
    /// Quality for constant-quality mode, 1 (best) to 51.
    pub cq: u8,
    pub preset: u8,
    pub capture_cursor: bool,
    /// Seconds between keyframes. A clip can only start on one.
    pub keyframe_secs: u32,
    pub tune: Tune,
    pub multipass: Multipass,
    /// Frames the encoder looks ahead to plan the bitrate, 0 for none.
    pub lookahead: u8,
    /// Spends more bits on flat areas, where blocking shows most.
    pub spatial_aq: bool,
    /// Spends more bits on parts of the picture that stay still.
    pub temporal_aq: bool,
    /// How strongly spatial AQ works, 1 to 15.
    pub aq_strength: u8,
    pub scaling: Scaling,
    /// What an HDR screen is recorded as.
    pub hdr: HdrChoice,
    /// Leave out the frames of a still screen instead of encoding the last picture again, so
    /// the encoder rests while nothing moves. The video's frame rate then varies.
    pub skip_still: bool,
}

/// How a screen in HDR mode is recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HdrChoice {
    /// Converted to SDR, so the video looks right everywhere.
    #[default]
    Sdr,
    /// Kept as HDR10, with HEVC or AV1 (H.264 is converted to SDR).
    Hdr10,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MicPrefs {
    pub enabled: bool,
    pub device_id: Option<String>,
    /// Only record the microphone while `push_to_talk_key` is held.
    pub push_to_talk: bool,
    pub push_to_talk_key: String,
    /// Also mix the microphone into the combined track. It always has its own track too.
    pub in_mix: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioPrefs {
    pub mode: AudioMode,
    pub game: bool,
    pub apps: Vec<String>,
    pub others: bool,
    pub mic: MicPrefs,
    pub bitrate_kbps: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipPrefs {
    pub length_secs: u32,
    pub hotkey: String,
    pub save_sound: bool,
    pub save_sound_volume: f32,
    /// Seconds recorded after the clip hotkey before the clip is saved, so the moment
    /// right after a play is in it too.
    pub post_secs: u32,
    /// Keep the replay buffer in files on disk instead of in memory.
    pub buffer_on_disk: bool,
    /// Folder for those files; empty uses Umbra's cache folder.
    pub buffer_folder: String,
}

impl ClipPrefs {
    /// Seconds the replay buffer holds: the clip and what follows the hotkey.
    pub fn buffer_secs(&self) -> u32 {
        self.length_secs + self.post_secs
    }

    /// Where the buffer files go, when the buffer is on disk.
    pub fn buffer_dir(&self) -> Option<PathBuf> {
        self.buffer_on_disk.then(|| {
            if self.buffer_folder.trim().is_empty() {
                cache_dir().join("buffer")
            } else {
                PathBuf::from(self.buffer_folder.trim())
            }
        })
    }
}

/// Clips saved by themselves when something happens in a game (see `autoclip`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoClipPrefs {
    pub enabled: bool,
    /// In games without rounds, events this close to the one before go in the same clip.
    pub merge_secs: f32,
    /// Volume of the sound for a clip saved this way, 0 to 1; 0 is silent.
    pub sound_volume: f32,
    /// Kills it takes for a clip: 1 for every kill, 3 for a 3K, 5 for an ace. Counted per
    /// round in games with rounds, else within the merge window.
    pub min_kills: u32,
    pub kill: EventRule,
    /// Special kills always show in a clip's name; switched on, one makes a clip by itself,
    /// even in a round with fewer kills than `min_kills`.
    pub headshot: EventRule,
    pub knife: EventRule,
    pub zeus: EventRule,
    /// One bullet, two kills.
    pub collateral: EventRule,
    /// A kill while flashed.
    pub blind_kill: EventRule,
    pub death: EventRule,
    pub round_win: EventRule,
    pub cs2: Cs2Prefs,
}

/// What one kind of event does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EventRule {
    /// Makes a clip. Off, it is only named in a clip something else made.
    pub enabled: bool,
    /// Events a detector is less sure of (0 to 1) are left out.
    pub min_confidence: f32,
    /// Seconds kept before and after the event.
    pub pre_secs: f32,
    pub post_secs: f32,
}

/// Counter-Strike 2's Game State Integration: the game posts its state to Umbra.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cs2Prefs {
    pub enabled: bool,
    /// Umbra listens on this port, on 127.0.0.1 only.
    pub port: u16,
    /// The game sends this back with every post, so nothing else can pass for it.
    pub token: String,
    /// The game modes clipped, from [`CS2_MODES`].
    pub modes: Vec<String>,
}

pub const CS2_PORT: u16 = 47380;

/// CS2's game modes as Settings offers them; `other` is every mode not named here.
pub const CS2_MODES: [&str; 6] = [
    "competitive",
    "wingman",
    "casual",
    "deathmatch",
    "armsrace",
    "other",
];

impl Default for AutoClipPrefs {
    fn default() -> Self {
        let on = EventRule::default();
        let off = EventRule {
            enabled: false,
            ..EventRule::default()
        };
        Self {
            enabled: false,
            merge_secs: 10.0,
            sound_volume: 0.3,
            min_kills: 1,
            kill: on.clone(),
            headshot: off.clone(),
            knife: on.clone(),
            zeus: on.clone(),
            collateral: on,
            blind_kill: off.clone(),
            death: off.clone(),
            round_win: off,
            cs2: Cs2Prefs::default(),
        }
    }
}

impl Default for EventRule {
    fn default() -> Self {
        Self {
            enabled: true,
            min_confidence: 0.5,
            pre_secs: 8.0,
            post_secs: 3.0,
        }
    }
}

impl Default for Cs2Prefs {
    fn default() -> Self {
        Self {
            enabled: false,
            port: CS2_PORT,
            token: String::new(),
            // Deathmatch and Arms Race are a kill every few seconds.
            modes: ["competitive", "wingman", "casual", "other"]
                .map(String::from)
                .to_vec(),
        }
    }
}

/// What a marker (see `EventKind::RoundDone`) does: nothing.
const NO_RULE: EventRule = EventRule {
    enabled: false,
    min_confidence: 0.0,
    pre_secs: 0.0,
    post_secs: 0.0,
};

impl AutoClipPrefs {
    pub fn rule(&self, kind: crate::autoclip::EventKind) -> &EventRule {
        use crate::autoclip::EventKind::*;
        match kind {
            Kill => &self.kill,
            Headshot => &self.headshot,
            Knife => &self.knife,
            Zeus => &self.zeus,
            Collateral => &self.collateral,
            BlindKill => &self.blind_kill,
            Death => &self.death,
            RoundWin => &self.round_win,
            RoundDone => &NO_RULE,
        }
    }

    fn normalized(mut self) -> Self {
        let defaults = Self::default();
        let within = |value: f32, low: f32, high: f32, default: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                default
            }
        };
        self.merge_secs = within(self.merge_secs, 0.0, 60.0, defaults.merge_secs);
        self.sound_volume = within(self.sound_volume, 0.0, 1.0, defaults.sound_volume);
        self.min_kills = self.min_kills.clamp(1, 5);
        for rule in [
            &mut self.kill,
            &mut self.headshot,
            &mut self.knife,
            &mut self.zeus,
            &mut self.collateral,
            &mut self.blind_kill,
            &mut self.death,
            &mut self.round_win,
        ] {
            rule.min_confidence = within(rule.min_confidence, 0.0, 1.0, 0.5);
            rule.pre_secs = within(rule.pre_secs, 0.0, 120.0, 8.0);
            rule.post_secs = within(rule.post_secs, 0.0, 60.0, 3.0);
        }
        let cs2 = &mut self.cs2;
        if cs2.port < 1024 {
            cs2.port = CS2_PORT;
        }
        // It goes between quotes in the game's config file.
        cs2.token.retain(|c| c.is_ascii_alphanumeric());
        if cs2.token.is_empty() {
            cs2.token = random_token();
        }
        cs2.modes.retain(|m| CS2_MODES.contains(&m.as_str()));
        cs2.modes.sort();
        cs2.modes.dedup();
        self
    }
}

/// 128 random bits as hex, from the standard library's randomly keyed hasher.
fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        hasher.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// Keyboard and mouse input recorded with clips, to show over them (see `input`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InputPrefs {
    pub enabled: bool,
    /// Switches recording input on and off; empty for none.
    pub hotkey: String,
    /// What is kept in which program; the first that matches applies.
    pub rules: Vec<InputRule>,
    /// How the overlay looks on new clips, until moved in the editor.
    pub overlay: InputOverlayPrefs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyboardMode {
    /// Every key, by name.
    Full,
    /// Keys that type text without saying which; others (and shortcuts) by name.
    Redacted,
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MouseMode {
    Full,
    /// Buttons and the wheel, not where the mouse goes.
    ButtonsOnly,
    Drop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleMatch {
    /// The game Umbra detected.
    Game,
    /// The programs in `apps`: names, paths or folders, as for video presets.
    Apps,
    /// Every program.
    Any,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InputRule {
    pub name: String,
    pub matches: RuleMatch,
    pub apps: Vec<String>,
    pub keyboard: KeyboardMode,
    pub mouse: MouseMode,
    /// Keys that open a chat (`Enter`, `KeyY`): until Enter or Esc, keys typed are kept
    /// as for `Redacted`. Only for rules that keep the keyboard in full.
    pub chat_keys: Vec<String>,
}

impl Default for InputRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            matches: RuleMatch::Apps,
            apps: Vec::new(),
            keyboard: KeyboardMode::Redacted,
            mouse: MouseMode::Full,
            chat_keys: Vec::new(),
        }
    }
}

/// The rules a new install starts with.
pub fn default_input_rules() -> Vec<InputRule> {
    let apps = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
    vec![
        InputRule {
            name: "Games".into(),
            matches: RuleMatch::Game,
            keyboard: KeyboardMode::Full,
            chat_keys: vec!["Enter".into()],
            ..InputRule::default()
        },
        InputRule {
            name: "Chat apps".into(),
            apps: apps(&[
                "Discord.exe",
                "Telegram.exe",
                "WhatsApp.exe",
                "Signal.exe",
                "ms-teams.exe",
                "Teams.exe",
                "slack.exe",
            ]),
            keyboard: KeyboardMode::Drop,
            ..InputRule::default()
        },
        InputRule {
            name: "Password managers".into(),
            apps: apps(&[
                "1Password.exe",
                "Bitwarden.exe",
                "KeePass.exe",
                "KeePassXC.exe",
                "Dashlane.exe",
            ]),
            keyboard: KeyboardMode::Drop,
            mouse: MouseMode::Drop,
            ..InputRule::default()
        },
        InputRule {
            name: "Browsers".into(),
            apps: apps(&[
                "chrome.exe",
                "msedge.exe",
                "firefox.exe",
                "brave.exe",
                "opera.exe",
            ]),
            ..InputRule::default()
        },
        InputRule {
            name: "Everything else".into(),
            matches: RuleMatch::Any,
            ..InputRule::default()
        },
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayKeys {
    None,
    /// W, A, S, D and the keys around them.
    Wasd,
    /// A compact keyboard.
    Keyboard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InputOverlayPrefs {
    pub keys: OverlayKeys,
    pub mouse: bool,
    /// The mouse shows which way it moves.
    pub movement: bool,
    /// A line of the keys pressed last.
    pub ticker: bool,
    /// Where, and how wide; the height follows the layout.
    pub placement: Placement,
    pub opacity: f64,
    /// CSS colour of keys and buttons held down.
    pub highlight: String,
}

impl Default for InputOverlayPrefs {
    /// Bottom left, WASD and the mouse.
    fn default() -> Self {
        Self {
            keys: OverlayKeys::Wasd,
            mouse: true,
            movement: true,
            ticker: false,
            placement: Placement {
                x: 0.02,
                y: 0.74,
                width: 0.2,
                shape: OverlayShape::Rectangle,
                ..Placement::default()
            },
            opacity: 0.9,
            highlight: "#ffffff".into(),
        }
    }
}

impl Default for InputPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey: String::new(),
            rules: default_input_rules(),
            overlay: InputOverlayPrefs::default(),
        }
    }
}

impl InputPrefs {
    fn normalized(mut self) -> Self {
        if !self.hotkey.trim().is_empty() && self.hotkey.parse::<Hotkey>().is_err() {
            self.hotkey.clear();
        }
        for rule in &mut self.rules {
            rule.apps = rule
                .apps
                .iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            rule.apps.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            rule.chat_keys
                .retain(|k| crate::hotkey::key_code(k).is_some());
        }
        let o = &mut self.overlay;
        o.placement = o.placement.clone().normalized();
        o.opacity = if o.opacity.is_finite() {
            o.opacity.clamp(0.1, 1.0)
        } else {
            0.9
        };
        if !o.highlight.starts_with('#') {
            o.highlight = InputOverlayPrefs::default().highlight;
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecordingPrefs {
    pub mode: RecordingMode,
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub games: Vec<String>,
    /// Starts or stops a recording of what is being captured.
    pub record_hotkey: String,
    /// Starts or stops recording the current game session.
    pub session_hotkey: String,
    /// Record every game session from the moment the game is found until it closes.
    pub full_sessions: bool,
    /// What a game session records.
    pub session_source: SessionSource,
    /// Pauses or resumes the recording to a file.
    pub pause_hotkey: String,
    /// Programs that never count as games, so no session records them: program names,
    /// program paths or folders.
    pub never_record: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoragePrefs {
    pub folder: String,
    pub folder_template: String,
    pub name_template: String,
    /// Delete files for good instead of moving them to the Recycle Bin.
    pub skip_recycle_bin: bool,
    /// Keep the clip folder under this many gigabytes by deleting the oldest clips; 0 is off.
    pub limit_gb: u32,
    /// Delete clips older than this many days; 0 is off.
    pub max_age_days: u32,
    /// Never delete clips in an album. Favourites are always kept.
    pub keep_albums: bool,
    /// Stop recordings to a file when the clip folder's drive has less than this many
    /// gigabytes free, and start none; 0 is off.
    pub min_free_gb: u32,
    pub shrink: ShrinkPrefs,
}

/// Making old recordings smaller in the background: encoded again as HEVC at a lower bitrate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShrinkPrefs {
    pub enabled: bool,
    /// Only recordings at least this many days old.
    pub after_days: u32,
    pub quality: ShrinkQuality,
    /// Clips too, not only recordings.
    pub include_clips: bool,
    /// At most this many frames per second afterwards; 0 keeps the recording's. Fewer frames
    /// make it smaller and much quicker to compress (a 144 fps recording at 60 takes well
    /// under half the time).
    pub max_fps: u32,
}

impl Default for ShrinkPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            after_days: 7,
            quality: ShrinkQuality::Balanced,
            include_clips: false,
            max_fps: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShrinkQuality {
    Small,
    #[default]
    Balanced,
    High,
}

impl ShrinkQuality {
    /// Bits per pixel per frame for HEVC.
    pub fn bits_per_pixel(self) -> f64 {
        match self {
            ShrinkQuality::Small => 0.035,
            ShrinkQuality::Balanced => 0.055,
            ShrinkQuality::High => 0.085,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    Png,
    Jpg,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotPrefs {
    pub hotkey: String,
    pub format: ScreenshotFormat,
    pub sound: bool,
}

impl Default for ScreenshotPrefs {
    fn default() -> Self {
        Self {
            hotkey: "Alt+F11".into(),
            format: ScreenshotFormat::Png,
            sound: true,
        }
    }
}

/// How the clip player and the editor move through a clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerPrefs {
    /// Seconds the arrow keys skip.
    pub seek_secs: f64,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self { seek_secs: 5.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub capture: CaptureChoice,
    /// Video settings for everything no preset claims: the desktop and unlisted games.
    pub video: VideoPrefs,
    /// Video settings for kinds of games, each used while one of its games is recorded.
    pub presets: Vec<VideoPreset>,
    pub audio: AudioPrefs,
    pub clip: ClipPrefs,
    pub screenshot: ScreenshotPrefs,
    pub recording: RecordingPrefs,
    pub storage: StoragePrefs,
    pub player: PlayerPrefs,
    pub export: ExportPrefs,
    pub overlay: OverlayPrefs,
    pub panel: PanelPrefs,
    pub camera: CameraPrefs,
    pub logo: LogoPrefs,
    pub auto_clip: AutoClipPrefs,
    pub input: InputPrefs,
}

/// A picture (a logo, a watermark) shown over every new clip, where the editor can move or
/// remove it; drawn in on export. It is not recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LogoPrefs {
    /// A copy of the chosen image in Umbra's folder (see `logos_dir`); none for no logo.
    pub path: Option<String>,
    pub placement: Placement,
}

impl Default for LogoPrefs {
    /// Top left, small.
    fn default() -> Self {
        Self {
            path: None,
            placement: Placement {
                x: 0.02,
                y: 0.03,
                width: 0.1,
                shape: OverlayShape::Rectangle,
                ..Placement::default()
            },
        }
    }
}

/// Where chosen logos are kept, one file each, so a clip keeps its logo when another is chosen.
pub fn logos_dir() -> PathBuf {
    data_dir().join("logos")
}

/// A webcam, recorded as a stream of its own and drawn over the video by the player and by
/// export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CameraPrefs {
    /// The camera is filming. Switching it costs no recorder restart: the stream stays, empty
    /// while it is off, and the camera's light goes out.
    pub enabled: bool,
    /// Windows' id of the camera; none records no camera stream at all.
    pub device_id: Option<String>,
    /// Its name, shown while it is not connected.
    pub device_name: String,
    /// Size it is recorded at; one of the camera's own sizes. Both even.
    pub width: u32,
    pub height: u32,
    /// Where new clips show it.
    pub placement: Placement,
    /// Switches the camera on and off.
    pub hotkey: String,
}

impl Default for CameraPrefs {
    fn default() -> Self {
        Self {
            hotkey: "Alt+F7".into(),
            enabled: true,
            device_id: None,
            device_name: String::new(),
            width: 1280,
            height: 720,
            placement: Placement::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayShape {
    Rectangle,
    #[default]
    Rounded,
    Circle,
}

/// A rectangle as fractions (0 to 1) of a picture, so it fits the picture at any size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

/// Where a picture (the camera) sits over the video and how it looks. Saved into each clip, so
/// a clip keeps its look when the settings change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Placement {
    /// Left and top edge, as fractions of the video's width and height.
    pub x: f64,
    pub y: f64,
    /// Width as a fraction of the video's; the height follows the shape of what is shown.
    pub width: f64,
    /// The part of the picture shown.
    pub crop: Rect,
    pub shape: OverlayShape,
    /// Border thickness as a fraction of the shown width; 0 for none.
    pub border: f64,
    /// CSS colour, such as `#ffffff`.
    pub border_color: String,
    /// Flipped left to right, like a mirror.
    pub mirror: bool,
}

impl Placement {
    /// Kept on the video and at a size that still shows something.
    pub fn normalized(mut self) -> Self {
        self.width = self.width.clamp(0.02, 1.0);
        self.x = self.x.clamp(0.0, 1.0 - self.width);
        self.y = self.y.clamp(0.0, 0.98);
        self.border = self.border.clamp(0.0, 0.1);
        let c = &mut self.crop;
        c.width = c.width.clamp(0.05, 1.0);
        c.height = c.height.clamp(0.05, 1.0);
        c.x = c.x.clamp(0.0, 1.0 - c.width);
        c.y = c.y.clamp(0.0, 1.0 - c.height);
        self
    }
}

impl Default for Placement {
    /// Bottom right, a fifth of the width wide.
    fn default() -> Self {
        Self {
            x: 0.78,
            y: 0.75,
            width: 0.2,
            crop: Rect::default(),
            shape: OverlayShape::Rounded,
            border: 0.0,
            border_color: "#ffffff".into(),
            mirror: false,
        }
    }
}

/// The recording panel on the right: which widgets it shows, top to bottom. The window
/// decides what each id is (`status`, `actions`, `source`, `mic`, `usage`, or `setting:` and
/// the name of a setting); ids it does not know are skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PanelPrefs {
    pub widgets: Vec<String>,
}

impl Default for PanelPrefs {
    fn default() -> Self {
        Self {
            widgets: ["status", "actions", "source", "mic", "usage"]
                .map(String::from)
                .to_vec(),
        }
    }
}

/// Video settings for a kind of game, such as fast shooters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoPreset {
    pub id: String,
    pub name: String,
    /// Program names (`cs2.exe`) that use this preset.
    pub games: Vec<String>,
    pub video: VideoPrefs,
}

impl Default for VideoPreset {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            games: Vec::new(),
            video: VideoPrefs::default(),
        }
    }
}

/// The presets a new install starts with: smooth clips for shooters, detailed ones for
/// slower games. Their game lists hold common titles; any game can be added.
pub fn default_presets() -> Vec<VideoPreset> {
    let list = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
    vec![
        VideoPreset {
            id: "shooters".into(),
            name: "FPS".into(),
            games: list(&[
                "cs2.exe",
                "VALORANT-Win64-Shipping.exe",
                "r5apex.exe",
                "r5apex_dx12.exe",
                "FortniteClient-Win64-Shipping.exe",
                "Overwatch.exe",
                "RainbowSix.exe",
                "RainbowSix_Vulkan.exe",
                "cod.exe",
                "TslGame.exe",
                "EscapeFromTarkov.exe",
                "Marvel-Win64-Shipping.exe",
                "deadlock.exe",
            ]),
            video: VideoPrefs {
                fps: 120,
                rate_mode: RateMode::Vbr,
                bitrate_kbps: 50_000,
                max_bitrate_kbps: 80_000,
                preset: 5,
                spatial_aq: true,
                ..VideoPrefs::default()
            },
        },
        VideoPreset {
            id: "story".into(),
            name: "Story and open world".into(),
            games: list(&[
                "eldenring.exe",
                "Cyberpunk2077.exe",
                "RDR2.exe",
                "witcher3.exe",
                "bg3.exe",
                "bg3_dx11.exe",
                "HogwartsLegacy.exe",
                "GTA5.exe",
                "GTA5_Enhanced.exe",
                "Starfield.exe",
                "sekiro.exe",
                "GoW.exe",
            ]),
            video: VideoPrefs {
                fps: 60,
                rate_mode: RateMode::Vbr,
                bitrate_kbps: 45_000,
                max_bitrate_kbps: 70_000,
                preset: 6,
                spatial_aq: true,
                ..VideoPrefs::default()
            },
        },
    ]
}

/// How exports are made.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportPrefs {
    /// Processor threads for decoding, 0 for one per core.
    pub threads: u32,
    /// Burn in (from the library) puts the new file in the original's place, once it checks
    /// out; the original goes like a deleted clip. Otherwise both stay.
    pub burn_replace_original: bool,
}

impl Default for ExportPrefs {
    fn default() -> Self {
        Self {
            threads: 0,
            burn_replace_original: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// When the status badge (recording or replay buffer, always on screen) shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusOverlay {
    #[default]
    Off,
    Always,
    /// While a game from `status_games` is in front; any game when the list is empty.
    Games,
}

/// Short messages over the game when a clip is saved or a recording starts, and a badge that
/// stays on screen and shows what is being recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlayPrefs {
    pub enabled: bool,
    pub corner: Corner,
    pub status: StatusOverlay,
    /// Program names, paths or folders, as in the other game lists.
    pub status_games: Vec<String>,
    pub status_corner: Corner,
}

impl Default for OverlayPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            corner: Corner::TopRight,
            status: StatusOverlay::Off,
            status_games: Vec::new(),
            status_corner: Corner::TopLeft,
        }
    }
}

/// What a game session records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionSource {
    /// The same screen or window as the replay buffer.
    Capture,
    /// Only the game's own window, whatever the replay buffer captures.
    GameWindow,
}

impl Settings {
    /// The video settings for recording `exe` (the game or captured program, at `path` if
    /// known), and the preset they come from, if any.
    pub fn video_for(
        &self,
        exe: Option<&str>,
        path: Option<&str>,
    ) -> (&VideoPrefs, Option<&VideoPreset>) {
        let preset = exe.and_then(|exe| {
            self.presets
                .iter()
                .find(|p| p.games.iter().any(|g| crate::apps::matches(g, exe, path)))
        });
        match preset {
            Some(p) => (&p.video, Some(p)),
            None => (&self.video, None),
        }
    }
}

impl Default for VideoPrefs {
    fn default() -> Self {
        Self {
            codec: CodecChoice::Hevc,
            width: None,
            height: None,
            fps: 60,
            rate_mode: RateMode::Cbr,
            bitrate_kbps: 50_000,
            max_bitrate_kbps: 80_000,
            qp: 20,
            cq: 23,
            preset: 4,
            capture_cursor: true,
            keyframe_secs: 1,
            tune: Tune::Hq,
            multipass: Multipass::Off,
            lookahead: 0,
            spatial_aq: false,
            temporal_aq: false,
            aq_strength: 8,
            scaling: Scaling::Stretch,
            hdr: HdrChoice::Sdr,
            skip_still: false,
        }
    }
}

impl Default for MicPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            device_id: None,
            push_to_talk: false,
            push_to_talk_key: "KeyV".into(),
            in_mix: false,
        }
    }
}

impl Default for AudioPrefs {
    fn default() -> Self {
        Self {
            mode: AudioMode::Split,
            game: true,
            // Voice chat is the most common reason to want a separate track.
            apps: vec!["Discord.exe".into()],
            others: true,
            mic: MicPrefs::default(),
            bitrate_kbps: 160,
        }
    }
}

impl Default for ClipPrefs {
    fn default() -> Self {
        Self {
            length_secs: 30,
            hotkey: "Alt+F10".into(),
            save_sound: true,
            save_sound_volume: 0.6,
            post_secs: 0,
            buffer_on_disk: false,
            buffer_folder: String::new(),
        }
    }
}

impl Default for RecordingPrefs {
    fn default() -> Self {
        Self {
            mode: RecordingMode::Games,
            launch_at_startup: false,
            start_minimized: true,
            games: Vec::new(),
            record_hotkey: "Alt+F9".into(),
            session_hotkey: "Alt+F12".into(),
            full_sessions: false,
            session_source: SessionSource::Capture,
            pause_hotkey: "Alt+F8".into(),
            never_record: Vec::new(),
        }
    }
}

impl Default for StoragePrefs {
    fn default() -> Self {
        let videos = std::env::var_os("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Videos"))
            .unwrap_or_else(|| PathBuf::from("Videos"));
        Self {
            folder: videos.join("Umbra").to_string_lossy().into_owned(),
            folder_template: "{game}".into(),
            name_template: "{game} {date} {time}".into(),
            skip_recycle_bin: false,
            limit_gb: 0,
            max_age_days: 0,
            keep_albums: true,
            min_free_gb: 5,
            shrink: ShrinkPrefs::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture: CaptureChoice::Monitor {
                device: String::new(),
            },
            video: VideoPrefs::default(),
            presets: default_presets(),
            audio: AudioPrefs::default(),
            clip: ClipPrefs::default(),
            screenshot: ScreenshotPrefs::default(),
            recording: RecordingPrefs::default(),
            storage: StoragePrefs::default(),
            player: PlayerPrefs::default(),
            export: ExportPrefs::default(),
            overlay: OverlayPrefs::default(),
            panel: PanelPrefs::default(),
            camera: CameraPrefs::default(),
            logo: LogoPrefs::default(),
            auto_clip: AutoClipPrefs::default(),
            input: InputPrefs::default(),
        }
    }
}

impl VideoPrefs {
    /// Brings every value into the range the encoder supports.
    pub fn normalized(mut self) -> Self {
        let v = &mut self;
        v.fps = v.fps.clamp(1, 360);
        v.bitrate_kbps = v.bitrate_kbps.clamp(100, 2_000_000);
        v.max_bitrate_kbps = v.max_bitrate_kbps.clamp(v.bitrate_kbps, 4_000_000);
        v.qp = v.qp.min(51);
        v.cq = v.cq.clamp(1, 51);
        v.preset = v.preset.clamp(1, 7);
        v.keyframe_secs = v.keyframe_secs.clamp(1, 10);
        v.lookahead = v.lookahead.min(32);
        v.aq_strength = v.aq_strength.clamp(1, 15);
        // Width and height come as a pair, and encoders need even sizes.
        match (v.width, v.height) {
            (Some(w), Some(h)) if w >= 16 && h >= 16 => {
                v.width = Some((w.min(16384) / 2) * 2);
                v.height = Some((h.min(16384) / 2) * 2);
            }
            _ => {
                v.width = None;
                v.height = None;
            }
        }
        self
    }
}

impl Settings {
    /// Brings every value into the range the engine supports. The UI shows what comes back,
    /// so a value that was changed here is visible to the user.
    pub fn normalized(mut self) -> Self {
        let defaults = Settings::default();

        self.video = self.video.normalized();
        let camera = &mut self.camera;
        camera.width = (camera.width & !1).clamp(160, 3840);
        camera.height = (camera.height & !1).clamp(120, 2160);
        camera.placement = camera.placement.clone().normalized();
        self.logo.placement = self.logo.placement.clone().normalized();
        for (i, preset) in self.presets.iter_mut().enumerate() {
            preset.video = preset.video.clone().normalized();
            if preset.id.trim().is_empty() {
                preset.id = format!("preset-{}", i + 1);
            }
            if preset.name.trim().is_empty() {
                preset.name = format!("Preset {}", i + 1);
            }
            // The built-in preset's first name, from before it was renamed.
            if preset.id == "shooters" && preset.name == "Fast shooters" {
                preset.name = "FPS".into();
            }
            preset.games.retain(|g| !g.trim().is_empty());
            preset.games.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        }
        self.export.threads = self.export.threads.min(256);

        let a = &mut self.audio;
        a.bitrate_kbps = a.bitrate_kbps.clamp(32, 512);
        a.apps.retain(|app| !app.trim().is_empty());
        a.apps.dedup();

        let c = &mut self.clip;
        c.length_secs = c.length_secs.clamp(5, 600);
        c.save_sound_volume = c.save_sound_volume.clamp(0.0, 1.0);
        if c.hotkey.parse::<Hotkey>().is_err() {
            c.hotkey = defaults.clip.hotkey.clone();
        }
        if self.screenshot.hotkey.parse::<Hotkey>().is_err() {
            self.screenshot.hotkey = defaults.screenshot.hotkey.clone();
        }
        if self.recording.record_hotkey.parse::<Hotkey>().is_err() {
            self.recording.record_hotkey = defaults.recording.record_hotkey.clone();
        }
        if self.recording.session_hotkey.parse::<Hotkey>().is_err() {
            self.recording.session_hotkey = defaults.recording.session_hotkey.clone();
        }
        if self.recording.pause_hotkey.parse::<Hotkey>().is_err() {
            self.recording.pause_hotkey = defaults.recording.pause_hotkey.clone();
        }
        if self.audio.mic.push_to_talk_key.parse::<Hotkey>().is_err() {
            self.audio.mic.push_to_talk_key = defaults.audio.mic.push_to_talk_key.clone();
        }
        if self.camera.hotkey.parse::<Hotkey>().is_err() {
            self.camera.hotkey = defaults.camera.hotkey.clone();
        }

        let p = &mut self.player;
        p.seek_secs = if p.seek_secs.is_finite() {
            (p.seek_secs.clamp(0.1, 600.0) * 10.0).round() / 10.0
        } else {
            defaults.player.seek_secs
        };

        let s = &mut self.storage;
        if s.folder.trim().is_empty() {
            s.folder = defaults.storage.folder.clone();
        }
        if s.name_template.trim().is_empty() {
            s.name_template = defaults.storage.name_template.clone();
        }
        self.auto_clip = self.auto_clip.normalized();
        self.input = self.input.normalized();
        self
    }

    /// Memory the replay buffer needs at its worst, in bytes, for the settings' bitrate.
    /// For constant-quality recording the real figure varies, so this uses the peak bitrate.
    pub fn estimated_buffer_bytes(&self) -> u64 {
        let video_kbps = match self.video.rate_mode {
            RateMode::Cbr => self.video.bitrate_kbps,
            RateMode::Vbr | RateMode::Cq => self.video.max_bitrate_kbps,
            // Near-lossless at 1440p can pass a gigabit; this is a rough ceiling.
            RateMode::Cqp => 400_000u32
                .saturating_sub(self.video.qp as u32 * 6_000)
                .max(30_000),
        } as u64;
        let audio_kbps = self.audio.bitrate_kbps as u64 * 6;
        (video_kbps + audio_kbps) * 1000 / 8 * self.clip.length_secs as u64
    }
}

/// Where the settings, albums and bookmarks live: `Documents\Umbra`, so they are easy to
/// find, back up and copy to another PC. `UMBRA_SETTINGS_DIR` overrides it (for testing).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("UMBRA_SETTINGS_DIR") {
        return PathBuf::from(dir);
    }
    documents_dir().join("Umbra")
}

/// Thumbnails and other files Umbra can make again: `%LOCALAPPDATA%\app.umbra.clipper\cache`,
/// beside the window's own data and apart from `%LOCALAPPDATA%\Umbra`, where the installer puts
/// the program. `UMBRA_CACHE_DIR` overrides it (for testing).
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("UMBRA_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    local_app_data().join("app.umbra.clipper").join("cache")
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// The user's Documents folder, wherever Windows has it (it may be moved, or in OneDrive).
fn documents_dir() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        if let Ok(path) = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) {
            let text = path.to_string().ok();
            CoTaskMemFree(Some(path.0 as *const _));
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                return PathBuf::from(text);
            }
        }
    }
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Documents"))
        .unwrap_or_else(std::env::temp_dir)
}

/// Where Umbra kept everything before: `%APPDATA%\Umbra`.
fn legacy_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Umbra"))
}

/// Moves the settings, albums and bookmarks from the old folder to [`data_dir`], and the
/// caches to [`cache_dir`], once. Nothing is overwritten. Returns what was moved.
pub fn migrate_legacy() -> Vec<PathBuf> {
    let mut moved = match legacy_dir() {
        Some(old) => migrate_from(&old, &data_dir(), &cache_dir()),
        None => Vec::new(),
    };
    // A build of 2026-10-01 kept its caches in the program folder; they are moved out of it.
    if std::env::var_os("UMBRA_CACHE_DIR").is_none() {
        let program = local_app_data().join("Umbra");
        for name in ["thumbs", "tracks"] {
            let (from, to) = (program.join(name), cache_dir().join(name));
            if from.is_dir() && !to.exists() {
                let _ = std::fs::create_dir_all(cache_dir());
                if std::fs::rename(&from, &to).is_ok() {
                    moved.push(to);
                }
            }
        }
    }
    moved
}

fn migrate_from(old: &Path, data: &Path, cache: &Path) -> Vec<PathBuf> {
    let mut moved = Vec::new();
    if old == data || !old.join("settings.json").exists() || data.join("settings.json").exists() {
        return moved;
    }
    let mut take = |from: PathBuf, to: PathBuf| {
        if !from.exists() || to.exists() {
            return;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // A rename fails across drives; files are copied then, folders are left to rebuild.
        let done = std::fs::rename(&from, &to).is_ok()
            || (from.is_file()
                && std::fs::copy(&from, &to).is_ok()
                && std::fs::remove_file(&from).is_ok());
        if done {
            moved.push(to);
        } else {
            log::warn!("cannot move {} to {}", from.display(), to.display());
        }
    };
    for name in ["settings.json", "albums.json", "recordings.json"] {
        take(old.join(name), data.join(name));
    }
    for name in ["thumbs", "tracks"] {
        take(old.join(name), cache.join(name));
    }
    // The old folder goes once nothing of Umbra's is left in it.
    let _ = std::fs::remove_dir(old);
    moved
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Reads the settings file. A missing or unreadable file gives the defaults.
pub fn load(path: &Path) -> Settings {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(settings) => settings.normalized(),
            Err(err) => {
                // Kept aside, as the next save replaces it: the settings can still be put back
                // by hand (an older Umbra cannot read what a newer one wrote, for one).
                let kept = path.with_extension("json.unreadable");
                let _ = std::fs::copy(path, &kept);
                log::warn!(
                    "settings file {} is invalid, using defaults (kept as {}): {err}",
                    path.display(),
                    kept.display()
                );
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    }
}

pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    // Write beside the file and rename, so a crash never leaves half a settings file.
    let partial = path.with_extension("json.part");
    std::fs::write(&partial, text)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    std::fs::rename(&partial, path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_a_round_trip() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, back);
    }

    #[test]
    fn json_uses_the_field_names_the_ui_expects() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json["capture"]["kind"], "monitor");
        assert_eq!(json["video"]["rateMode"], "cbr");
        assert_eq!(json["video"]["bitrateKbps"], 50_000);
        assert_eq!(json["video"]["width"], serde_json::Value::Null);
        assert_eq!(json["audio"]["mic"]["deviceId"], serde_json::Value::Null);
        assert_eq!(json["clip"]["lengthSecs"], 30);
        assert_eq!(json["clip"]["saveSoundVolume"], 0.6f32 as f64);
        assert_eq!(json["recording"]["launchAtStartup"], false);
        assert_eq!(json["storage"]["nameTemplate"], "{game} {date} {time}");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"video":{"fps":144}}"#).unwrap();
        assert_eq!(settings.video.fps, 144);
        assert_eq!(settings.video.codec, CodecChoice::Hevc);
        assert_eq!(settings.clip.length_secs, 30);
    }

    #[test]
    fn normalizing_fixes_out_of_range_values() {
        let mut settings = Settings::default();
        settings.video.fps = 5000;
        settings.video.width = Some(1921);
        settings.video.height = None;
        settings.video.qp = 99;
        settings.clip.length_secs = 1;
        settings.clip.hotkey = "banana".into();
        let fixed = settings.normalized();
        assert_eq!(fixed.video.fps, 360);
        assert_eq!((fixed.video.width, fixed.video.height), (None, None));
        assert_eq!(fixed.video.qp, 51);
        assert_eq!(fixed.clip.length_secs, 5);
        assert_eq!(fixed.clip.hotkey, "Alt+F10");
    }

    #[test]
    fn older_files_get_the_new_video_options() {
        let settings: Settings =
            serde_json::from_str(r#"{"video":{"codec":"h264","rateMode":"vbr"}}"#).unwrap();
        assert_eq!(settings.video.rate_mode, RateMode::Vbr);
        assert_eq!(settings.video.scaling, Scaling::Stretch);
        assert_eq!(settings.video.keyframe_secs, 1);
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json["video"]["multipass"], "off");
        assert_eq!(json["video"]["spatialAq"], false);
    }

    #[test]
    fn sizes_are_made_even() {
        let mut settings = Settings::default();
        settings.video.width = Some(1921);
        settings.video.height = Some(1081);
        let fixed = settings.normalized();
        assert_eq!(
            (fixed.video.width, fixed.video.height),
            (Some(1920), Some(1080))
        );
    }

    #[test]
    fn the_documents_folder_is_found() {
        let docs = documents_dir();
        assert!(docs.is_dir(), "{}", docs.display());
    }

    #[test]
    fn old_files_move_to_the_new_folders_once() {
        let root = std::env::temp_dir().join("umbra-migrate-test");
        let _ = std::fs::remove_dir_all(&root);
        let (old, data, cache) = (root.join("old"), root.join("docs"), root.join("cache"));
        std::fs::create_dir_all(old.join("thumbs")).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        std::fs::write(old.join("albums.json"), "[]").unwrap();
        std::fs::write(old.join("thumbs").join("a.jpg"), "x").unwrap();
        let moved = migrate_from(&old, &data, &cache);
        assert_eq!(moved.len(), 3);
        assert!(data.join("settings.json").exists() && data.join("albums.json").exists());
        assert!(cache.join("thumbs").join("a.jpg").exists());
        assert!(!old.exists());
        // A second start finds nothing to move, and a newer file is never replaced.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{\"old\":1}").unwrap();
        assert!(migrate_from(&old, &data, &cache).is_empty());
        assert_eq!(
            std::fs::read_to_string(data.join("settings.json")).unwrap(),
            "{}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_broken_file_gives_defaults() {
        let dir = std::env::temp_dir().join("umbra-settings-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        let mut changed = Settings::default();
        changed.clip.length_secs = 77;
        save(&path, &changed).unwrap();
        assert_eq!(load(&path).clip.length_secs, 77);
        let _ = std::fs::remove_dir_all(dir);
    }
}
