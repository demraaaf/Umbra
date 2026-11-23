//! Finds the best video settings this PC can record without dropping frames.
//!
//! The user says what they play, how much game performance recording may cost, and what is
//! fixed (recording size, frame rate). The benchmark then encodes moving test frames on the
//! graphics card's video encoder at the candidate settings, as fast as it can, and measures
//! how many frames per second each one manages. A candidate passes when the encoder is fast
//! enough to leave the asked-for headroom: a recording that uses a third of the encoder
//! leaves room for a game that also works the GPU hard; one that uses three quarters does not.
//! The slowest (best-looking) preset that passes wins, and the bitrate follows from the size,
//! the frame rate, the codec and how busy the games are.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use crate::d3d::Gpu;
use crate::encode::{EncoderTuning, RateControl, VideoCodec, VideoEncoder, VideoSettings};
use crate::settings::{CodecChoice, Multipass, RateMode, Scaling, Tune, VideoPrefs};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameKind {
    /// Fast shooters and other competitive games: lots of motion, smoothness matters.
    Shooter,
    /// Story and open-world games: detail matters more than frame rate.
    Story,
    Mixed,
}

/// How much game performance recording may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cost {
    /// The game's frame rate comes first.
    Least,
    /// A few frames for clearly better clips.
    Some,
    /// The best clips; the PC has frames to spare.
    Most,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answers {
    pub game: GameKind,
    pub cost: Cost,
    /// A recording size the user fixed, or none to record at the screen's own size.
    pub size: Option<(u32, u32)>,
    /// A frame rate the user fixed, or none to let the benchmark choose.
    pub fps: Option<u32>,
    pub scaling: Scaling,
    /// Clips mostly go to Discord or the web, so they must play everywhere.
    pub share: bool,
}

/// The monitor that is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

/// One measured candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trial {
    pub codec: CodecChoice,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub preset: u8,
    /// Frames per second the encoder managed flat out, or none if it would not start.
    pub measured_fps: Option<f64>,
    /// Share of the encoder recording at `fps` would use, 0 to 1 and beyond.
    pub load: Option<f64>,
    pub passed: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The current video settings with the recommendation applied.
    pub video: VideoPrefs,
    pub trials: Vec<Trial>,
    /// What was decided and why, one sentence each.
    pub notes: Vec<String>,
    /// True if nothing passed and the result is the fastest setting there is.
    pub compromise: bool,
}

/// Sent while the benchmark runs: what is being tested, then each result.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub label: String,
    pub trial: Option<Trial>,
}

/// Presets tried, best first. The first that passes is used, so a fast PC needs one test.
const PRESETS: [u8; 6] = [7, 6, 5, 4, 3, 1];
/// How long each candidate is measured, after warming up.
const MEASURE: Duration = Duration::from_millis(900);
const WARMUP_FRAMES: usize = 12;
/// Test frames cycled through; each is the same scene moved a little, like a camera pan.
const FRAMES: usize = 8;

/// The frame rate to record at.
pub fn target_fps(answers: &Answers, screen: &Screen) -> u32 {
    if let Some(fps) = answers.fps {
        return fps.clamp(1, 360);
    }
    let fps = match (answers.game, answers.cost) {
        (GameKind::Shooter, Cost::Least) => 60,
        (GameKind::Shooter, Cost::Some) => 120,
        (GameKind::Shooter, Cost::Most) => 144,
        (GameKind::Mixed, Cost::Most) => 120,
        _ => 60,
    };
    // More frames than the screen shows add nothing.
    if screen.refresh_hz >= 30 {
        fps.min(screen.refresh_hz)
    } else {
        fps
    }
}

/// How many times faster than the recording the encoder must be.
pub fn headroom(cost: Cost) -> f64 {
    match cost {
        Cost::Least => 3.0,
        Cost::Some => 2.0,
        Cost::Most => 1.4,
    }
}

/// Encoder controls for each answer. Better clips get adaptive quantisation, and the most
/// demanding answer a quarter-resolution first pass as well.
pub fn tuning(cost: Cost) -> EncoderTuning {
    EncoderTuning {
        tune: Tune::Hq,
        multipass: if cost == Cost::Most {
            Multipass::Quarter
        } else {
            Multipass::Off
        },
        lookahead: 0,
        spatial_aq: cost != Cost::Least,
        temporal_aq: false,
        aq_strength: 8,
    }
}

