//! The data the UI sees. Field names match `app/ui/src/lib/api/types.ts`.

use serde::Serialize;
use umbra_core::controller::{State, Status};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub state: &'static str,
    pub error: Option<String>,
    pub target: Option<String>,
    pub game: Option<String>,
    pub game_reason: Option<String>,
    pub encoder: Option<String>,
    pub hdr: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub buffer_secs: f64,
    pub buffer_bytes: u64,
    pub buffer_on_disk: bool,
    pub mic_live: bool,
    pub push_to_talk: bool,
    pub dropped_frames: u64,
    pub usage: UsageDto,
    pub file: Option<FileStatusDto>,
    pub session_stopped: bool,
    pub audio_problems: Vec<AudioProblemDto>,
    /// The camera is switched on.
    pub camera_on: bool,
    /// The camera is being recorded right now.
    pub camera_live: bool,
    pub camera_problem: Option<String>,
    /// What the keyboard and mouse capture is doing, and the rule for the program in front.
    pub input: umbra_core::input::InputStatus,
}

/// An audio track recorded as silence because its source cannot be opened.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioProblemDto {
    pub track: String,
    pub problem: String,
}

/// A recording to a file in progress.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileStatusDto {
    pub kind: umbra_core::recordings::RecordingKind,
    pub secs: f64,
    pub bytes: u64,
    pub game: String,
    pub path: String,
    pub source: umbra_core::settings::CaptureChoice,
    pub paused: bool,
}

#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageDto {
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub gpu_percent: Option<f64>,
    pub encoder_percent: Option<f64>,
}

impl From<Status> for StatusDto {
    fn from(s: Status) -> Self {
        Self {
            state: match s.state {
                State::Idle => "idle",
                State::Recording => "recording",
                State::Error => "error",
            },
            error: s.error,
            target: s.target,
            game: s.game,
            game_reason: s.game_reason,
            encoder: s.encoder,
            hdr: s.hdr,
            width: s.width,
            height: s.height,
            fps: s.fps,
            buffer_secs: s.buffer_secs,
            buffer_bytes: s.buffer_bytes,
            buffer_on_disk: s.buffer_on_disk,
            mic_live: s.mic_live,
            push_to_talk: s.push_to_talk,
            dropped_frames: s.dropped_frames,
            usage: UsageDto {
                cpu_percent: (s.usage.cpu_percent * 10.0).round() / 10.0,
                memory_bytes: s.usage.memory_bytes,
                gpu_percent: s.usage.gpu_percent.map(|v| (v * 10.0).round() / 10.0),
                encoder_percent: s.usage.encoder_percent.map(|v| (v * 10.0).round() / 10.0),
            },
            session_stopped: s.session_stopped,
            camera_on: s.camera_on,
            camera_live: s.camera_live,
            camera_problem: s.camera_problem,
            input: s.input,
            audio_problems: s
                .audio_problems
                .into_iter()
                .map(|(track, problem)| AudioProblemDto { track, problem })
                .collect(),
            file: s.file.map(|f| FileStatusDto {
                kind: f.kind,
                secs: f.secs,
                bytes: f.bytes,
                game: f.game,
                path: f.path.to_string_lossy().into_owned(),
                source: f.source,
                paused: f.paused,
            }),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDto {
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub primary: bool,
    /// The monitor's own resolution, which differs while the screen runs a stretched mode.
    pub native_width: u32,
    pub native_height: u32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunningGameDto {
    pub exe: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WindowDto {
    pub exe: String,
    pub title: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioAppDto {
    pub exe: String,
    pub name: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneDto {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfoDto {
    pub app_version: String,
    pub gpu: String,
    pub codecs: Vec<&'static str>,
    pub settings_folder: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSourceDto {
    /// Same shape as the `capture` setting.
    pub target: umbra_core::settings::CaptureChoice,
    pub label: String,
    pub detail: String,
    pub is_game: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UsageEstimateDto {
    pub buffer_bytes: u64,
    pub clip_bytes: u64,
    pub encoder_load: f64,
    pub cpu_percent: f64,
    pub level: &'static str,
    pub notes: Vec<String>,
    pub breakdown: Vec<umbra_core::estimate::Cost>,
}
