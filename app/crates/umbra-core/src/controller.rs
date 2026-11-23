//! The brain of the app: applies the user's settings, decides when recording runs (always,
//! only while a game runs, or by hand), saves clips and screenshots on their hotkeys, and
//! reports what the app is using.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::audio::AudioSource;
use crate::autoclip::{self, GameEvent};
use crate::capture::{CaptureTarget, list_monitors};
use crate::encode::{EncoderTuning, RateControl, VideoCodec};
use crate::games::{self, GameInfo, detect_game};
use crate::hotkey::{Hotkey, HotkeyListener, KeyHoldWatcher};
use crate::naming;
use crate::process::find_roots;
use crate::recorder::{
    AudioConfig, AudioTrackConfig, FileRecording, Recorder, RecorderConfig, SavedClip,
    SavedRecording,
};
pub use crate::recordings::RecordingKind;
use crate::recordings::{self, RecordingInfo};
use crate::settings::{
    self, AudioMode, CaptureChoice, CodecChoice, RateMode, RecordingMode, ScreenshotFormat,
    SessionSource, Settings, VideoPrefs,
};
use crate::snapshot;
use crate::sound::SaveSound;
use crate::usage::{Usage, UsageSampler};
use crate::util::local_datetime;
use crate::wnd::find_window;

/// How long another program has to fill the screen before it replaces a game that still runs.
const TAKE_OVER: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Recording,
    Error,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub state: State,
    pub error: Option<String>,
    pub target: Option<String>,
    pub game: Option<String>,
    /// Why the game counts as one, such as `installed through Steam`.
    pub game_reason: Option<String>,
    pub encoder: Option<String>,
    /// The screen is in HDR: `sdr` when converted, `hdr10` when kept.
    pub hdr: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub buffer_secs: f64,
    pub buffer_bytes: u64,
    /// The buffer is on disk; `buffer_bytes` is disk space.
    pub buffer_on_disk: bool,
    /// The microphone is being recorded right now.
    pub mic_live: bool,
    pub push_to_talk: bool,
    pub dropped_frames: u64,
    pub usage: Usage,
    /// The recording to a file that is running, if any.
    pub file: Option<FileStatus>,
    /// The user stopped the session of the game that is still running, so it does not start
    /// again by itself; the panel offers to restart it.
    pub session_stopped: bool,
    /// Audio tracks recorded as silence because their source cannot be opened, as (track,
    /// why): a microphone that is not connected, for one.
    pub audio_problems: Vec<(String, String)>,
    /// The camera is being recorded right now.
    /// The camera is switched on (the setting, which the hotkey and tray change too).
    pub camera_on: bool,
    pub camera_live: bool,
    /// Why the camera is not recorded while it should be, such as it not being connected.
    pub camera_problem: Option<String>,
    /// What the keyboard and mouse capture is doing.
    pub input: crate::input::InputStatus,
}

#[derive(Clone, Debug)]
pub struct FileStatus {
    pub kind: RecordingKind,
    /// Seconds recorded so far.
    pub secs: f64,
    pub bytes: u64,
    /// The game or program it is of, or `Desktop`.
    pub game: String,
    pub path: PathBuf,
    /// What it records, for a live preview.
    pub source: CaptureChoice,
    /// Paused: nothing is added to the file until it is resumed.
    pub paused: bool,
}