/// Codecs to try, preferred first.
pub fn codec_order(answers: &Answers, supported: &[CodecChoice]) -> Vec<CodecChoice> {
    let order: &[CodecChoice] = if answers.share {
        // H.264 plays in every browser and in Discord's embeds.
        &[CodecChoice::H264, CodecChoice::Hevc]
    } else if answers.cost == Cost::Most {
        &[CodecChoice::Av1, CodecChoice::Hevc, CodecChoice::H264]
    } else {
        &[CodecChoice::Hevc, CodecChoice::H264, CodecChoice::Av1]
    };
    order
        .iter()
        .copied()
        .filter(|c| supported.contains(c))
        .collect()
}

/// Recording sizes to try, largest first: the fixed size, or the screen's and then smaller
/// ones of the same shape in case the screen's is too much for the encoder.
pub fn sizes(answers: &Answers, screen: &Screen) -> Vec<(u32, u32)> {
    if let Some((w, h)) = answers.size {
        return vec![(w & !1, h & !1)];
    }
    let mut list = vec![(screen.width & !1, screen.height & !1)];
    for height in [1440u32, 1080, 720] {
        if height < screen.height {
            let width = (screen.width as u64 * height as u64 / screen.height.max(1) as u64) as u32;
            list.push((width & !1, height));
        }
    }
    list
}

/// Video bitrate in kbit/s for clips that look good at this size and frame rate.
///
/// Anchored on 1080p at 60 fps in HEVC: 25 Mbit/s for fast shooters, a little less for
/// calmer games. It grows a little slower than the pixel count (bigger pictures compress
/// better) and much slower than the frame rate (frames close in time are alike).
pub fn bitrate_kbps(answers: &Answers, codec: CodecChoice, size: (u32, u32), fps: u32) -> u32 {
    let base = match answers.game {
        GameKind::Shooter => 25_000.0,
        GameKind::Mixed => 22_500.0,
        GameKind::Story => 20_000.0,
    };
    let quality = match answers.cost {
        Cost::Least => 0.8,
        Cost::Some => 1.0,
        Cost::Most => 1.4,
    };
    let codec = match codec {
        CodecChoice::H264 => 1.35,
        CodecChoice::Hevc => 1.0,
        CodecChoice::Av1 => 0.85,
    };
    let pixels = (size.0 as f64 * size.1 as f64 / (1920.0 * 1080.0)).powf(0.8);
    let frames = (fps as f64 / 60.0).powf(0.6);
    let kbps = base * quality * codec * pixels * frames;
    // Whole megabits read better.
    ((kbps / 1000.0).round().max(2.0) * 1000.0) as u32
}

