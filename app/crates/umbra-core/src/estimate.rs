//! Works out what a set of settings will cost before recording with them.
//!
//! The encoder figures come from the milestone 1 measurements on an RTX 5070 Ti
//! (docs/spike-results.md). Other GPUs will differ, which the notes say.

use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use serde::Serialize;

use crate::settings::{AudioMode, CodecChoice, Multipass, RateMode, RecordingMode, Settings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Light,
    Moderate,
    Heavy,
    TooHeavy,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Light => "light",
            Level::Moderate => "moderate",
            Level::Heavy => "heavy",
            Level::TooHeavy => "too-heavy",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Estimate {
    pub buffer_bytes: u64,
    pub clip_bytes: u64,
    /// Share of the encoder's capacity, where 1 is the most it can do.
    pub encoder_load: f64,
    /// Share of the whole CPU, 0 to 100.
    pub cpu_percent: f64,
    pub level: Level,
    pub notes: Vec<String>,
    /// What each costly setting costs, most costly first.
    pub breakdown: Vec<Cost>,
}

/// One setting's cost, explained: what it uses, why, what to change and what that gives up.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    /// `encoder`, `gpu`, `cpu`, `memory` or `disk`.
    pub resource: &'static str,
    /// `high`, `medium` or `low`.
    pub impact: &'static str,
    pub title: String,
    pub why: String,
    pub change: String,
    pub catch: String,
}

/// Pixels per second the encoder handles at presets 1 to 7, for H.264 on the reference GPU.
/// Measured: preset 4 tops out near 336 fps at 1440p, preset 1 kept up at 360 fps, and
/// preset 7 managed about 100 fps with HEVC.
const H264_CAPACITY: [f64; 7] = [1.60e9, 1.50e9, 1.40e9, 1.24e9, 0.95e9, 0.60e9, 0.41e9];

fn capacity(codec: CodecChoice, preset: u8) -> f64 {
    let base = H264_CAPACITY[(preset.clamp(1, 7) - 1) as usize];
    match codec {
        CodecChoice::H264 => base,
        // HEVC reached about 283 fps where H.264 reached 336 at the same preset.
        CodecChoice::Hevc => base * 0.84,
        CodecChoice::Av1 => base * 0.85,
    }
}

pub fn total_ram() -> Option<u64> {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullTotalPhys)
}

fn gib(bytes: u64) -> String {
    let value = bytes as f64 / (1u64 << 30) as f64;
    if value >= 10.0 {
        format!("{value:.0} GB")
    } else {
        format!("{value:.1} GB")
    }
}