impl Status {
    fn idle() -> Self {
        Self {
            state: State::Idle,
            error: None,
            target: None,
            game: None,
            game_reason: None,
            encoder: None,
            hdr: None,
            width: 0,
            height: 0,
            fps: 0,
            buffer_secs: 0.0,
            buffer_bytes: 0,
            buffer_on_disk: false,
            mic_live: false,
            push_to_talk: false,
            dropped_frames: 0,
            usage: Usage::default(),
            file: None,
            session_stopped: false,
            audio_problems: Vec::new(),
            camera_on: false,
            camera_live: false,
            camera_problem: None,
            input: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClipEvent {
    pub clip: SavedClip,
    /// The game or program the clip is of, or `Desktop`.
    pub game: String,
}

#[derive(Clone, Debug)]
pub struct ScreenshotEvent {
    pub path: PathBuf,
    pub game: String,
}

#[derive(Clone, Debug)]
pub struct RecordingEvent {
    /// None when nothing could be kept.
    pub saved: Option<SavedRecording>,
    pub kind: RecordingKind,
    pub game: String,
    /// Why the recording ended early, or a problem while finishing it.
    pub error: Option<String>,
}

pub enum Event {
    Status(Status),
    ClipSaved(ClipEvent),
    ScreenshotSaved(ScreenshotEvent),
    /// A recording to a file began.
    RecordingStarted {
        kind: RecordingKind,
        game: String,
    },
    /// A recording stopped and its file is being finished; `RecordingSaved` follows.
    RecordingFinishing {
        kind: RecordingKind,
        game: String,
    },
    /// Umbra started working on a file the library should show as in progress
    /// ([`crate::processing::list`]), such as a recording left in fragments being rewritten.
    ProcessingStarted,
    RecordingSaved(RecordingEvent),
    /// A clip is being saved: after `secs` more seconds (0 for none), the last `length_secs`
    /// of `game`. `ClipSaved` or `ClipFailed` follows.
    ClipPending {
        secs: u32,
        game: String,
        length_secs: u32,
    },
    /// Saving a clip did not work.
    ClipFailed {
        error: String,
    },
    /// The clip folder's drive is below the free-space limit: the recording to a file was
    /// stopped (`stopped`), or one could not start.
    LowDisk {
        free_bytes: u64,
        limit_bytes: u64,
        stopped: Option<RecordingKind>,
    },
    /// The recording to a file was paused or resumed.
    RecordingPaused {
        paused: bool,
        kind: RecordingKind,
        game: String,
    },
    /// Files in the clip folder changed on their own (a recording was rewritten).
    LibraryChanged,
    /// A line for the auto-clip log: an event that came in, or what was done about it.
    AutoClipLog(autoclip::LogEntry),
}

/// A clip auto-clip decided on.
struct AutoClip {
    /// What to keep.
    span: (Instant, Instant),
    /// When its first event happened.
    moment: Instant,
    /// What happened, for its name: `Triple kill`.
    label: String,
    tags: Vec<(String, String)>,
}

/// A recording to a file in progress.
struct ActiveFile {
    file: FileRecording,
    kind: RecordingKind,
    game: String,
    /// The game process a session follows; it ends when this one closes. 0 for none.
    game_pid: u32,
    /// Seconds from the start, one per clip saved while recording.
    bookmarks: Vec<f64>,
    /// A recorder of its own, when a session records the game's window while the replay
    /// buffer records something else.
    own: Option<Arc<Recorder>>,
    /// What it records.
    source: CaptureChoice,
}

type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

struct Active {
    recorder: Arc<Recorder>,
    /// What the recorder was built from; a change means it has to be rebuilt.
    fingerprint: String,
    /// The same with the user's own video settings, without a game's preset or the screen's
    /// HDR: when it stays the same, the change came from the game or the screen.
    base: String,
    target_label: String,
}

struct Inner {
    settings_path: PathBuf,
    settings: Mutex<Settings>,
    active: Mutex<Option<Active>>,
    /// The recorder replaced because a game's preset or the screen's HDR changed, still
    /// running until the new one holds a full buffer: a clip saved meanwhile still has what
    /// came before the switch instead of a few seconds.
    // ponytail: two recorders run for one buffer length after such a switch (twice the
    // encoder work and buffer memory); join the two buffers on save if that ever matters.
    previous: Mutex<Option<Arc<Recorder>>>,
    game: Mutex<Option<GameInfo>>,
    /// Process id of the game, shared with the audio tracks so they follow it.
    game_pid: Arc<AtomicU32>,
    /// Set by the start and stop commands; overrides the recording mode until the mode changes.
    forced: Mutex<Option<bool>>,
    last_error: Mutex<Option<String>>,
    retry_at: Mutex<Instant>,
    sound: Mutex<SaveSound>,
    /// Held while a clip's name is picked and its file written. A name is free while the
    /// clip before it is still being written under a temporary one, so two saves in the same
    /// second (the hotkey and the tray) would pick the same name.
    saving: Mutex<()>,
    clip_hotkey: Mutex<Option<HotkeyListener>>,
    screenshot_hotkey: Mutex<Option<HotkeyListener>>,
    camera_hotkey: Mutex<Option<HotkeyListener>>,
    /// Switches input capture; none while unbound (the default).
    input_hotkey: Mutex<Option<HotkeyListener>>,
    input_key_tx: mpsc::Sender<()>,
    /// Keyboard and mouse input for the overlay, captured while switched on.
    input: Arc<crate::input::InputHub>,
    /// The game Umbra detected, for the input rules (0 for none).
    detected_pid: Arc<AtomicU32>,
    record_hotkey: Mutex<Option<HotkeyListener>>,
    session_hotkey: Mutex<Option<HotkeyListener>>,
    pause_hotkey: Mutex<Option<HotkeyListener>>,
    /// The recording to a file, next to the replay buffer.
    file: Mutex<Option<ActiveFile>>,
    /// A recording asked for; it starts as soon as the recorder runs.
    pending_file: Mutex<Option<RecordingKind>>,
    /// What a pending recording should record, when chosen for it rather than the capture.
    pending_source: Mutex<Option<CaptureChoice>>,
    /// The game whose session the user stopped, so it is not started again by itself.
    session_declined: AtomicU32,
    /// Free space on the clip folder's drive, looked at every five seconds: when, and whether
    /// it was under the limit.
    disk_checked: Mutex<Option<Instant>>,
    disk_low: AtomicBool,
    /// The low-space message was given for this time the drive ran low.
    disk_reported: AtomicBool,
    /// Whether the captured screen was in HDR when last looked at (every two seconds): what is
    /// captured, the answer and when.
    hdr_seen: Mutex<Option<(CaptureChoice, bool, Instant)>>,
    /// The captured window's program, looked up once a second: its exe, process id and when.
    followed: Mutex<Option<(String, u32, Instant)>>,
    /// The last game found, kept while its program runs even when it is not in front, so
    /// switching to another window does not end its session or its recording.
    sticky_game: Mutex<Option<GameInfo>>,
    /// Another program that fills the screen while the game still runs, and since when. It
    /// takes over only once it has done so for `TAKE_OVER`.
    challenger: Mutex<Option<(u32, Instant)>>,
    /// When a recording last carried on in a new file after an interruption; a second one
    /// right after (a disk that fails every write) stops it instead of looping.
    carried_on: Mutex<Option<Instant>>,
    /// True while the push-to-talk key is held; gates the microphone track.
    talking: Arc<AtomicBool>,
    /// The camera is filming (`settings.camera.enabled`); read by the recorder's camera.
    camera_on: Arc<AtomicBool>,
    push_to_talk: Mutex<Option<KeyHoldWatcher>>,
    usage: Mutex<Usage>,
    /// Where detectors send what they find; the auto-clip thread decides on clips.
    events: mpsc::SyncSender<GameEvent>,
    auto_log: Arc<autoclip::Log>,
    /// Counter-Strike 2's listener, with the settings it was started with.
    cs2: Mutex<Option<(crate::settings::Cs2Prefs, crate::cs2::Cs2Gsi)>>,
    /// The sound for an auto-clip, at its own volume.
    auto_sound: Mutex<SaveSound>,
    /// Set while the benchmark runs; nothing is recorded.
    paused: AtomicBool,
    stop: AtomicBool,
    sink: EventSink,
}

pub struct Controller {
    inner: Arc<Inner>,
    threads: Vec<JoinHandle<()>>,
}

/// Starts a thread that runs `work` every time `rx` receives, until the channel closes.
fn worker(
    name: &str,
    rx: mpsc::Receiver<()>,
    inner: Arc<Inner>,
    work: fn(&Inner) -> Result<()>,
) -> Result<JoinHandle<()>> {
    let label = name.to_string();
    Ok(std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Ok(()) = rx.recv() {
                if inner.stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(err) = work(&inner) {
                    log::warn!("{label} failed: {err:#}");
                }
            }
        })?)
}

impl Controller {
    /// Loads the settings from `settings_path`, registers the hotkeys and starts deciding
    /// when to record.
    pub fn start(
        settings_path: PathBuf,
        sink: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Self> {
        let settings = settings::load(&settings_path);
        let sink: EventSink = Arc::new(sink);
        let log_sink = sink.clone();
        let auto_log = Arc::new(autoclip::Log::new(move |entry| {
            log_sink(Event::AutoClipLog(entry.clone()))
        }));
        let (events, events_rx) = mpsc::sync_channel(autoclip::QUEUE);
        let (input_key_tx, input_key_rx) = mpsc::channel::<()>();
        let detected_pid = Arc::new(AtomicU32::new(0));
        let inner = Arc::new(Inner {
            settings_path,
            sound: Mutex::new(SaveSound::new(settings.clip.save_sound_volume)),
            saving: Mutex::new(()),
            settings: Mutex::new(settings.clone()),
            active: Mutex::new(None),
            previous: Mutex::new(None),
            game: Mutex::new(None),
            game_pid: Arc::new(AtomicU32::new(0)),
            forced: Mutex::new(None),
            last_error: Mutex::new(None),
            retry_at: Mutex::new(Instant::now()),
            clip_hotkey: Mutex::new(None),
            screenshot_hotkey: Mutex::new(None),
            camera_hotkey: Mutex::new(None),
            input_hotkey: Mutex::new(None),
            input_key_tx,
            input: Arc::new(crate::input::InputHub::new(detected_pid.clone())),
            detected_pid,
            record_hotkey: Mutex::new(None),
            session_hotkey: Mutex::new(None),
            pause_hotkey: Mutex::new(None),
            file: Mutex::new(None),
            pending_file: Mutex::new(None),
            pending_source: Mutex::new(None),
            session_declined: AtomicU32::new(0),
            hdr_seen: Mutex::new(None),
            followed: Mutex::new(None),
            disk_checked: Mutex::new(None),
            disk_low: AtomicBool::new(false),
            disk_reported: AtomicBool::new(false),
            sticky_game: Mutex::new(None),
            challenger: Mutex::new(None),
            carried_on: Mutex::new(None),
            talking: Arc::new(AtomicBool::new(false)),
            camera_on: Arc::new(AtomicBool::new(settings.camera.enabled)),
            push_to_talk: Mutex::new(None),
            usage: Mutex::new(Usage::default()),
            events,
            auto_log,
            cs2: Mutex::new(None),
            auto_sound: Mutex::new(SaveSound::new(settings.auto_clip.sound_volume)),
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            sink,
        });

        // The key pollers only signal; worker threads do the saving.
        let mut threads = Vec::new();
        let (clip_tx, clip_rx) = mpsc::channel::<()>();
        *inner.clip_hotkey.lock().unwrap() = Some(HotkeyListener::start(
            settings.clip.hotkey.parse()?,
            move || {
                let _ = clip_tx.send(());
            },
        )?);
        threads.push(worker(
            "umbra-save-clip",
            clip_rx,
            inner.clone(),
            |inner| inner.save_clip().map(|_| ()),
        )?);

        let (shot_tx, shot_rx) = mpsc::channel::<()>();
        *inner.screenshot_hotkey.lock().unwrap() = Some(HotkeyListener::start(
            settings.screenshot.hotkey.parse()?,
            move || {
                let _ = shot_tx.send(());
            },
        )?);
        threads.push(worker(
            "umbra-screenshot",
            shot_rx,
            inner.clone(),
            |inner| inner.take_screenshot().map(|_| ()),
        )?);

        let (camera_tx, camera_rx) = mpsc::channel::<()>();
        *inner.camera_hotkey.lock().unwrap() = Some(HotkeyListener::start(
            settings.camera.hotkey.parse()?,
            move || {
                let _ = camera_tx.send(());
            },
        )?);
        threads.push(worker(
            "umbra-camera-key",
            camera_rx,
            inner.clone(),
            |inner| inner.set_camera_on(None),
        )?);

        inner.update_input_hotkey(&settings);
        threads.push(worker(
            "umbra-input-key",
            input_key_rx,
            inner.clone(),
            |inner| inner.set_input_capture(None),
        )?);
        inner
            .input
            .set(settings.input.enabled, settings.input.rules.clone());

        let (record_tx, record_rx) = mpsc::channel::<()>();
        *inner.record_hotkey.lock().unwrap() = Some(HotkeyListener::start(
            settings.recording.record_hotkey.parse()?,
            move || {
                let _ = record_tx.send(());
            },
        )?);
        threads.push(worker("umbra-record", record_rx, inner.clone(), |inner| {
            inner.toggle_file(RecordingKind::Manual);
            Ok(())
        })?);

        let (session_tx, session_rx) = mpsc::channel::<()>();
        *inner.session_hotkey.lock().unwrap() = Some(HotkeyListener::start(
            settings.recording.session_hotkey.parse()?,
            move || {
                let _ = session_tx.send(());
            },
        )?);
        threads.push(worker(
            "umbra-session",
            session_rx,
            inner.clone(),
            |inner| {
                inner.toggle_file(RecordingKind::Session);
                Ok(())
            },
        )?);

        let (pause_tx, pause_rx) = mpsc::channel::<()>();
        *inner.pause_hotkey.lock().unwrap() = Some(HotkeyListener::start(
            settings.recording.pause_hotkey.parse()?,
            move || {
                let _ = pause_tx.send(());
            },
        )?);
        threads.push(worker("umbra-pause", pause_rx, inner.clone(), |inner| {
            inner.set_file_paused(None);
            Ok(())
        })?);

        inner.update_push_to_talk(&settings);

        let auto_inner = inner.clone();
        threads.push(
            std::thread::Builder::new()
                .name("umbra-auto-clip".into())
                .spawn(move || auto_inner.run_auto_clip(events_rx))?,
        );
        inner.update_detectors(&settings);

        // Recordings cut short by a crash or power loss are still playable; name them back.
        // Recordings still in fragments (from a crash, or from an older Umbra) are rewritten as
        // ordinary MP4 files, which open at once.
        let folder = PathBuf::from(&settings.storage.folder);
        let library_sink = inner.sink.clone();
        std::thread::spawn(move || {
            for path in crate::recorder::recover_recordings(&folder) {
                log::info!("recovered an unfinished recording: {}", path.display());
            }
            let mut changed = false;
            for path in fragmented_recordings(&folder) {
                let ordinary = path.with_extension("mp4.part");
                let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
                let kind = if name.is_some_and(|n| n.contains("(session)")) {
                    RecordingKind::Session
                } else {
                    RecordingKind::Manual
                };
                let game = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let size = std::fs::metadata(&path).map_or(0, |m| m.len());
                let job = crate::processing::begin(&path, &game, kind, size);
                library_sink(Event::ProcessingStarted);
                match crate::defrag::finish_in_place(&path, &mut |read| job.progress(read, size)) {
                    Ok(()) => {
                        log::info!("finished {} in place", path.display());
                        changed = true;
                        continue;
                    }
                    Err(err) => log::warn!(
                        "cannot finish {} in place, copying it: {err:#}",
                        path.display()
                    ),
                }
                let finalized =
                    crate::recorder::finalize_recording(&path, &ordinary, &mut |read| {
                        job.progress(read, size)
                    });
                match finalized {
                    Ok(()) => {
                        if std::fs::rename(&ordinary, &path).is_ok() {
                            log::info!("rewrote {} as an ordinary MP4", path.display());
                            changed = true;
                        }
                    }
                    Err(err) => {
                        log::warn!("cannot rewrite {}: {err:#}", path.display());
                        let _ = std::fs::remove_file(&ordinary);
                    }
                }
            }
            if changed {
                library_sink(Event::LibraryChanged);
            }
        });

        let tick_inner = inner.clone();
        threads.push(
            std::thread::Builder::new()
                .name("umbra-controller".into())
                .spawn(move || tick_inner.run())?,
        );
        Ok(Self { inner, threads })
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.lock().unwrap().clone()
    }

    /// Validates and applies new settings and stores them. Returns what was actually kept.
    pub fn set_settings(&self, new: Settings) -> Result<Settings> {
        let new = new.normalized();
        let old = std::mem::replace(&mut *self.inner.settings.lock().unwrap(), new.clone());

        let rebind = |slot: &Mutex<Option<HotkeyListener>>, before: &str, after: &str| {
            if before != after {
                if let (Some(listener), Ok(hotkey)) =
                    (slot.lock().unwrap().as_ref(), after.parse::<Hotkey>())
                {
                    listener.set_hotkey(hotkey);
                }
            }
        };
        rebind(&self.inner.clip_hotkey, &old.clip.hotkey, &new.clip.hotkey);
        rebind(
            &self.inner.screenshot_hotkey,
            &old.screenshot.hotkey,
            &new.screenshot.hotkey,
        );
        rebind(
            &self.inner.camera_hotkey,
            &old.camera.hotkey,
            &new.camera.hotkey,
        );
        rebind(
            &self.inner.record_hotkey,
            &old.recording.record_hotkey,
            &new.recording.record_hotkey,
        );
        rebind(
            &self.inner.session_hotkey,
            &old.recording.session_hotkey,
            &new.recording.session_hotkey,
        );
        rebind(
            &self.inner.pause_hotkey,
            &old.recording.pause_hotkey,
            &new.recording.pause_hotkey,
        );
        self.inner.update_push_to_talk(&new);
        self.inner.update_detectors(&new);
        self.inner.update_input_hotkey(&new);
        self.inner
            .input
            .set(new.input.enabled, new.input.rules.clone());
        if old.auto_clip.sound_volume != new.auto_clip.sound_volume {
            *self.inner.auto_sound.lock().unwrap() = SaveSound::new(new.auto_clip.sound_volume);
        }
        self.inner
            .camera_on
            .store(new.camera.enabled, Ordering::Relaxed);

        if old.clip.save_sound_volume != new.clip.save_sound_volume {
            *self.inner.sound.lock().unwrap() = SaveSound::new(new.clip.save_sound_volume);
        }
        if old.recording.mode != new.recording.mode {
            *self.inner.forced.lock().unwrap() = None;
        }
        if old.clip.buffer_secs() != new.clip.buffer_secs() {
            if let Some(active) = self.inner.active.lock().unwrap().as_ref() {
                active.recorder.set_buffer_secs(new.clip.buffer_secs());
            }
        }
        if old.recording.launch_at_startup != new.recording.launch_at_startup {
            if let Err(err) = apply_startup(&new) {
                log::warn!("cannot change launch at startup: {err:#}");
            }
        }
        settings::save(&self.inner.settings_path, &new)?;
        // Other changes are picked up by the next tick, which rebuilds the recorder if needed.
        Ok(new)
    }

    pub fn status(&self) -> Status {
        self.inner.status()
    }

    /// The game detected in front, if any.
    pub fn detected_game(&self) -> Option<GameInfo> {
        self.inner.game.lock().unwrap().clone()
    }

    /// Starts recording now, whatever the recording mode says.
    pub fn start_recording(&self) {
        *self.inner.forced.lock().unwrap() = Some(true);
        *self.inner.retry_at.lock().unwrap() = Instant::now();
    }

    /// Stops recording and drops the buffer, whatever the recording mode says.
    pub fn stop_recording(&self) {
        *self.inner.forced.lock().unwrap() = Some(false);
    }

    /// Stops recording until `pause(false)`, without changing what the user asked for. The
    /// benchmark uses this so the recorder does not compete for the encoder.
    pub fn pause(&self, paused: bool) {
        self.inner.paused.store(paused, Ordering::Relaxed);
        if paused {
            *self.inner.active.lock().unwrap() = None;
            *self.inner.previous.lock().unwrap() = None;
        } else {
            *self.inner.retry_at.lock().unwrap() = Instant::now();
        }
    }

    /// Switches the camera on or off (`None` switches it over) and remembers it. The recorder
    /// keeps running; the camera's stream is empty while it is off.
    pub fn set_camera_on(&self, on: Option<bool>) -> Result<()> {
        self.inner.set_camera_on(on)
    }

    pub fn save_clip(&self) -> Result<ClipEvent> {
        self.inner.save_clip()
    }

    /// Switches input capture on or off (`None` switches it over) and remembers it.
    pub fn set_input_capture(&self, on: Option<bool>) -> Result<()> {
        self.inner.set_input_capture(on)
    }

    /// The auto-clip log, oldest first.
    pub fn auto_clip_log(&self) -> Vec<autoclip::LogEntry> {
        self.inner.auto_log.entries()
    }

    /// Writes Counter-Strike 2's config file for the port and token in the settings, and
    /// stores the settings so the token stays the same.
    pub fn install_cs2_config(&self) -> Result<PathBuf> {
        let settings = self.settings();
        let cs2 = &settings.auto_clip.cs2;
        let path = crate::cs2::install_config(cs2.port, &cs2.token)?;
        settings::save(&self.inner.settings_path, &settings)?;
        self.inner.auto_log.add(
            None,
            format!(
                "CS2 config written to {}; restart CS2 to load it",
                path.display()
            ),
        );
        Ok(path)
    }

    /// Starts a recording to a file of what is being captured. The replay buffer keeps
    /// running, so clips can still be saved.
    pub fn start_file_recording(&self, kind: RecordingKind) {
        *self.inner.pending_source.lock().unwrap() = None;
        self.inner.request_file(kind);
    }

    /// Starts a recording of `source` (a screen or a window). When it is not what the replay
    /// buffer captures, the recording gets a recorder of its own and the buffer carries on.
    pub fn start_file_recording_of(&self, source: CaptureChoice) {
        if self.inner.file.lock().unwrap().is_some() {
            return;
        }
        *self.inner.pending_source.lock().unwrap() = Some(source);
        self.inner.request_file(RecordingKind::Manual);
    }

    /// Stops the recording to a file and keeps it.
    pub fn stop_file_recording(&self) {
        self.inner.stop_file(true, true);
    }

    /// Pauses (`true`) or resumes (`false`) the recording to a file, if one runs.
    pub fn pause_file_recording(&self, paused: bool) {
        self.inner.set_file_paused(Some(paused));
    }

    /// A recorder of what is captured with `video`, for the load test. It records nothing to
    /// disk; the caller drops it when done.
    pub fn recorder_for_test(&self, video: &VideoPrefs) -> Result<Recorder> {
        let settings = self.inner.settings.lock().unwrap().clone();
        let (target, _) = resolve_target(&settings.capture)?;
        let monitor = matches!(settings.capture, CaptureChoice::Monitor { .. });
        let config = self.inner.recorder_config(
            &settings,
            target,
            monitor,
            video,
            settings.clip.buffer_secs(),
        );
        Recorder::start(config)
    }

    pub fn is_file_recording(&self) -> bool {
        self.inner.file.lock().unwrap().is_some()
            || self.inner.pending_file.lock().unwrap().is_some()
    }

    pub fn take_screenshot(&self) -> Result<ScreenshotEvent> {
        self.inner.take_screenshot()
    }

    pub fn shutdown(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        // A recording in progress is closed properly before anything else goes. There is no
        // time to rewrite it now; the next start does that.
        self.inner.stop_file(false, false);
        self.inner.stop.store(true, Ordering::Relaxed);
        // The input hotkey's worker waits on a channel the controller itself holds open.
        let _ = self.inner.input_key_tx.send(());
        // Dropping the listeners ends their threads, which drops the senders and so ends
        // the worker threads.
        *self.inner.clip_hotkey.lock().unwrap() = None;
        *self.inner.screenshot_hotkey.lock().unwrap() = None;
        *self.inner.camera_hotkey.lock().unwrap() = None;
        *self.inner.input_hotkey.lock().unwrap() = None;
        self.inner.input.set(false, Vec::new());
        *self.inner.record_hotkey.lock().unwrap() = None;
        *self.inner.session_hotkey.lock().unwrap() = None;
        *self.inner.pause_hotkey.lock().unwrap() = None;
        *self.inner.push_to_talk.lock().unwrap() = None;
        *self.inner.cs2.lock().unwrap() = None;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        *self.inner.active.lock().unwrap() = None;
        *self.inner.previous.lock().unwrap() = None;
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Makes Windows' startup entry match the setting, pointing at this program. The app does
/// it at every launch too, so an entry that went missing, or points at another copy of
/// Umbra, is put right. Debug builds and test copies with their own settings folder
/// (`UMBRA_SETTINGS_DIR`) leave it alone: it belongs to the installed Umbra.
pub fn apply_startup(settings: &Settings) -> Result<()> {
    if cfg!(debug_assertions) || std::env::var_os("UMBRA_SETTINGS_DIR").is_some() {
        log::info!("launch at startup left alone (test build or settings folder)");
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let command = format!("\"{}\" --minimized", exe.display());
    let current = crate::startup::launch_command();
    if settings.recording.launch_at_startup {
        if current.as_deref() != Some(command.as_str()) {
            log::info!("startup entry set to {command} (was {current:?})");
            crate::startup::set_launch_at_startup(true, &command)?;
        }
    } else if current.is_some() {
        crate::startup::set_launch_at_startup(false, "")?;
    }
    Ok(())
}

/// True when both name the same screen or window (an empty device is the primary screen).
fn same_source(a: &CaptureChoice, b: &CaptureChoice) -> bool {
    match (a, b) {
        (CaptureChoice::Monitor { device: x }, CaptureChoice::Monitor { device: y }) => {
            let primary = || {
                list_monitors()
                    .into_iter()
                    .find(|m| m.primary)
                    .map(|m| m.device)
                    .unwrap_or_default()
            };
            let x = if x.is_empty() { primary() } else { x.clone() };
            let y = if y.is_empty() { primary() } else { y.clone() };
            x.eq_ignore_ascii_case(&y)
        }
        (
            CaptureChoice::Window { exe: e1, title: t1 },
            CaptureChoice::Window { exe: e2, title: t2 },
        ) => e1.eq_ignore_ascii_case(e2) && t1 == t2,
        _ => false,
    }
}

/// What to capture for the current settings, and a short description of it.
pub fn resolve_target(capture: &CaptureChoice) -> Result<(CaptureTarget, String)> {
    match capture {
        CaptureChoice::Monitor { device } => {
            let monitors = list_monitors();
            let index = monitors
                .iter()
                .position(|m| !device.is_empty() && m.device.eq_ignore_ascii_case(device))
                .or_else(|| monitors.iter().position(|m| m.primary))
                .or(if monitors.is_empty() { None } else { Some(0) })
                .ok_or_else(|| anyhow!("no monitor found"))?;
            let monitor = &monitors[index];
            // Numbered in list order, the same way the capture switcher names them.
            Ok((
                CaptureTarget::Monitor(monitor.handle),
                format!(
                    "Display {}, {}×{}",
                    index + 1,
                    monitor.width,
                    monitor.height
                ),
            ))
        }
        CaptureChoice::Window { exe, title } => {
            let window = find_window(exe, title)
                .ok_or_else(|| anyhow!("{exe} has no window to capture (is it running?)"))?;
            Ok((CaptureTarget::Window(window.handle), window.title))
        }
    }
}

impl Inner {
    fn run(self: &Arc<Self>) {
        let mut sampler = UsageSampler::new();
        let mut last_sample = Instant::now() - Duration::from_secs(10);
        let mut last_status: Option<String> = None;
        let mut last_emit = Instant::now() - Duration::from_secs(10);
        while !self.stop.load(Ordering::Relaxed) {
            self.tick();

            if last_sample.elapsed() >= Duration::from_secs(2) {
                last_sample = Instant::now();
                *self.usage.lock().unwrap() = sampler.sample();
            }

            let status = self.status();
            let summary = format!(
                "{:?}{:?}{:?}{}{}{}{}{}{}{}{:?}",
                status.state,
                status.error,
                status.game,
                status.mic_live,
                status.push_to_talk,
                status.width,
                status.height,
                status.fps,
                status.session_stopped,
                status.file.is_some(),
                status.file.as_ref().map(|f| f.paused)
            );
            // Send when something changed, once a second while recording so the buffer
            // figures move, and every two seconds otherwise so the usage figures do.
            let changed = last_status.as_ref() != Some(&summary);
            let period = if status.state == State::Recording {
                1
            } else {
                2
            };
            if changed || last_emit.elapsed() >= Duration::from_secs(period) {
                last_status = Some(summary);
                last_emit = Instant::now();
                (self.sink)(Event::Status(status));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Starts, stops or re-keys the push-to-talk watcher to match the settings.
    fn update_push_to_talk(&self, settings: &Settings) {
        let mic = &settings.audio.mic;
        let mut slot = self.push_to_talk.lock().unwrap();
        if !(mic.enabled && mic.push_to_talk) {
            *slot = None;
            self.talking.store(false, Ordering::Relaxed);
            return;
        }
        let Ok(key) = mic.push_to_talk_key.parse::<Hotkey>() else {
            return;
        };
        match slot.as_ref() {
            Some(watcher) => watcher.set_key(key),
            None => match KeyHoldWatcher::start(key, self.talking.clone()) {
                Ok(watcher) => *slot = Some(watcher),
                Err(err) => log::warn!("cannot start push-to-talk: {err:#}"),
            },
        }
    }

    fn tick(self: &Arc<Self>) {
        let settings = self.settings.lock().unwrap().clone();

        // Which game is running, and which program the "Game" audio track follows. Games in
        // the presets count as listed games.
        let mut listed = settings.recording.games.clone();
        listed.extend(
            settings
                .presets
                .iter()
                .flat_map(|p| p.games.iter().cloned()),
        );
        // The game a session records stays the game until it closes or the user stops it.
        let session_pid = self
            .file
            .lock()
            .unwrap()
            .as_ref()
            .filter(|f| f.kind == RecordingKind::Session)
            .map_or(0, |f| f.game_pid);
        let game = {
            let mut sticky = self.sticky_game.lock().unwrap();
            let never = &settings.recording.never_record;
            // A game added to "never record" while it runs stops counting at once.
            if sticky
                .as_ref()
                .is_some_and(|g| crate::apps::listed(never, &g.exe, g.pid))
            {
                *sticky = None;
            }
            // A program found only because it fills the screen does not replace a game that is
            // still running straight away: screenshot tools, overlays and the like cover the
            // screen for a moment, and a session would end for each of them.
            let detected = detect_game(&listed, never).filter(|found| {
                let game_running = sticky
                    .as_ref()
                    .is_some_and(|g| g.pid != found.pid && crate::games::is_running(g.pid, &g.exe));
                if game_running && sticky.as_ref().is_some_and(|g| g.pid == session_pid) {
                    return false;
                }
                let mut challenger = self.challenger.lock().unwrap();
                if !game_running || found.reason != crate::games::FILLS_THE_SCREEN {
                    *challenger = None;
                    return true;
                }
                let since = match *challenger {
                    Some((pid, at)) if pid == found.pid => at,
                    _ => {
                        *challenger = Some((found.pid, Instant::now()));
                        Instant::now()
                    }
                };
                since.elapsed() >= TAKE_OVER
            });
            match detected {
                Some(mut found) => {
                    // Seen from the background, the game keeps the screen it last filled.
                    if found.screen.is_none() && crate::games::foreground_pid() != found.pid {
                        found.screen = sticky
                            .as_ref()
                            .filter(|g| g.pid == found.pid)
                            .and_then(|g| g.screen);
                    }
                    if sticky.as_ref().is_none_or(|g| g.pid != found.pid) {
                        log::info!("game: {} ({}, {})", found.name, found.reason, found.exe);
                    }
                    *sticky = Some(found.clone());
                    Some(found)
                }
                None => {
                    if sticky
                        .as_ref()
                        .is_some_and(|g| !crate::games::is_running(g.pid, &g.exe))
                    {
                        *sticky = None;
                    }
                    sticky.clone()
                }
            }
        };
        let followed = match &settings.capture {
            CaptureChoice::Window { exe, .. } => self.window_program(exe),
            CaptureChoice::Monitor { .. } => game.as_ref().map_or(0, |g| g.pid),
        };
        self.game_pid.store(followed, Ordering::Relaxed);
        self.detected_pid
            .store(game.as_ref().map_or(0, |g| g.pid), Ordering::Relaxed);
        *self.game.lock().unwrap() = game.clone();

        self.drive_file(&settings, game.as_ref());
        // A recording with a recorder of its own (another screen or window) does not need
        // the replay buffer's.
        let recording_file = self
            .file
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|f| f.own.is_none())
            || (self.pending_file.lock().unwrap().is_some()
                && !self
                    .pending_source
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|c| !same_source(c, &settings.capture)));

        let wanted = !self.paused.load(Ordering::Relaxed)
            && (recording_file
                || match *self.forced.lock().unwrap() {
                    Some(forced) => forced,
                    None => match settings.recording.mode {
                        RecordingMode::Always => true,
                        RecordingMode::Games => {
                            game.is_some()
                                || matches!(settings.capture, CaptureChoice::Window { .. })
                                    && followed != 0
                        }
                        RecordingMode::Manual => false,
                    },
                });

        let mut active = self.active.lock().unwrap();

        // A recorder that stopped by itself, for example after a resolution change.
        if let Some(current) = active.as_ref() {
            if !current.recorder.is_running() {
                let message = current
                    .recorder
                    .error()
                    .unwrap_or_else(|| "recording stopped".into());
                log::warn!("recorder stopped: {message}");
                *self.last_error.lock().unwrap() = Some(message);
                // Its buffer still holds what led up to it (a game that crashed or rebuilt its
                // window): clips come from it until the new recorder holds as much.
                *self.previous.lock().unwrap() = active.take().map(|a| a.recorder);
                *self.retry_at.lock().unwrap() = Instant::now() + Duration::from_secs(3);
            }
        }

        if !wanted {
            if active.take().is_some() {
                log::info!("recording stopped");
            }
            *self.previous.lock().unwrap() = None;
            *self.last_error.lock().unwrap() = None;
            return;
        }

        let fingerprint = fingerprint(
            &settings,
            &self.effective_video(&settings).0,
            self.capture_in_hdr(&settings),
        );
        let base = self::fingerprint(&settings, &settings.video, false);
        if let Some(current) = active.as_ref() {
            if current.fingerprint == fingerprint {
                // The replaced recorder goes once the new one holds as much.
                let mut previous = self.previous.lock().unwrap();
                if previous.is_some()
                    && current.recorder.stats().buffer_secs >= settings.clip.buffer_secs() as f64
                {
                    *previous = None;
                }
                return;
            }
            log::info!("settings changed, restarting the recorder");
            // A recording can't change encoder settings mid-file: it is closed here and
            // carries on in a new file once the recorder runs again. One with a recorder of
            // its own is not affected.
            let on_shared = self
                .file
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|f| f.own.is_none());
            if on_shared {
                if let Some(kind) = self.finish_file(
                    Some(&current.recorder),
                    Some("the video settings changed; recording carries on in a new file"),
                    true,
                ) {
                    *self.pending_file.lock().unwrap() = Some(kind);
                }
            }
            let current = active.take().unwrap();
            // Only the game or the screen changed, not the user's settings: the old buffer is
            // kept until the new one is full.
            *self.previous.lock().unwrap() =
                (current.base == base && current.recorder.is_running()).then_some(current.recorder);
        }

        if Instant::now() < *self.retry_at.lock().unwrap() {
            return;
        }
        // Starting takes a while (each audio source can take seconds to open); saving a clip
        // and the status must not wait for it.
        drop(active);
        let built = self.build_recorder(&settings);
        let mut active = self.active.lock().unwrap();
        if self.paused.load(Ordering::Relaxed) || self.stop.load(Ordering::Relaxed) {
            return;
        }
        match built {
            Ok((recorder, target_label)) => {
                log::info!("recording {target_label}");
                *self.last_error.lock().unwrap() = None;
                *active = Some(Active {
                    recorder: Arc::new(recorder),
                    fingerprint,
                    base,
                    target_label,
                });
            }
            Err(err) => {
                log::warn!("cannot start recording: {err:#}");
                *self.last_error.lock().unwrap() = Some(format!("{err:#}"));
                *self.retry_at.lock().unwrap() = Instant::now() + Duration::from_secs(5);
            }
        }
    }

    /// The program whose video settings apply, and its path if known: a captured window's,
    /// else the game found.
    fn recorded_exe(&self, settings: &Settings) -> Option<(String, Option<String>)> {
        match &settings.capture {
            CaptureChoice::Window { exe, .. } => Some((exe.clone(), None)),
            CaptureChoice::Monitor { .. } => self
                .game
                .lock()
                .unwrap()
                .as_ref()
                .map(|g| (g.exe.clone(), g.path.clone())),
        }
    }

    /// The video settings in effect: a preset's when the recorded program is in one.
    fn effective_video(&self, settings: &Settings) -> (VideoPrefs, Option<String>) {
        let exe = self.recorded_exe(settings);
        let (video, preset) = settings.video_for(
            exe.as_ref().map(|(e, _)| e.as_str()),
            exe.as_ref().and_then(|(_, p)| p.as_deref()),
        );
        (video.clone(), preset.map(|p| p.name.clone()))
    }

    /// A recorder of the game's own window, for a session, with the game's video settings.
    fn game_window_recorder(&self, settings: &Settings, game: &GameInfo) -> Result<Recorder> {
        let window = find_window(&game.exe, &game.title)
            .ok_or_else(|| anyhow!("{} has no window to record", game.exe))?;
        let (video, _) = settings.video_for(Some(&game.exe), game.path.as_deref());
        let mut config = self.recorder_config(
            settings,
            CaptureTarget::Window(window.handle),
            false,
            video,
            // Only the file is wanted from it; a short buffer is enough to start on a keyframe.
            3,
        );
        config.buffer_dir = None;
        // Started while the game is minimized (a session restarted from Umbra's window): the
        // window is a strip of title bar, so take the size the game is played at instead.
        if config.output_size.is_none() && crate::wnd::is_minimized(window.handle) {
            config.output_size = game
                .screen
                .and_then(|m| screen_size(&CaptureTarget::Monitor(m)))
                .or_else(|| crate::wnd::restored_size(window.handle));
        }
        Recorder::start(config)
    }

    /// A recorder for a recording of `choice`, next to the replay buffer's.
    fn source_recorder(&self, settings: &Settings, choice: &CaptureChoice) -> Result<Recorder> {
        let (target, _) = resolve_target(choice)?;
        let video = match choice {
            CaptureChoice::Window { exe, .. } => settings.video_for(Some(exe), None).0.clone(),
            CaptureChoice::Monitor { .. } => self.effective_video(settings).0,
        };
        let monitor = matches!(choice, CaptureChoice::Monitor { .. });
        // Only the file is wanted from it; a short buffer is enough to start on a keyframe.
        let mut config = self.recorder_config(settings, target, monitor, &video, 3);
        config.buffer_dir = None;
        if let CaptureTarget::Window(handle) = target {
            if config.output_size.is_none() && crate::wnd::is_minimized(handle) {
                config.output_size = crate::wnd::restored_size(handle);
            }
        }
        Recorder::start(config)
    }

    fn build_recorder(&self, settings: &Settings) -> Result<(Recorder, String)> {
        let (target, label) = resolve_target(&settings.capture)?;
        let (video, _) = self.effective_video(settings);
        let monitor = matches!(settings.capture, CaptureChoice::Monitor { .. });
        let config = self.recorder_config(
            settings,
            target,
            monitor,
            &video,
            settings.clip.buffer_secs(),
        );
        Ok((Recorder::start(config)?, label))
    }

    fn recorder_config(
        &self,
        settings: &Settings,
        target: CaptureTarget,
        monitor: bool,
        video: &VideoPrefs,
        buffer_secs: u32,
    ) -> RecorderConfig {
        let rate = match video.rate_mode {
            RateMode::Cbr => RateControl::Cbr {
                kbps: video.bitrate_kbps,
            },
            RateMode::Vbr => RateControl::Vbr {
                kbps: video.bitrate_kbps,
                max_kbps: video.max_bitrate_kbps,
            },
            RateMode::Cq => RateControl::ConstQuality {
                cq: video.cq,
                max_kbps: video.max_bitrate_kbps,
            },
            RateMode::Cqp => RateControl::ConstQp { qp: video.qp },
        };
        let output_size = match (video.width, video.height) {
            (Some(w), Some(h)) => Some((w, h)),
            // A whole screen is recorded at the monitor's own resolution, so a game that
            // switches the screen to a stretched or lower mode is recorded the way it looks.
            _ if monitor => screen_size(&target),
            // So is a window filling the screen: a game running a stretched 1440×1080 is
            // recorded stretched (or fitted) to the monitor, not at its own 4:3 size.
            _ => match target {
                CaptureTarget::Window(window) => games::filled_monitor(window)
                    .and_then(|m| screen_size(&CaptureTarget::Monitor(m))),
                CaptureTarget::Monitor(_) => None,
            },
        };
        // A screen in HDR: captured as such and converted to SDR, or kept as HDR10 where the
        // codec can carry it.
        let screen = match target {
            CaptureTarget::Monitor(m) => m,
            CaptureTarget::Window(w) => games::monitor_of(w),
        };
        let hdr = crate::hdr::monitor_hdr(screen).map(|info| {
            let keep = video.hdr == settings::HdrChoice::Hdr10
                && matches!(video.codec, CodecChoice::Hevc | CodecChoice::Av1);
            let output = if keep {
                crate::hdr::HdrOutput::Hdr10
            } else {
                crate::hdr::HdrOutput::Sdr
            };
            (info, output)
        });
        RecorderConfig {
            hdr,
            target,
            codec: match video.codec {
                CodecChoice::H264 => VideoCodec::H264,
                CodecChoice::Hevc => VideoCodec::Hevc,
                CodecChoice::Av1 => VideoCodec::Av1,
            },
            fps: video.fps,
            rate,
            preset: video.preset,
            capture_cursor: video.capture_cursor,
            output_size,
            scaling: video.scaling,
            keyframe_secs: video.keyframe_secs,
            tuning: tuning(video),
            skip_still: video.skip_still,
            buffer_secs,
            buffer_dir: settings.clip.buffer_dir(),
            audio: self.audio_config(settings),
            camera: settings
                .camera
                .device_id
                .clone()
                .map(|id| crate::camera::CameraConfig {
                    input: crate::camera::CameraInput::Device(id),
                    width: settings.camera.width,
                    height: settings.camera.height,
                    max_fps: video.fps,
                    codec: match video.codec {
                        CodecChoice::H264 => VideoCodec::H264,
                        CodecChoice::Hevc => VideoCodec::Hevc,
                        CodecChoice::Av1 => VideoCodec::Av1,
                    },
                    preset: video.preset,
                    on: self.camera_on.clone(),
                }),
            input: Some(self.input.clone()),
        }
    }

    fn audio_config(&self, settings: &Settings) -> AudioConfig {
        let audio = &settings.audio;
        let mut tracks = Vec::new();
        let track = |title: String, source: AudioSource, in_mix: bool| AudioTrackConfig {
            title,
            source,
            in_mix,
            gate: None,
        };
        match audio.mode {
            AudioMode::Split => {
                if audio.game {
                    tracks.push(track(
                        "Game".into(),
                        AudioSource::GameProcess {
                            pid: self.game_pid.clone(),
                        },
                        true,
                    ));
                }
                for exe in &audio.apps {
                    tracks.push(track(
                        naming::game_name_from_exe(exe),
                        AudioSource::ProcessByName { exe: exe.clone() },
                        true,
                    ));
                }
                if audio.others {
                    // Built without the game and the apps above, so nothing is in two tracks.
                    tracks.push(track(
                        "Other apps".into(),
                        AudioSource::OtherApps {
                            game: self.game_pid.clone(),
                            except: audio.apps.clone(),
                        },
                        true,
                    ));
                }
            }
            AudioMode::System => tracks.push(track(
                "System".into(),
                AudioSource::AllExceptTree {
                    pid: std::process::id(),
                },
                true,
            )),
        }
        if audio.mic.enabled {
            // Always on its own track; in the combined one too only when asked for.
            let mut mic = track(
                "Microphone".into(),
                AudioSource::Microphone {
                    device_id: audio.mic.device_id.clone(),
                },
                audio.mic.in_mix,
            );
            if audio.mic.push_to_talk {
                mic.gate = Some(self.talking.clone());
            }
            tracks.push(mic);
        }
        AudioConfig {
            // A combined first track plays in every player and in link previews. It is only
            // worth having when it combines something.
            mix: tracks.iter().filter(|t| t.in_mix).count() > 1,
            tracks,
            bitrate_kbps: audio.bitrate_kbps,
        }
    }

    fn status(&self) -> Status {
        let (game, game_reason) = match self.game.lock().unwrap().as_ref() {
            Some(g) => (Some(g.name.clone()), Some(g.reason.clone())),
            None => (None, None),
        };
        let (mic_enabled, push_to_talk) = {
            let settings = self.settings.lock().unwrap();
            (
                settings.audio.mic.enabled,
                settings.audio.mic.enabled && settings.audio.mic.push_to_talk,
            )
        };
        let usage = *self.usage.lock().unwrap();
        let game_pid = self.game.lock().unwrap().as_ref().map_or(0, |g| g.pid);
        let session_stopped = game_pid != 0
            && self.session_declined.load(Ordering::Relaxed) == game_pid
            && self.file.lock().unwrap().is_none()
            && self.pending_file.lock().unwrap().is_none();
        let file = self.file.lock().unwrap().as_ref().map(|f| FileStatus {
            kind: f.kind,
            secs: f.file.secs(),
            bytes: f.file.bytes(),
            game: f.game.clone(),
            path: f.file.path.clone(),
            source: f.source.clone(),
            paused: f.file.is_paused(),
        });
        let active = self.active.lock().unwrap();
        let error = self.last_error.lock().unwrap().clone();
        match active.as_ref() {
            Some(active) => {
                let info = active.recorder.info();
                let stats = active.recorder.stats();
                let audio_problems = active.recorder.audio_problems();
                let camera_on = self.camera_on.load(Ordering::Relaxed);
                let camera_problem = active.recorder.camera_problem().filter(|_| camera_on);
                // Clips come from the replaced recorder while it holds more (see `previous`).
                let held = self
                    .previous
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map_or(0.0, |p| p.stats().buffer_secs);
                let mic_missing = audio_problems.iter().any(|(t, _)| t == "Microphone");
                Status {
                    state: State::Recording,
                    error,
                    target: Some(active.target_label.clone()),
                    game,
                    game_reason: game_reason.clone(),
                    encoder: Some(info.encoder.clone()),
                    hdr: info.hdr.map(|h| match h {
                        crate::hdr::HdrOutput::Sdr => "sdr".to_string(),
                        crate::hdr::HdrOutput::Hdr10 => "hdr10".to_string(),
                    }),
                    width: info.width,
                    height: info.height,
                    fps: info.fps,
                    buffer_secs: stats.buffer_secs.max(held),
                    buffer_bytes: stats.buffer_bytes,
                    buffer_on_disk: stats.buffer_on_disk,
                    mic_live: mic_enabled
                        && !mic_missing
                        && (!push_to_talk || self.talking.load(Ordering::Relaxed)),
                    push_to_talk,
                    dropped_frames: stats.dropped_frames,
                    usage,
                    file,
                    session_stopped,
                    audio_problems,
                    camera_on,
                    camera_live: camera_on && info.camera.is_some() && camera_problem.is_none(),
                    camera_problem,
                    input: self.input.status(),
                }
            }
            None => Status {
                state: if error.is_some() {
                    State::Error
                } else {
                    State::Idle
                },
                error,
                game,
                game_reason,
                push_to_talk,
                usage,
                file,
                session_stopped,
                camera_on: self.camera_on.load(Ordering::Relaxed),
                input: self.input.status(),
                ..Status::idle()
            },
        }
    }

    /// The game a clip or screenshot belongs to: a captured window's program, or the game in
    /// front of a captured monitor, or `Desktop`.
    fn subject(&self, capture: &CaptureChoice) -> String {
        match capture {
            CaptureChoice::Window { exe, .. } => naming::game_name_from_exe(exe),
            CaptureChoice::Monitor { .. } => self
                .game
                .lock()
                .unwrap()
                .as_ref()
                .map(|g| naming::game_name_from_exe(&g.exe))
                .unwrap_or_else(|| "Desktop".into()),
        }
    }

    /// What a clip records about how it was made, shown in its properties.
    fn clip_tags(
        &self,
        settings: &Settings,
        game: &str,
        recorder: &Recorder,
    ) -> Vec<(String, String)> {
        let (video, preset) = self.effective_video(settings);
        let video = &video;
        let info = recorder.info();
        let mut tags = vec![
            ("game".to_string(), game.to_string()),
            ("encoder".into(), info.encoder.clone()),
            ("gpu".into(), info.adapter.clone()),
            ("app".into(), format!("Umbra {}", env!("CARGO_PKG_VERSION"))),
        ];
        match &settings.capture {
            CaptureChoice::Window { exe, title } => {
                tags.push(("game_exe".into(), exe.clone()));
                tags.push(("window".into(), title.clone()));
                tags.push(("detected".into(), "captured window".into()));
            }
            CaptureChoice::Monitor { .. } => {
                if let Some(found) = self.game.lock().unwrap().as_ref() {
                    tags.push(("game_exe".into(), found.exe.clone()));
                    if !found.title.is_empty() {
                        tags.push(("window".into(), found.title.clone()));
                    }
                    tags.push((
                        "detected".into(),
                        if settings
                            .recording
                            .games
                            .iter()
                            .any(|g| g.eq_ignore_ascii_case(&found.exe))
                        {
                            "listed game".into()
                        } else {
                            "fullscreen program".into()
                        },
                    ));
                } else {
                    tags.push(("detected".into(), "no game".into()));
                }
            }
        }
        if let Some(active) = self.active.lock().unwrap().as_ref() {
            tags.push(("source".into(), active.target_label.clone()));
        }
        let rate = match video.rate_mode {
            RateMode::Cbr => format!("CBR {} kbit/s", video.bitrate_kbps),
            RateMode::Vbr => format!(
                "VBR {} kbit/s, up to {} kbit/s",
                video.bitrate_kbps, video.max_bitrate_kbps
            ),
            RateMode::Cq => format!(
                "Constant quality {}, up to {} kbit/s",
                video.cq, video.max_bitrate_kbps
            ),
            RateMode::Cqp => format!("Constant QP {}", video.qp),
        };
        tags.push(("rate".into(), rate));
        tags.push(("preset".into(), format!("P{}", video.preset)));
        if let Some(name) = preset {
            tags.push(("video_preset".into(), name));
        }
        tags.push(("recorded_fps".into(), info.fps.to_string()));
        // Where the camera shows, until it is moved in the editor.
        if info.camera.is_some() {
            if let Ok(placement) = serde_json::to_string(&settings.camera.placement) {
                tags.push(("camera".into(), placement));
            }
        }
        // How the input overlay looks on this clip, until changed in the editor.
        if settings.input.enabled {
            if let Ok(overlay) = serde_json::to_string(&settings.input.overlay) {
                tags.push(("input".into(), overlay));
            }
        }
        // The logo, kept with the clip so choosing another later does not change it.
        if settings.logo.path.is_some() {
            if let Ok(logo) = serde_json::to_string(&settings.logo) {
                tags.push(("logo".into(), logo));
            }
        }
        // Which tracks the combined first track is made of, so the editor and the export
        // know without guessing from the names.
        let audio = self.audio_config(settings);
        if audio.mix {
            let parts: Vec<&str> = audio
                .tracks
                .iter()
                .filter(|t| t.in_mix)
                .map(|t| t.title.as_str())
                .collect();
            tags.push(("mix".into(), parts.join("|")));
        }
        tags
    }

    fn play_sound(&self, wanted: bool) {
        let volume = self.settings.lock().unwrap().clip.save_sound_volume;
        if wanted && volume > 0.0 {
            self.sound.lock().unwrap().play();
        }
    }

    fn save_clip(&self) -> Result<ClipEvent> {
        let result = self.save_clip_with(None);
        if let Err(err) = &result {
            (self.sink)(Event::ClipFailed {
                error: format!("{err:#}"),
            });
        }
        result
    }

    /// Saves a clip: the hotkey's (the whole buffer, after the wait set for it), or with
    /// `auto` the stretch auto-clip decided on, named after what happened.
    fn save_clip_with(&self, auto: Option<AutoClip>) -> Result<ClipEvent> {
        let recorder = self
            .clip_source()
            .ok_or_else(|| anyhow!("not recording, so there is nothing to save"))?;
        let settings = self.settings.lock().unwrap().clone();
        // While recording to a file, a clip also marks the moment in the recording: when the
        // hotkey was pressed, before any wait, or when the auto-clip's first event happened.
        if let Some(active) = self.file.lock().unwrap().as_mut() {
            let ago = auto
                .as_ref()
                .map_or(0.0, |a| a.moment.elapsed().as_secs_f64());
            active.bookmarks.push((active.file.secs() - ago).max(0.0));
        }
        // Keep recording a little longer, so what happens right after is in the clip too.
        // Auto-clip has waited for that already.
        let post = if auto.is_some() {
            0
        } else {
            settings.clip.post_secs
        };
        let game = self.subject(&settings.capture);
        (self.sink)(Event::ClipPending {
            secs: post,
            game: game.clone(),
            length_secs: match &auto {
                Some(a) => a.span.1.saturating_duration_since(a.span.0).as_secs() as u32,
                None => settings.clip.length_secs + post,
            },
        });
        if post > 0 {
            std::thread::sleep(Duration::from_secs(post as u64));
        }
        // Input reaches the buffer a moment late (the rules wait for it); the clip waits too,
        // so it has input to its end.
        self.input.wait_handed_out(Duration::from_millis(800));
        let one_at_a_time = self.saving.lock().unwrap();
        let path = match &auto {
            Some(a) => {
                naming::event_clip_path(&settings.storage, &game, local_datetime(), &a.label)
            }
            None => naming::clip_path(&settings.storage, &game, local_datetime()),
        };
        let mut tags = self.clip_tags(&settings, &game, &recorder);
        if let Some(a) = &auto {
            tags.extend(a.tags.iter().cloned());
        }
        let clip = recorder.save_clip(&path, &tags, auto.as_ref().map(|a| a.span))?;
        drop(one_at_a_time);
        if auto.is_some() {
            if settings.auto_clip.sound_volume > 0.0 {
                self.auto_sound.lock().unwrap().play();
            }
        } else {
            self.play_sound(settings.clip.save_sound);
        }
        let event = ClipEvent { clip, game };
        (self.sink)(Event::ClipSaved(event.clone()));
        Ok(event)
    }

    /// Turns what the detectors find into clips (see [`autoclip::Policy`]) until Umbra stops.
    fn run_auto_clip(&self, events: mpsc::Receiver<GameEvent>) {
        // The policy counts seconds from here.
        let base = Instant::now();
        let secs = |at: Instant| at.saturating_duration_since(base).as_secs_f64();
        let instant = |t: f64| {
            if t >= 0.0 {
                base + Duration::from_secs_f64(t)
            } else {
                base.checked_sub(Duration::from_secs_f64(-t))
                    .unwrap_or(base)
            }
        };
        let mut policy = autoclip::Policy::new(Default::default(), 0.0);
        while !self.stop.load(Ordering::Relaxed) {
            // Awake for the next clip that is due, and once a second to see if Umbra stops.
            let wait = policy
                .next_deadline()
                .map_or(1.0, |due| (due - secs(Instant::now())).clamp(0.0, 1.0));
            let received = events.recv_timeout(Duration::from_secs_f64(wait)).ok();
            {
                let settings = self.settings.lock().unwrap();
                policy.prefs = settings.auto_clip.clone();
                policy.max_secs = settings.clip.buffer_secs() as f64;
            }
            let now = secs(Instant::now());
            if let Some(event) = received {
                let note = match policy.push(secs(event.at), event.clone(), now) {
                    Ok(()) if event.kind == autoclip::EventKind::RoundDone => {
                        "end of the round".into()
                    }
                    Ok(()) => match policy.open_events() {
                        1 => "first event, waiting for more".to_string(),
                        n => format!("{n} events so far"),
                    },
                    Err(why) => format!("left out: {why}"),
                };
                self.auto_log.add(Some(&event), note);
            }
            for decision in policy.take(now) {
                let plan = match decision {
                    autoclip::Decision::Save(plan) => plan,
                    autoclip::Decision::Skip { label, why } => {
                        let what = if label.is_empty() { "this" } else { &label };
                        self.auto_log.add(None, format!("no clip of {what}: {why}"));
                        continue;
                    }
                };
                let label = plan.label();
                let request = AutoClip {
                    span: (instant(plan.from), instant(plan.to)),
                    moment: instant(plan.events.first().map_or(plan.from, |(t, _)| *t)),
                    label: label.clone(),
                    tags: plan.tags(),
                };
                match self.save_clip_with(Some(request)) {
                    Ok(saved) => self.auto_log.add(
                        None,
                        format!(
                            "saved {} ({:.1} s, {} events)",
                            saved
                                .clip
                                .path
                                .file_name()
                                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                            saved.clip.duration_secs,
                            plan.events.len()
                        ),
                    ),
                    Err(err) => self
                        .auto_log
                        .add(None, format!("{label} not saved: {err:#}")),
                }
            }
        }
    }

    /// Starts or stops the detectors to match the settings.
    fn update_detectors(&self, settings: &Settings) {
        let auto = &settings.auto_clip;
        let wanted = (auto.enabled && auto.cs2.enabled).then(|| auto.cs2.clone());
        let mut slot = self.cs2.lock().unwrap();
        if slot.as_ref().map(|(prefs, _)| prefs) == wanted.as_ref() {
            return;
        }
        // The old listener goes first, freeing its port.
        if slot.take().is_some() {
            self.auto_log.add(None, "CS2: stopped listening");
        }
        let Some(prefs) = wanted else {
            return;
        };
        let port = prefs.port;
        match crate::cs2::Cs2Gsi::start(
            port,
            prefs.token.clone(),
            prefs.modes.clone(),
            self.events.clone(),
            self.auto_log.clone(),
        ) {
            Ok(gsi) => {
                self.auto_log
                    .add(None, format!("CS2: listening on 127.0.0.1:{port}"));
                *slot = Some((prefs, gsi));
            }
            Err(err) => self.auto_log.add(None, format!("CS2: {err:#}")),
        }
    }

    /// The recorder a clip comes from: the running one, or right after a game's preset or the
    /// screen's HDR changed, the one it replaced while that still holds more.
    fn clip_source(&self) -> Option<Arc<Recorder>> {
        let active = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.recorder.clone());
        let previous = self.previous.lock().unwrap().clone();
        match (active, previous) {
            (Some(a), Some(p)) if p.stats().buffer_secs > a.stats().buffer_secs => Some(p),
            (a, p) => a.or(p),
        }
    }

    /// Asks for a recording to a file; the tick starts it once the recorder runs.
    fn request_file(&self, kind: RecordingKind) {
        if self.file.lock().unwrap().is_some() {
            return;
        }
        *self.pending_file.lock().unwrap() = Some(kind);
        *self.retry_at.lock().unwrap() = Instant::now();
    }

    /// Switches input capture on or off (`None` switches it over) and remembers it.
    fn set_input_capture(&self, on: Option<bool>) -> Result<()> {
        let mut settings = self.settings.lock().unwrap();
        let on = on.unwrap_or(!settings.input.enabled);
        settings.input.enabled = on;
        self.input.set(on, settings.input.rules.clone());
        log::info!("input capture {}", if on { "on" } else { "off" });
        settings::save(&self.settings_path, &settings)
    }

    /// Starts, stops or re-keys the input hotkey to match the settings; unbound is none.
    fn update_input_hotkey(&self, settings: &Settings) {
        let mut slot = self.input_hotkey.lock().unwrap();
        let Ok(key) = settings.input.hotkey.parse::<Hotkey>() else {
            *slot = None;
            return;
        };
        match slot.as_ref() {
            Some(listener) => listener.set_hotkey(key),
            None => {
                let tx = self.input_key_tx.clone();
                match HotkeyListener::start(key, move || {
                    let _ = tx.send(());
                }) {
                    Ok(listener) => *slot = Some(listener),
                    Err(err) => log::warn!("cannot start the input hotkey: {err:#}"),
                }
            }
        }
    }

    fn set_camera_on(&self, on: Option<bool>) -> Result<()> {
        let mut settings = self.settings.lock().unwrap();
        let on = on.unwrap_or(!settings.camera.enabled);
        settings.camera.enabled = on;
        self.camera_on.store(on, Ordering::Relaxed);
        settings::save(&self.settings_path, &settings)
    }

    /// Pauses or resumes the running recording; `None` switches between the two.
    fn set_file_paused(&self, paused: Option<bool>) {
        let changed = {
            let file = self.file.lock().unwrap();
            let Some(active) = file.as_ref() else {
                return;
            };
            let now = paused.unwrap_or(!active.file.is_paused());
            if now == active.file.is_paused() {
                return;
            }
            if now {
                active.file.pause();
            } else {
                active.file.resume();
            }
            log::info!("recording {}", if now { "paused" } else { "resumed" });
            (now, active.kind, active.game.clone())
        };
        let (paused, kind, game) = changed;
        (self.sink)(Event::RecordingPaused { paused, kind, game });
    }

    /// The hotkeys: stop whatever recording runs, or start one of `kind`.
    fn toggle_file(&self, kind: RecordingKind) {
        let running =
            self.file.lock().unwrap().is_some() || self.pending_file.lock().unwrap().is_some();
        if running {
            self.stop_file(true, true);
        } else {
            self.request_file(kind);
        }
    }

    /// Stops the recording (or a pending start) and keeps the file. `by_user` means the user
    /// asked: a game session stopped by hand is not started again until the game restarts.
    fn stop_file(&self, by_user: bool, background: bool) {
        *self.pending_file.lock().unwrap() = None;
        *self.pending_source.lock().unwrap() = None;
        if by_user {
            if let Some(active) = self.file.lock().unwrap().as_ref() {
                if active.kind == RecordingKind::Session && active.game_pid != 0 {
                    self.session_declined
                        .store(active.game_pid, Ordering::Relaxed);
                }
            }
        }
        let recorder = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.recorder.clone());
        self.finish_file(recorder.as_deref(), None, background);
    }

    /// Closes the running recording and reports it. Returns its kind, if there was one.
    /// `recorder` (if still there) stops feeding it first, so nothing recorded is cut off.
    /// With `background`, the file is rewritten as an ordinary MP4 on another thread and
    /// `RecordingSaved` follows when that is done.
    fn finish_file(
        &self,
        recorder: Option<&Recorder>,
        reason: Option<&str>,
        background: bool,
    ) -> Option<RecordingKind> {
        let active = self.file.lock().unwrap().take()?;
        if let Some(feeder) = active.own.as_deref().or(recorder) {
            feeder.end_file();
        }
        let kind = active.kind;
        let reason = reason.map(str::to_string);
        if background {
            (self.sink)(Event::RecordingFinishing {
                kind,
                game: active.game.clone(),
            });
            let sink = self.sink.clone();
            let spawned = std::thread::Builder::new()
                .name("umbra-finish-recording".into())
                .spawn(move || complete_file(active, true, reason, &sink));
            if let Err(err) = spawned {
                log::warn!("cannot finish the recording in the background: {err}");
            }
        } else {
            complete_file(active, false, reason, &self.sink);
        }
        Some(kind)
    }

    /// Starts, ends and checks on the recording to a file; called every tick.
    /// Checks the clip folder's drive every five seconds. Under the limit, a recording to a
    /// file is stopped and kept, and none starts until there is room again (1 GB over the
    /// limit, so it does not flicker on and off).
    fn guard_disk(&self, settings: &Settings) {
        let limit = settings.storage.min_free_gb as u64 * (1u64 << 30);
        if limit == 0 {
            self.disk_low.store(false, Ordering::Relaxed);
            return;
        }
        {
            let mut checked = self.disk_checked.lock().unwrap();
            if checked.is_some_and(|at| at.elapsed() < Duration::from_secs(5)) {
                return;
            }
            *checked = Some(Instant::now());
        }
        let Some(free) = crate::disk::free_bytes(std::path::Path::new(&settings.storage.folder))
        else {
            return;
        };
        let was_low = self.disk_low.load(Ordering::Relaxed);
        let low = free < limit || (was_low && free < limit + (1u64 << 30));
        self.disk_low.store(low, Ordering::Relaxed);
        if !low {
            self.disk_reported.store(false, Ordering::Relaxed);
            return;
        }
        let gb = free as f64 / (1u64 << 30) as f64;
        let reason = format!("the disk is almost full ({gb:.1} GB left)");
        let recorder = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.recorder.clone());
        *self.pending_file.lock().unwrap() = None;
        if let Some(kind) = self.finish_file(recorder.as_deref(), Some(&reason), true) {
            log::warn!("recording stopped: {reason}");
            self.disk_reported.store(true, Ordering::Relaxed);
            (self.sink)(Event::LowDisk {
                free_bytes: free,
                limit_bytes: limit,
                stopped: Some(kind),
            });
        }
    }

    fn drive_file(&self, settings: &Settings, game: Option<&GameInfo>) {
        self.guard_disk(settings);
        let game_pid = game.map_or(0, |g| g.pid);
        if game_pid == 0 {
            self.session_declined.store(0, Ordering::Relaxed);
        }

        // Writing stopped by itself: the recorder went away, or the disk is full.
        let ended = self
            .file
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|f| f.file.has_ended());
        // A session's own recorder stops when the game's window goes away.
        let own_stopped = self
            .file
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|f| f.own.as_ref())
            .is_some_and(|r| !r.is_running());
        if ended || own_stopped {
            // Only the user or the game closing ends a recording: one interrupted (the capture
            // started again after a change of screen mode, the game made a new window)
            // carries on in a new file, as long as its game still runs.
            let (kind, session_game, own_source) = {
                let file = self.file.lock().unwrap();
                let f = file.as_ref().unwrap();
                (
                    f.kind,
                    f.game_pid,
                    f.own.is_some().then(|| f.source.clone()),
                )
            };
            let mut carried_on = self.carried_on.lock().unwrap();
            let again_at_once = carried_on.is_some_and(|at| at.elapsed() < Duration::from_secs(10));
            let carry_on = !again_at_once
                && match kind {
                    RecordingKind::Session => session_game != 0 && session_game == game_pid,
                    // A recording of a chosen window ends with that window; a chosen screen
                    // is still there.
                    RecordingKind::Manual => {
                        !matches!(own_source, Some(CaptureChoice::Window { .. }))
                    }
                };
            let reason = if carry_on {
                "recording was interrupted; it carries on in a new file"
            } else {
                "recording was interrupted"
            };
            self.finish_file(None, Some(reason), true);
            if carry_on {
                log::info!("recording carries on in a new file");
                *carried_on = Some(Instant::now());
                if kind == RecordingKind::Manual {
                    *self.pending_source.lock().unwrap() = own_source;
                }
                *self.pending_file.lock().unwrap() = Some(kind);
            }
        }