/// Runs the benchmark and returns the recommendation built on `current`.
///
/// `clip_secs` and `ram` keep the replay buffer to an eighth of the PC's memory.
pub fn run(
    answers: &Answers,
    screen: &Screen,
    current: &VideoPrefs,
    clip_secs: u32,
    ram: Option<u64>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Report> {
    let gpu = Gpu::new()?;
    let fps = target_fps(answers, screen);
    let tuning = tuning(answers.cost);
    let need = fps as f64 * headroom(answers.cost);

    progress(Progress {
        label: "Checking which codecs this graphics card can encode".into(),
        trial: None,
    });
    let supported: Vec<CodecChoice> = [CodecChoice::H264, CodecChoice::Hevc, CodecChoice::Av1]
        .into_iter()
        .filter(|&codec| {
            VideoEncoder::new(&gpu, settings_for(codec, (1280, 720), 30, 1, tuning)).is_ok()
        })
        .collect();
    let codecs = codec_order(answers, &supported);
    if codecs.is_empty() {
        bail!("This graphics card's video encoder could not be started.");
    }

    let mut trials = Vec::new();
    // (codec rank, size rank, trial) of the best pass so far.
    let mut chosen: Option<Trial> = None;
    let mut fastest: Option<Trial> = None;
    'sizes: for size in sizes(answers, screen) {
        progress(Progress {
            label: format!("Preparing test frames at {}×{}", size.0, size.1),
            trial: None,
        });
        let frames = test_frames(&gpu, size)?;
        let mut best_here: Option<Trial> = None;
        for &codec in &codecs {
            for preset in PRESETS {
                if cancel.load(Ordering::Relaxed) {
                    bail!("cancelled");
                }
                progress(Progress {
                    label: format!(
                        "Testing {} at {}×{}, {fps} fps, preset {preset}",
                        codec_name(codec),
                        size.0,
                        size.1
                    ),
                    trial: None,
                });
                let measured = measure(&gpu, codec, size, fps, preset, tuning, &frames);
                let trial = Trial {
                    codec,
                    width: size.0,
                    height: size.1,
                    fps,
                    preset,
                    measured_fps: measured.as_ref().ok().copied(),
                    load: measured.as_ref().ok().map(|m| fps as f64 / m),
                    passed: measured.as_ref().is_ok_and(|&m| m >= need),
                    error: measured.as_ref().err().map(|e| format!("{e:#}")),
                };
                progress(Progress {
                    label: String::new(),
                    trial: Some(trial.clone()),
                });
                trials.push(trial.clone());
                if trial.measured_fps.is_some()
                    && fastest
                        .as_ref()
                        .is_none_or(|f| f.measured_fps < trial.measured_fps)
                {
                    fastest = Some(trial.clone());
                }
                if trial.error.is_some() {
                    break;
                }
                if trial.passed {
                    if best_here.as_ref().is_none_or(|b| b.preset < trial.preset) {
                        best_here = Some(trial);
                    }
                    break;
                }
            }
            // A preferred codec at a middle preset or better beats trying the others.
            if best_here.as_ref().is_some_and(|b| b.preset >= 4) {
                break;
            }
        }
        if best_here.is_some() {
            chosen = best_here;
            break 'sizes;
        }
    }

    let mut notes = Vec::new();
    let compromise = chosen.is_none();
    let pick = match chosen.or(fastest) {
        Some(pick) => pick,
        None => bail!("No setting could be measured. Is the graphics card's driver up to date?"),
    };

    let mut video = current.clone();
    video.codec = pick.codec;
    video.fps = pick.fps;
    video.preset = pick.preset;
    video.width = None;
    video.height = None;
    if answers.size.is_some()
        || (pick.width, pick.height) != (screen.width & !1, screen.height & !1)
    {
        video.width = Some(pick.width);
        video.height = Some(pick.height);
    }
    video.rate_mode = RateMode::Vbr;
    let mut kbps = bitrate_kbps(answers, pick.codec, (pick.width, pick.height), pick.fps);
    let mut peak = kbps / 2 * 3;
    // The replay buffer lives in memory and reserves room for the peak bitrate.
    if let Some(ram) = ram {
        let budget_kbps = (ram / 8 * 8 / 1000 / (clip_secs.max(1) as u64 + 1)) as u32;
        if peak > budget_kbps {
            peak = budget_kbps / 1000 * 1000;
            kbps = (peak / 3 * 2 / 1000 * 1000).max(2000);
            notes.push(format!(
                "The bitrate is held back so a {clip_secs}-second replay buffer stays within an eighth of this PC's memory."
            ));
        }
    }
    video.bitrate_kbps = kbps;
    video.max_bitrate_kbps = peak.max(kbps);
    video.keyframe_secs = 1;
    video.tune = tuning.tune;
    video.multipass = tuning.multipass;
    video.lookahead = tuning.lookahead;
    video.spatial_aq = tuning.spatial_aq;
    video.temporal_aq = tuning.temporal_aq;
    video.aq_strength = tuning.aq_strength;
    video.scaling = answers.scaling;

    let measured = pick.measured_fps.unwrap_or(0.0);
    if compromise {
        notes.insert(
            0,
            format!(
                "Nothing tested left the headroom you asked for, so this is the fastest setting there is. The encoder managed {measured:.0} fps against {fps} recorded; expect some dropped frames, or record fewer frames per second."
            ),
        );
    } else {
        notes.insert(
            0,
            format!(
                "The encoder managed {measured:.0} fps at these settings, so recording {fps} fps uses about {:.0}% of it.",
                fps as f64 / measured.max(1.0) * 100.0
            ),
        );
    }
    if answers.size.is_none() && video.width.is_some() {
        notes.push(format!(
            "Your screen's {}×{} was too much for the encoder at {fps} fps, so clips are scaled down to {}×{}.",
            screen.width, screen.height, pick.width, pick.height
        ));
    }
    if answers.share && pick.codec == CodecChoice::H264 {
        notes.push("H.264, because it plays everywhere clips get shared.".into());
    } else if pick.codec != codecs[0] {
        notes.push(format!(
            "{} instead of {}, which could only keep up at a much faster, lower-quality preset.",
            codec_name(pick.codec),
            codec_name(codecs[0])
        ));
    }
    Ok(Report {
        video,
        trials,
        notes,
        compromise,
    })
}

fn codec_name(codec: CodecChoice) -> &'static str {
    match codec {
        CodecChoice::H264 => "H.264",
        CodecChoice::Hevc => "HEVC",
        CodecChoice::Av1 => "AV1",
    }
}