/// `source` is the size of what is captured, used when the settings keep the source size.
/// `gpu` is the adapter name, to say when the encoder figures come from another GPU.
pub fn estimate(settings: &Settings, source: (u32, u32), gpu: Option<&str>) -> Estimate {
    let video = &settings.video;
    let (width, height) = match (video.width, video.height) {
        (Some(w), Some(h)) => (w, h),
        _ => source,
    };
    let pixels_per_frame = width as f64 * height as f64;
    let pixel_rate = pixels_per_frame * video.fps as f64;

    let video_bps = match video.rate_mode {
        RateMode::Cbr => video.bitrate_kbps as f64 * 1000.0,
        RateMode::Vbr => video.max_bitrate_kbps as f64 * 1000.0,
        // Bits per pixel for busy gameplay at a given quantiser: about 0.5 near-lossless
        // (QP 10), falling by roughly a quarter every two steps.
        RateMode::Cqp => pixel_rate * 0.5 * 0.87f64.powi(video.qp as i32 - 10),
        // Constant quality behaves like a quantiser a few steps lower, capped at the peak.
        RateMode::Cq => (pixel_rate * 0.5 * 0.87f64.powi(video.cq as i32 - 13))
            .min(video.max_bitrate_kbps as f64 * 1000.0),
    };
    let tracks = audio_track_count(settings);
    let audio_bps = tracks as f64 * settings.audio.bitrate_kbps as f64 * 1000.0;
    // The camera's stream, at the rate `camera::CameraWorker` gives it (counted at 30 fps).
    let camera = &settings.camera;
    let camera_bps = if camera.device_id.is_some() && camera.enabled {
        camera.width as f64 * camera.height as f64 * 30f64.min(video.fps as f64) / 10.0
    } else {
        0.0
    };
    let stream_bps = video_bps + audio_bps + camera_bps;
    let seconds = settings.clip.buffer_secs() as f64;
    // A clip starts on a keyframe, so it can be up to a second longer than the buffer length.
    let buffer_bytes = (stream_bps / 8.0 * (seconds + 1.0)) as u64;
    let clip_bytes = (stream_bps / 8.0 * seconds) as u64;

    // A first pass costs the encoder time: about half at full resolution.
    let multipass = match video.multipass {
        Multipass::Off => 1.0,
        Multipass::Quarter => 0.75,
        Multipass::Full => 0.5,
    };
    let encoder_load = pixel_rate / (capacity(video.codec, video.preset) * multipass);
    let scaled = video.width.is_some();
    let cpu_percent = (0.2
        + 0.5 * video.fps as f64 / 240.0
        + 0.1 * tracks as f64
        + if scaled { 0.1 } else { 0.0 })
    .min(100.0);

    let mut notes = Vec::new();
    let mut level = Level::Light;
    let mut raise = |to: Level| {
        if (to as u8) > (level as u8) {
            level = to;
        }
    };

    if encoder_load > 1.0 {
        raise(Level::TooHeavy);
        notes.push(format!(
            "The encoder can't keep up with {width}×{height} at {} fps on preset {}; frames will be dropped. A faster preset, a lower frame rate or a smaller size fixes it.",
            video.fps, video.preset
        ));
    } else if encoder_load > 0.7 {
        raise(Level::Heavy);
        notes.push("The encoder is close to its limit. A game that also works the GPU hard may cause dropped frames.".into());
    } else if encoder_load > 0.35 {
        raise(Level::Moderate);
    }

    if video.fps >= 240 {
        raise(Level::Heavy);
        notes.push(
            "Needs a high-end PC. This can cause dropped frames in recordings and in-game.".into(),
        );
    }

    if let Some(ram) = total_ram() {
        let share = buffer_bytes as f64 / ram as f64;
        if share > 0.5 {
            raise(Level::TooHeavy);
        } else if share > 0.25 {
            raise(Level::Heavy);
        } else if share > 0.1 {
            raise(Level::Moderate);
        }
        if share > 0.1 {
            notes.push(format!(
                "The replay buffer needs up to {} of RAM, {:.0}% of this PC's {}.",
                gib(buffer_bytes),
                share * 100.0,
                gib(ram)
            ));
        }
    }

    if matches!(video.rate_mode, RateMode::Cqp | RateMode::Cq) {
        notes.push("Constant quality: the size depends on what is on screen. These figures assume busy gameplay.".into());
    }
    if !gpu.is_some_and(|name| name.contains("5070 Ti")) {
        notes.push(
            "Encoder figures come from measurements on an RTX 5070 Ti; other GPUs differ.".into(),
        );
    }

    let breakdown = breakdown(
        settings,
        (width, height),
        encoder_load,
        video_bps,
        buffer_bytes,
        tracks,
    );
    Estimate {
        buffer_bytes,
        clip_bytes,
        encoder_load,
        cpu_percent,
        level,
        notes,
        breakdown,
    }
}

