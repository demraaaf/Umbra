//! Recording under a game's load: a stand-in game (see `gameload`) runs alone, then while
//! recording with the user's settings. The difference is what recording costs the game, and
//! the recorder's dropped frames are what the game's load costs the recording. From those
//! come suggestions, each a ready change to the video settings or the clip length.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::Serialize;

use crate::gameload::{GameLoad, GameLoadSpec, frame_stats};
use crate::recorder::Recorder;
use crate::settings::{Multipass, VideoPrefs};

/// How long each part is measured.
const WARMUP: Duration = Duration::from_secs(2);
const MEASURE: Duration = Duration::from_secs(8);
/// Recording costs this much of the game's frame rate before it counts as noticeable.
const FPS_COST_LIMIT: f64 = 10.0;
/// The same for the 1 % lows, where stutter shows.
const LOW_COST_LIMIT: f64 = 15.0;
/// Recorded frames that may go missing before it counts.
const DROP_LIMIT: f64 = 0.5;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameFrames {
    pub fps: f64,
    /// Frame rate of the slowest 1 % of frames: where stutter shows.
    pub low_fps: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// What to change and why, in a sentence.
    pub text: String,
    /// The video settings with the change made, if it is one.
    pub video: Option<VideoPrefs>,
    /// A new clip length in seconds, if that is the change.
    pub clip_secs: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadReport {
    pub alone: GameFrames,
    pub recording: GameFrames,
    /// Share of the game's frame rate recording took, in percent.
    pub fps_cost: f64,
    pub low_cost: f64,
    pub frames_recorded: u64,
    pub frames_dropped: u64,
    /// Dropped recorded frames, in percent.
    pub drop_percent: f64,
    /// Memory the stand-in game held and the replay buffer can need, in bytes.
    pub game_memory: u64,
    pub buffer_memory: u64,
    pub total_memory: u64,
    pub spec: LoadSpecView,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSpecView {
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub cpu_threads: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadProgress {
    pub label: String,
    /// 0 to 1.
    pub fraction: f64,
}

/// The stand-in game for this PC: the screen's size, a fast shooter's frame rate, half the
/// processor and a quarter of the memory (at most 8 GB).
pub fn default_spec(screen: (u32, u32), total_memory: u64) -> GameLoadSpec {
    let cores = std::thread::available_parallelism().map_or(8, |n| n.get());
    GameLoadSpec {
        width: screen.0.max(640),
        height: screen.1.max(360),
        target_fps: 200,
        cpu_threads: (cores / 2).max(1),
        memory_bytes: (total_memory / 4).min(8 << 30),
    }
}

/// Runs the test. `start_recorder` starts recording with the settings under test.
pub fn run(
    spec: GameLoadSpec,
    video: &VideoPrefs,
    output_height: u32,
    clip_secs: u32,
    buffer_memory: u64,
    total_memory: u64,
    start_recorder: &mut dyn FnMut() -> Result<Recorder>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(LoadProgress),
) -> Result<LoadReport> {
    let total_secs = (WARMUP * 2 + MEASURE * 2).as_secs_f64() + 1.0;
    let started = Instant::now();
    let say = |label: &str, progress: &mut dyn FnMut(LoadProgress)| {
        progress(LoadProgress {
            label: label.into(),
            fraction: (started.elapsed().as_secs_f64() / total_secs).min(0.99),
        })
    };
    let wait = |d: Duration, label: &str, progress: &mut dyn FnMut(LoadProgress)| -> Result<()> {
        let until = Instant::now() + d;
        while Instant::now() < until {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            std::thread::sleep(Duration::from_millis(250));
            progress(LoadProgress {
                label: label.into(),
                fraction: (started.elapsed().as_secs_f64() / total_secs).min(0.99),
            });
        }
        Ok(())
    };

    say("Starting the simulated game", progress);
    let game = GameLoad::start(spec)?;
    wait(WARMUP, "Warming up the simulated game", progress)?;
    game.take_frames();
    wait(MEASURE, "Measuring the game on its own", progress)?;
    let alone = frame_stats(&game.take_frames());

    say("Starting to record with your settings", progress);
    let recorder = start_recorder()?;
    wait(WARMUP, "Recording starts", progress)?;
    game.take_frames();
    let before = recorder.stats();
    wait(MEASURE, "Measuring the game while recording", progress)?;
    let with = frame_stats(&game.take_frames());
    let after = recorder.stats();
    if let Some(err) = recorder.error() {
        bail!("recording stopped during the test: {err}");
    }
    drop(recorder);
    if let Some(err) = game.error() {
        bail!("the simulated game stopped: {err}");
    }
    drop(game);

    let recorded = after.frames_encoded.saturating_sub(before.frames_encoded);
    let dropped = after.dropped_frames.saturating_sub(before.dropped_frames);
    let percent = |part: f64, whole: f64| {
        if whole > 0.0 {
            part / whole * 100.0
        } else {
            0.0
        }
    };
    let mut report = LoadReport {
        alone: GameFrames {
            fps: alone.0,
            low_fps: alone.1,
        },
        recording: GameFrames {
            fps: with.0,
            low_fps: with.1,
        },
        fps_cost: percent(alone.0 - with.0, alone.0).max(0.0),
        low_cost: percent(alone.1 - with.1, alone.1).max(0.0),
        frames_recorded: recorded,
        frames_dropped: dropped,
        drop_percent: percent(dropped as f64, (recorded + dropped) as f64),
        game_memory: spec.memory_bytes,
        buffer_memory,
        total_memory,
        spec: LoadSpecView {
            width: spec.width,
            height: spec.height,
            target_fps: spec.target_fps,
            cpu_threads: spec.cpu_threads,
        },
        suggestions: Vec::new(),
    };
    report.suggestions = suggest(&report, video, output_height, clip_secs);
    progress(LoadProgress {
        label: "Done".into(),
        fraction: 1.0,
    });
    Ok(report)
}

/// The next lower common frame rate.
fn lower_fps(fps: u32) -> Option<u32> {
    [240, 165, 144, 120, 60]
        .into_iter()
        .find(|&f| f < fps)
        .filter(|&f| f >= 60)
}

/// What to change, most effective first. Each suggestion changes one thing from the
/// current settings.
pub fn suggest(
    report: &LoadReport,
    video: &VideoPrefs,
    output_height: u32,
    clip_secs: u32,
) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    let change = |edit: &dyn Fn(&mut VideoPrefs)| {
        let mut next = video.clone();
        edit(&mut next);
        Some(next)
    };
    let drops = report.drop_percent > DROP_LIMIT;
    let costly = report.fps_cost > FPS_COST_LIMIT || report.low_cost > LOW_COST_LIMIT;

    if drops {
        if video.multipass != Multipass::Off {
            out.push(Suggestion {
                text: "Turn multipass off. It roughly halves how fast the encoder works, and the recording dropped frames.".into(),
                video: change(&|v| v.multipass = Multipass::Off),
                clip_secs: None,
            });
        }
        if video.preset > 3 {
            let preset = (video.preset - 2).max(1);
            out.push(Suggestion {
                text: format!(
                    "Use encoder preset {preset} instead of {}. Faster presets keep up under load at a small cost in quality.",
                    video.preset
                ),
                video: change(&|v| v.preset = preset),
                clip_secs: None,
            });
        }
    }
    if drops || costly {
        if let Some(fps) = lower_fps(video.fps) {
            out.push(Suggestion {
                text: format!(
                    "Record {fps} fps instead of {}. Every recorded frame is a copy of the screen the game's frames wait behind.",
                    video.fps
                ),
                video: change(&|v| v.fps = fps),
                clip_secs: None,
            });
        }
        if output_height > 1080 {
            out.push(Suggestion {
                text: format!(
                    "Record at 1080p instead of {output_height}p. Smaller frames are quicker to copy and encode."
                ),
                video: change(&|v| {
                    let width = ((output_height as u64 * 16 / 9) * 1080 / output_height as u64) as u32;
                    v.width = Some(width / 2 * 2);
                    v.height = Some(1080);
                }),
                clip_secs: None,
            });
        }
    }
    if costly && video.lookahead > 0 {
        out.push(Suggestion {
            text: "Turn look-ahead off. It keeps extra frames on the graphics card.".into(),
            video: change(&|v| v.lookahead = 0),
            clip_secs: None,
        });
    }
    // Memory: the game, the replay buffer and Windows have to fit.
    let needed = report.game_memory + report.buffer_memory;
    if report.total_memory > 0 && needed as f64 > report.total_memory as f64 * 0.8 && clip_secs > 15
    {
        let secs = (clip_secs / 2).max(15);
        out.push(Suggestion {
            text: format!(
                "Keep {secs} seconds instead of {clip_secs}. A game this size and the replay buffer together leave little memory free, which causes stutter."
            ),
            video: None,
            clip_secs: Some(secs),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(fps_cost: f64, low_cost: f64, drop_percent: f64) -> LoadReport {
        LoadReport {
            alone: GameFrames {
                fps: 200.0,
                low_fps: 170.0,
            },
            recording: GameFrames {
                fps: 200.0 * (1.0 - fps_cost / 100.0),
                low_fps: 170.0 * (1.0 - low_cost / 100.0),
            },
            fps_cost,
            low_cost,
            frames_recorded: 1000,
            frames_dropped: (drop_percent * 10.0) as u64,
            drop_percent,
            game_memory: 4 << 30,
            buffer_memory: 1 << 30,
            total_memory: 32 << 30,
            spec: LoadSpecView {
                width: 2560,
                height: 1440,
                target_fps: 200,
                cpu_threads: 8,
            },
            suggestions: Vec::new(),
        }
    }

    #[test]
    fn nothing_to_change_when_recording_is_light() {
        let video = VideoPrefs::default();
        assert!(suggest(&report(4.0, 6.0, 0.0), &video, 1440, 30).is_empty());
    }

    #[test]
    fn dropped_frames_point_at_the_encoder_first() {
        let video = VideoPrefs {
            preset: 7,
            fps: 144,
            multipass: Multipass::Full,
            ..VideoPrefs::default()
        };
        let list = suggest(&report(3.0, 4.0, 5.0), &video, 1440, 30);
        let first = list[0].video.as_ref().unwrap();
        assert_eq!(first.multipass, Multipass::Off);
        assert_eq!(list[1].video.as_ref().unwrap().preset, 5);
        assert_eq!(list[2].video.as_ref().unwrap().fps, 120);
        let smaller = list[3].video.as_ref().unwrap();
        assert_eq!((smaller.width, smaller.height), (Some(1920), Some(1080)));
    }

    #[test]
    fn a_costly_recording_records_less() {
        let video = VideoPrefs {
            fps: 240,
            lookahead: 16,
            ..VideoPrefs::default()
        };
        let list = suggest(&report(18.0, 25.0, 0.0), &video, 1080, 30);
        assert_eq!(list[0].video.as_ref().unwrap().fps, 165);
        assert_eq!(list.last().unwrap().video.as_ref().unwrap().lookahead, 0);
        assert!(list.iter().all(|s| s.clip_secs.is_none()));
    }

    #[test]
    fn little_memory_shortens_the_buffer() {
        let mut r = report(2.0, 2.0, 0.0);
        r.total_memory = 8 << 30;
        r.game_memory = 6 << 30;
        let list = suggest(&r, &VideoPrefs::default(), 1080, 60);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].clip_secs, Some(30));
    }
}