fn settings_for(
    codec: CodecChoice,
    size: (u32, u32),
    fps: u32,
    preset: u8,
    tuning: EncoderTuning,
) -> VideoSettings {
    VideoSettings {
        codec: match codec {
            CodecChoice::H264 => VideoCodec::H264,
            CodecChoice::Hevc => VideoCodec::Hevc,
            CodecChoice::Av1 => VideoCodec::Av1,
        },
        width: size.0,
        height: size.1,
        fps,
        hdr10: false,
        // Busy footage at a generous bitrate, so the encoder does real work.
        rate: RateControl::Vbr {
            kbps: 60_000,
            max_kbps: 90_000,
        },
        preset,
        keyframe_interval_secs: 1,
        tuning,
    }
}

/// Frames per second the encoder manages flat out.
fn measure(
    gpu: &Gpu,
    codec: CodecChoice,
    size: (u32, u32),
    fps: u32,
    preset: u8,
    tuning: EncoderTuning,
    frames: &[ID3D11Texture2D],
) -> Result<f64> {
    let mut encoder = VideoEncoder::new(gpu, settings_for(codec, size, fps, preset, tuning))?;
    let mut sink = |_packet: crate::Packet| {};
    let mut pts = 0i64;
    for _ in 0..WARMUP_FRAMES {
        encoder.encode_texture(&frames[pts as usize % frames.len()], pts, &mut sink)?;
        pts += 1;
    }
    let start = Instant::now();
    let mut count = 0u64;
    while start.elapsed() < MEASURE {
        encoder.encode_texture(&frames[pts as usize % frames.len()], pts, &mut sink)?;
        pts += 1;
        count += 1;
    }
    encoder.finish(&mut sink)?;
    let secs = start.elapsed().as_secs_f64();
    Ok(count as f64 / secs)
}