fn breakdown(
    settings: &Settings,
    (width, height): (u32, u32),
    encoder_load: f64,
    video_bps: f64,
    buffer_bytes: u64,
    tracks: usize,
) -> Vec<Cost> {
    let video = &settings.video;
    let always = settings.recording.mode == RecordingMode::Always;
    let mut costs = Vec::new();
    let mut add = |resource, impact, title: String, why: String, change: &str, catch: &str| {
        costs.push(Cost {
            resource,
            impact,
            title,
            why,
            change: change.into(),
            catch: catch.into(),
        })
    };
    let percent = |share: f64| format!("{:.0}%", share * 100.0);

    let pixel_rate = width as f64 * height as f64 * video.fps as f64;
    add(
        "encoder",
        if encoder_load > 0.5 {
            "high"
        } else if encoder_load > 0.2 {
            "medium"
        } else {
            "low"
        },
        format!("{width}×{height} at {} fps", video.fps),
        format!(
            "The encoder compresses every pixel of every frame: {:.0} million a second, about {} of what it can do with these settings. At 60 fps it would be about {}.",
            pixel_rate / 1e6,
            percent(encoder_load),
            percent(encoder_load * 60.0 / video.fps.max(1) as f64),
        ),
        "Lower Video › Frame rate, or record a smaller Video › Resolution.",
        "Fewer frames look less smooth on a high-refresh screen and in slow motion. A smaller size is less sharp, and scaling it takes a little of the graphics card's 3D time, which games use.",
    );

    if !video.skip_still {
        add(
            "encoder",
            if always { "high" } else { "medium" },
            "Still frames are encoded again".into(),
            format!(
                "To keep a steady {} fps, the last picture is encoded again whenever the screen has not changed. A still desktop costs the encoder as much as a busy game.",
                video.fps
            ),
            "Turn on Video › Skip still frames (with advanced settings on).",
            "Clips look the same, but their frame rate varies. Players and Umbra handle that; Premiere and DaVinci Resolve may warn or let the audio drift. Leave it off in the presets of games you edit.",
        );
    }

    if always {
        add(
            "encoder",
            "medium",
            "Records whenever Umbra is open".into(),
            "With Recording › Replay buffer set to Always, the screen is captured and encoded from the moment Umbra starts, on the desktop too, so the encoder and the buffer's memory are in use even when no game runs.".into(),
            "Set Recording › Replay buffer to Only while a game is running.",
            "No clips of the desktop or of programs Umbra does not know as games, until you add them to the game list.",
        );
    }

    if video.multipass != Multipass::Off {
        let (how, share) = match video.multipass {
            Multipass::Full => ("at full resolution", "half"),
            _ => ("at quarter resolution", "a quarter"),
        };
        add(
            "encoder",
            if video.multipass == Multipass::Full {
                "high"
            } else {
                "medium"
            },
            format!("Multipass {how}"),
            format!(
                "The encoder looks at every frame once more {how} before encoding it, which costs about {share} of its speed."
            ),
            "Set Video › Multipass to Off (with advanced settings on).",
            "At high bitrates the difference is hard to see. At low bitrates, fast motion may show a little more blocking.",
        );
    }

    if video.preset >= 5 {
        let slower = 1.0 - capacity(video.codec, video.preset) / capacity(video.codec, 4);
        add(
            "encoder",
            if video.preset >= 6 { "high" } else { "medium" },
            format!("Encoder preset {}", video.preset),
            format!(
                "Slower presets spend more time on each frame to fit more quality into the bitrate. Preset {} gets through about {} fewer pixels a second than preset 4.",
                video.preset,
                percent(slower)
            ),
            "Choose a lower Video › Encoder preset (with advanced settings on).",
            "At the same bitrate the picture is a little less sharp; a higher bitrate makes up for it, at the cost of memory and file size.",
        );
    }

    if video.lookahead > 0 {
        add(
            "encoder",
            "low",
            format!("Look-ahead of {} frames", video.lookahead),
            "The encoder holds upcoming frames to plan the bitrate around scene changes, which takes some encoder time and graphics memory.".into(),
            "Set Video › Look-ahead to 0 (with advanced settings on).",
            "Sudden changes, such as flashes and cuts, may look blockier for a moment.",
        );
    }

    // Bits per pixel that look clean for gameplay; more mostly makes larger files.
    let enough = match video.codec {
        CodecChoice::H264 => 0.1,
        CodecChoice::Hevc | CodecChoice::Av1 => 0.06,
    };
    // The average the encoder aims for; `video_bps` is the peak the buffer has room for.
    let target_bps = match video.rate_mode {
        RateMode::Cbr | RateMode::Vbr => video.bitrate_kbps as f64 * 1000.0,
        RateMode::Cq | RateMode::Cqp => video_bps,
    };
    let generous = target_bps > pixel_rate * enough * 1.5;
    let seconds = settings.clip.buffer_secs();
    let share = total_ram().map_or(0.0, |ram| buffer_bytes as f64 / ram as f64);
    let (resource, title, holds) = if settings.clip.buffer_on_disk {
        (
            "disk",
            format!("Replay buffer: {} on disk", gib(buffer_bytes)),
            format!(
                "The last {seconds} s are kept on the drive, written at {:.0} MB a second.",
                video_bps / 8e6
            ),
        )
    } else {
        (
            "memory",
            format!("Replay buffer: {} of RAM", gib(buffer_bytes)),
            format!("The last {seconds} s are kept in memory."),
        )
    };
    let advice = if generous {
        format!(
            " {:.0} Mbit/s is more than {} needs here; about {:.0} Mbit/s looks the same in most games.",
            target_bps / 1e6,
            match video.codec {
                CodecChoice::H264 => "H.264",
                CodecChoice::Hevc => "HEVC",
                CodecChoice::Av1 => "AV1",
            },
            pixel_rate * enough / 1e6
        )
    } else {
        String::new()
    };
    add(
        resource,
        if share > 0.25 {
            "high"
        } else if share > 0.1 || generous {
            "medium"
        } else {
            "low"
        },
        title,
        format!(
            "{holds} Its size is the bitrate times the clip length; the bitrate barely changes the encoder's load.{advice}"
        ),
        "Lower the video bitrate, or shorten Clips › Length.",
        "Too little bitrate makes fast motion, such as grass, smoke or particles, blocky. A shorter length keeps less of what happened before you pressed the hotkey.",
    );

    if let (Some(w), Some(h)) = (video.width, video.height) {
        add(
            "gpu",
            "low",
            format!("Scaling to {w}×{h}"),
            "Each frame is resized on the graphics card's 3D engine before encoding. Games use the same engine, but resizing is a small job.".into(),
            "Set Video › Resolution to the screen's own size.",
            "A larger video means more encoder work and larger files.",
        );
    }

    if settings.camera.enabled && settings.camera.device_id.is_some() {
        add(
            "cpu",
            "low",
            "Webcam".into(),
            "The camera's picture is decoded on the processor and encoded as a second video stream.".into(),
            "Turn the camera off when you do not need it.",
            "Clips have no camera to place.",
        );
    }

    if tracks >= 5 {
        add(
            "cpu",
            "low",
            format!("{tracks} audio tracks"),
            "Each track is captured and compressed on its own; that is light work, but it adds up and every track is in the buffer.".into(),
            "Set Audio › Tracks to One system track, or remove apps you never need on their own.",
            "With one track you can no longer turn Discord or music up, down or off in the editor.",
        );
    }

    let rank = |impact: &str| match impact {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    costs.sort_by_key(|c| rank(c.impact));
    costs
}

fn audio_track_count(settings: &Settings) -> usize {
    let audio = &settings.audio;
    let mixed = match audio.mode {
        AudioMode::Split => audio.game as usize + audio.apps.len() + audio.others as usize,
        AudioMode::System => 1,
    };
    // The microphone has its own track and is not in the combined one, which is added when
    // it combines more than one track.
    mixed + audio.mic.enabled as usize + (mixed > 1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_light_on_the_reference_gpu() {
        let e = estimate(
            &Settings::default(),
            (2560, 1440),
            Some("NVIDIA GeForce RTX 5070 Ti"),
        );
        assert!(e.encoder_load < 0.35, "{}", e.encoder_load);
        // 50 Mbit/s plus audio for 31 seconds is a little under 200 MB.
        assert!(
            (150_000_000..260_000_000).contains(&e.buffer_bytes),
            "{}",
            e.buffer_bytes
        );
        assert!(e.notes.iter().all(|n| !n.contains("5070 Ti")));
    }

    #[test]
    fn slow_preset_at_360_fps_cannot_keep_up() {
        let mut s = Settings::default();
        s.video.fps = 360;
        s.video.preset = 7;
        let e = estimate(&s, (2560, 1440), Some("RTX 5070 Ti"));
        assert!(e.encoder_load > 1.0);
        assert_eq!(e.level, Level::TooHeavy);
        assert!(e.notes.iter().any(|n| n.contains("high-end PC")));
    }

    #[test]
    fn matches_the_measured_limit_at_preset_4() {
        // Measured: H.264 at preset 4 managed about 336 fps at 1440p.
        let mut s = Settings::default();
        s.video.codec = CodecChoice::H264;
        s.video.fps = 336;
        let e = estimate(&s, (2560, 1440), None);
        assert!((0.9..1.1).contains(&e.encoder_load), "{}", e.encoder_load);
    }

    #[test]
    fn breakdown_explains_heavy_settings_first() {
        let mut s = Settings::default();
        s.video.fps = 144;
        s.video.multipass = Multipass::Quarter;
        s.recording.mode = RecordingMode::Always;
        let e = estimate(&s, (2560, 1440), None);
        let titles: Vec<_> = e.breakdown.iter().map(|c| c.title.as_str()).collect();
        assert!(
            titles.contains(&"Still frames are encoded again"),
            "{titles:?}"
        );
        assert!(
            titles.contains(&"Multipass at quarter resolution"),
            "{titles:?}"
        );
        assert_eq!(e.breakdown[0].impact, "high");

        s.video.skip_still = true;
        s.video.multipass = Multipass::Off;
        let e = estimate(&s, (2560, 1440), None);
        assert!(
            e.breakdown
                .iter()
                .all(|c| !c.title.starts_with("Still frames"))
        );
        assert!(
            e.breakdown
                .iter()
                .all(|c| !c.title.starts_with("Multipass"))
        );
    }

    #[test]
    fn other_gpus_get_a_caveat() {
        let e = estimate(&Settings::default(), (1920, 1080), Some("AMD Radeon"));
        assert!(e.notes.iter().any(|n| n.contains("other GPUs differ")));
    }
}