        // A game session ends with its game.
        let session_over = self.file.lock().unwrap().as_ref().is_some_and(|f| {
            f.kind == RecordingKind::Session && f.game_pid != 0 && f.game_pid != game_pid
        });
        if session_over {
            self.stop_file(false, true);
        }

        // Full sessions start by themselves when a game appears.
        if settings.recording.full_sessions
            && game_pid != 0
            && self.session_declined.load(Ordering::Relaxed) != game_pid
            && self.file.lock().unwrap().is_none()
        {
            let mut pending = self.pending_file.lock().unwrap();
            if pending.is_none() {
                *pending = Some(RecordingKind::Session);
            }
        }

        // Start what was asked for once the recorder runs.
        let Some(kind) = *self.pending_file.lock().unwrap() else {
            return;
        };
        // Not while the drive is nearly full; said once each time it runs low.
        if self.disk_low.load(Ordering::Relaxed) {
            *self.pending_file.lock().unwrap() = None;
            if !self.disk_reported.swap(true, Ordering::Relaxed) {
                let free = crate::disk::free_bytes(std::path::Path::new(&settings.storage.folder))
                    .unwrap_or(0);
                (self.sink)(Event::LowDisk {
                    free_bytes: free,
                    limit_bytes: settings.storage.min_free_gb as u64 * (1u64 << 30),
                    stopped: None,
                });
            }
            return;
        }
        // A session needs a game; without one it is an ordinary recording.
        let kind = if kind == RecordingKind::Session && game_pid == 0 {
            RecordingKind::Manual
        } else {
            kind
        };
        // A recording of something chosen for it, other than what the buffer captures.
        let chosen = match kind {
            RecordingKind::Manual => self
                .pending_source
                .lock()
                .unwrap()
                .clone()
                .filter(|c| !same_source(c, &settings.capture)),
            RecordingKind::Session => None,
        };
        // A session that records only the game's window gets its own recorder, so the
        // replay buffer can keep recording the whole screen.
        let own = match (kind, game) {
            _ if chosen.is_some() => match self.source_recorder(settings, chosen.as_ref().unwrap())
            {
                Ok(recorder) => Some(Arc::new(recorder)),
                Err(err) => {
                    log::warn!("cannot record the chosen source: {err:#}");
                    *self.pending_file.lock().unwrap() = None;
                    *self.pending_source.lock().unwrap() = None;
                    *self.last_error.lock().unwrap() = Some(format!("{err:#}"));
                    return;
                }
            },
            (RecordingKind::Session, Some(game))
                if settings.recording.session_source == SessionSource::GameWindow
                    && matches!(settings.capture, CaptureChoice::Monitor { .. }) =>
            {
                match self.game_window_recorder(settings, game) {
                    Ok(recorder) => Some(Arc::new(recorder)),
                    Err(err) => {
                        log::warn!(
                            "cannot record the game's window, recording the capture instead: {err:#}"
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        let shared = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|a| a.recorder.clone());
        let Some(recorder) = own.clone().or(shared) else {
            return;
        };
        let subject = if let Some(choice) = &chosen {
            match choice {
                CaptureChoice::Monitor { .. } => "Desktop".to_string(),
                CaptureChoice::Window { .. } => self.subject(choice),
            }
        } else if own.is_some() {
            game.map(|g| naming::game_name_from_exe(&g.exe))
                .unwrap_or_else(|| self.subject(&settings.capture))
        } else {
            self.subject(&settings.capture)
        };
        let label = match kind {
            RecordingKind::Manual => "recording",
            RecordingKind::Session => "session",
        };
        let path = naming::recording_path(&settings.storage, &subject, local_datetime(), label);
        let mut tags = self.clip_tags(settings, &subject, &recorder);
        tags.push(("recording".into(), kind.as_str().into()));
        // When it began; the file is written again when it stops, so its date is the end.
        tags.push((
            "recorded".into(),
            crate::library::iso8601(std::time::SystemTime::now()),
        ));
        if let Some(choice) = &chosen {
            tags.retain(|(k, _)| k != "source");
            if let Ok((_, label)) = resolve_target(choice) {
                tags.push(("source".into(), label));
            }
        } else if own.is_some() {
            tags.retain(|(k, _)| k != "source");
            tags.push(("source".into(), "The game's window".into()));
        }
        *self.pending_file.lock().unwrap() = None;
        *self.pending_source.lock().unwrap() = None;
        let subject_for_event = subject.clone();
        match recorder.start_file(&path, &tags) {
            Ok(file) => {
                log::info!("recording to {}", path.display());
                *self.file.lock().unwrap() = Some(ActiveFile {
                    file,
                    kind,
                    game: subject,
                    game_pid: if kind == RecordingKind::Session {
                        game_pid
                    } else {
                        0
                    },
                    bookmarks: Vec::new(),
                    source: match (&own, game) {
                        _ if chosen.is_some() => chosen.clone().unwrap(),
                        (Some(_), Some(g)) => CaptureChoice::Window {
                            exe: g.exe.clone(),
                            title: g.title.clone(),
                        },
                        _ => settings.capture.clone(),
                    },
                    own,
                });
                (self.sink)(Event::RecordingStarted {
                    kind,
                    game: subject_for_event,
                });
            }
            Err(err) => {
                log::warn!("cannot start recording: {err:#}");
                (self.sink)(Event::RecordingSaved(RecordingEvent {
                    saved: None,
                    kind,
                    game: subject,
                    error: Some(format!("{err:#}")),
                }));
            }
        }
    }

    fn take_screenshot(&self) -> Result<ScreenshotEvent> {
        let settings = self.settings.lock().unwrap().clone();
        let (target, _) = resolve_target(&settings.capture)?;
        let game = self.subject(&settings.capture);
        let extension = match settings.screenshot.format {
            ScreenshotFormat::Png => "png",
            ScreenshotFormat::Jpg => "jpg",
        };
        let path = naming::media_path(&settings.storage, &game, local_datetime(), extension);
        snapshot::save_screenshot(target, &path, settings.video.capture_cursor)?;
        self.play_sound(settings.screenshot.sound);
        let event = ScreenshotEvent { path, game };
        (self.sink)(Event::ScreenshotSaved(event.clone()));
        Ok(event)
    }
}

/// Everything that needs the recorder to be rebuilt when it changes. The push-to-talk key
/// is left out: changing it takes effect without a restart.
/// The encoder controls from the video settings.
pub fn tuning(video: &settings::VideoPrefs) -> EncoderTuning {
    EncoderTuning {
        tune: video.tune,
        multipass: video.multipass,
        lookahead: video.lookahead,
        spatial_aq: video.spatial_aq,
        temporal_aq: video.temporal_aq,
        aq_strength: video.aq_strength,
    }
}

/// The native resolution of the monitor `target` is, rounded to even sizes for the encoder.
fn screen_size(target: &CaptureTarget) -> Option<(u32, u32)> {
    let CaptureTarget::Monitor(handle) = target else {
        return None;
    };
    list_monitors()
        .into_iter()
        .find(|m| m.handle == *handle)
        .filter(|m| m.native_width > 0 && m.native_height > 0)
        .map(|m| (m.native_width & !1, m.native_height & !1))
}

impl Inner {
    /// Whether the screen being captured is in HDR, looked at every two seconds. Switching
    /// Windows' HDR changes the fingerprint, so the recorder starts again the right way. The
    /// capture is resolved only then too: it lists every screen (or window and program), and
    /// the tick runs ten times a second.
    fn capture_in_hdr(&self, settings: &Settings) -> bool {
        let mut seen = self.hdr_seen.lock().unwrap();
        if let Some((choice, on, at)) = seen.as_ref() {
            if *choice == settings.capture && at.elapsed() < Duration::from_secs(2) {
                return *on;
            }
        }
        let on = resolve_target(&settings.capture).is_ok_and(|(target, _)| {
            let screen = match target {
                CaptureTarget::Monitor(m) => m,
                CaptureTarget::Window(w) => games::monitor_of(w),
            };
            crate::hdr::monitor_hdr(screen).is_some()
        });
        *seen = Some((settings.capture.clone(), on, Instant::now()));
        on
    }

    /// The process of the captured window's program (0 when it is not running), looked up
    /// once a second: it lists every process.
    fn window_program(&self, exe: &str) -> u32 {
        let mut followed = self.followed.lock().unwrap();
        if let Some((known, pid, at)) = followed.as_ref() {
            if known == exe && at.elapsed() < Duration::from_secs(1) {
                return *pid;
            }
        }
        let pid = find_roots(exe).first().copied().unwrap_or(0);
        *followed = Some((exe.to_string(), pid, Instant::now()));
        pid
    }
}

fn fingerprint(settings: &Settings, video: &VideoPrefs, hdr: bool) -> String {
    #[derive(serde::Serialize)]
    struct Parts<'a> {
        capture: &'a CaptureChoice,
        video: &'a settings::VideoPrefs,
        audio: settings::AudioPrefs,
        /// Moving the buffer to or from disk needs a new recorder.
        buffer_dir: Option<std::path::PathBuf>,
        /// So does the screen going in or out of HDR.
        hdr: bool,
        /// And another camera or camera size; switching it on or off does not.
        camera: (Option<&'a str>, u32, u32),
    }
    let mut audio = settings.audio.clone();
    audio.mic.push_to_talk_key.clear();
    serde_json::to_string(&Parts {
        capture: &settings.capture,
        video,
        audio,
        buffer_dir: settings.clip.buffer_dir(),
        hdr,
        camera: (
            settings.camera.device_id.as_deref(),
            settings.camera.width,
            settings.camera.height,
        ),
    })
    .unwrap_or_default()
}

/// Writes a finished recording's file (rewritten as an ordinary MP4 with `finalize`) and its
/// bookmarks, then reports it.
fn complete_file(active: ActiveFile, finalize: bool, reason: Option<String>, sink: &EventSink) {
    // Shown in the library while it is rewritten, which takes minutes for a long session.
    let job = finalize.then(|| {
        crate::processing::begin(
            &active.file.path,
            &active.game,
            active.kind,
            active.file.bytes(),
        )
    });
    let (saved, mut error) = active.file.finish(finalize, &mut |read, size| {
        if let Some(job) = &job {
            job.progress(read, size);
        }
    });
    drop(job);
    if error.is_none() {
        error = reason;
    }
    if let Some(saved) = &saved {
        log::info!(
            "recording saved: {} ({:.0} s)",
            saved.path.display(),
            saved.duration_secs
        );
        if !active.bookmarks.is_empty() {
            recordings::put(
                &saved.path,
                RecordingInfo {
                    bookmarks: active.bookmarks.clone(),
                },
            );
        }
    }
    if let Some(error) = &error {
        log::warn!("recording: {error}");
    }
    sink(Event::RecordingSaved(RecordingEvent {
        saved,
        kind: active.kind,
        game: active.game,
        error,
    }));
    // A session's own recorder stops here.
    drop(active.own);
}

/// Recordings under `folder` still made of fragments. Only files Umbra named as recordings
/// are opened, so a big library is not read file by file.
fn fragmented_recordings(folder: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            let path = item.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
            let named = name.is_some_and(|n| {
                n.ends_with(".mp4") && (n.contains("(recording)") || n.contains("(session)"))
            });
            if named {
                if crate::recorder::is_fragmented(&path) {
                    found.push(path);
                } else if crate::defrag::needs_compact(&path) {
                    // Finished in place by an earlier version: thousands of boxes before the
                    // index made it freeze the app while loading. Rewrapped now, in a moment.
                    match crate::defrag::compact(&path) {
                        Ok(()) => log::info!("rewrapped {}", path.display()),
                        Err(err) => log::warn!("cannot rewrap {}: {err:#}", path.display()),
                    }
                }
            }
        }
    }
    found
}