/// Test frames: one detailed scene, seen through a window that moves a few pixels each frame,
/// so the encoder has detail to keep and motion to find, as in a game.
fn test_frames(gpu: &Gpu, size: (u32, u32)) -> Result<Vec<ID3D11Texture2D>> {
    const STEP: (usize, usize) = (6, 3);
    let (width, height) = (size.0 as usize, size.1 as usize);
    let scene_w = width + STEP.0 * FRAMES;
    let scene_h = height + STEP.1 * FRAMES;
    let mut scene = vec![0u8; scene_w * scene_h * 4];
    let mut seed = 0x9E37_79B9u32;
    let mut block_noise = vec![0u8; scene_w.div_ceil(8) * scene_h.div_ceil(8)];
    for value in &mut block_noise {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *value = (seed >> 24) as u8;
    }
    let blocks_per_row = scene_w.div_ceil(8);
    for y in 0..scene_h {
        let row = &mut scene[y * scene_w * 4..(y + 1) * scene_w * 4];
        let block_row = (y / 8) * blocks_per_row;
        for x in 0..scene_w {
            let noise = block_noise[block_row + x / 8] as u32;
            // Gradients for sky and ground, blocky detail for texture, and fine stripes.
            let fine = ((x ^ y) & 7) as u32 * 6;
            let r = ((x * 255 / scene_w) as u32 / 2 + noise / 2 + fine) as u8;
            let g = ((y * 255 / scene_h) as u32 / 2 + noise / 3 + fine / 2) as u8;
            let b = (noise / 2 + ((x / 32 + y / 32) & 1) as u32 * 90) as u8;
            row[x * 4..x * 4 + 4].copy_from_slice(&[b, g, r, 255]);
        }
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let mut frames = Vec::with_capacity(FRAMES);
    for i in 0..FRAMES {
        let offset = ((i * STEP.1) * scene_w + i * STEP.0) * 4;
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: scene[offset..].as_ptr() as *const _,
            SysMemPitch: (scene_w * 4) as u32,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe {
            gpu.device
                .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
                .context("cannot create a test frame")?;
        }
        frames.push(texture.context("no test frame")?);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(game: GameKind, cost: Cost) -> Answers {
        Answers {
            game,
            cost,
            size: None,
            fps: None,
            scaling: Scaling::Stretch,
            share: false,
        }
    }

    const SCREEN: Screen = Screen {
        width: 2560,
        height: 1440,
        refresh_hz: 165,
    };

    #[test]
    fn shooters_get_more_frames_when_performance_allows() {
        assert_eq!(
            target_fps(&answers(GameKind::Shooter, Cost::Least), &SCREEN),
            60
        );
        assert_eq!(
            target_fps(&answers(GameKind::Shooter, Cost::Some), &SCREEN),
            120
        );
        assert_eq!(
            target_fps(&answers(GameKind::Shooter, Cost::Most), &SCREEN),
            144
        );
        assert_eq!(
            target_fps(&answers(GameKind::Story, Cost::Most), &SCREEN),
            60
        );
    }

    #[test]
    fn frame_rate_stays_within_the_screen_and_a_fixed_one_wins() {
        let slow = Screen {
            refresh_hz: 100,
            ..SCREEN
        };
        assert_eq!(
            target_fps(&answers(GameKind::Shooter, Cost::Most), &slow),
            100
        );
        let mut fixed = answers(GameKind::Story, Cost::Least);
        fixed.fps = Some(240);
        assert_eq!(target_fps(&fixed, &slow), 240);
    }

    #[test]
    fn sharing_prefers_h264_and_best_quality_prefers_av1() {
        let all = [CodecChoice::H264, CodecChoice::Hevc, CodecChoice::Av1];
        let mut share = answers(GameKind::Mixed, Cost::Some);
        share.share = true;
        assert_eq!(codec_order(&share, &all)[0], CodecChoice::H264);
        assert_eq!(
            codec_order(&answers(GameKind::Mixed, Cost::Most), &all)[0],
            CodecChoice::Av1
        );
        assert_eq!(
            codec_order(&answers(GameKind::Mixed, Cost::Most), &all[..2])[0],
            CodecChoice::Hevc
        );
    }

    #[test]
    fn smaller_sizes_keep_the_screen_shape() {
        let wide = Screen {
            width: 3440,
            height: 1440,
            refresh_hz: 144,
        };
        assert_eq!(
            sizes(&answers(GameKind::Story, Cost::Some), &wide),
            vec![(3440, 1440), (2580, 1080), (1720, 720)]
        );
        let mut fixed = answers(GameKind::Story, Cost::Some);
        fixed.size = Some((1921, 1080));
        assert_eq!(sizes(&fixed, &wide), vec![(1920, 1080)]);
    }

    #[test]
    fn bitrates_are_sensible() {
        let a = answers(GameKind::Shooter, Cost::Some);
        assert_eq!(
            bitrate_kbps(&a, CodecChoice::Hevc, (1920, 1080), 60),
            25_000
        );
        let h264 = bitrate_kbps(&a, CodecChoice::H264, (1920, 1080), 60);
        let av1 = bitrate_kbps(&a, CodecChoice::Av1, (1920, 1080), 60);
        assert!(h264 > 25_000 && av1 < 25_000);
        // Double the frames costs far less than double the bits.
        let doubled = bitrate_kbps(&a, CodecChoice::Hevc, (1920, 1080), 120);
        assert!((35_000..41_000).contains(&doubled), "{doubled}");
        let story = bitrate_kbps(
            &answers(GameKind::Story, Cost::Least),
            CodecChoice::Hevc,
            (1920, 1080),
            60,
        );
        assert!(story < 20_000, "{story}");
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;

    /// Runs the real benchmark on this PC's GPU: `cargo test -p umbra-core benchmark -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn on_this_gpu() {
        let screen = Screen {
            width: 2560,
            height: 1440,
            refresh_hz: 165,
        };
        for cost in [Cost::Least, Cost::Some, Cost::Most] {
            let answers = Answers {
                game: GameKind::Shooter,
                cost,
                size: None,
                fps: None,
                scaling: Scaling::Stretch,
                share: false,
            };
            let started = Instant::now();
            let report = run(
                &answers,
                &screen,
                &VideoPrefs::default(),
                30,
                crate::estimate::total_ram(),
                &AtomicBool::new(false),
                &mut |p| {
                    if let Some(t) = p.trial {
                        println!(
                            "  {:?} {}x{} p{} -> {:?} fps, passed {}",
                            t.codec,
                            t.width,
                            t.height,
                            t.preset,
                            t.measured_fps.map(|f| f.round()),
                            t.passed
                        );
                    }
                },
            )
            .unwrap();
            let v = &report.video;
            println!(
                "{cost:?}: {:?} {:?}x{:?} {} fps p{} {} kbit/s (peak {}) in {:.1}s\n  {}",
                v.codec,
                v.width,
                v.height,
                v.fps,
                v.preset,
                v.bitrate_kbps,
                v.max_bitrate_kbps,
                started.elapsed().as_secs_f64(),
                report.notes.join("\n  ")
            );
            assert!(!report.trials.is_empty());
        }
    }
}
