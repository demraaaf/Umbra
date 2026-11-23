//! Trim and crop export.
//!
//! A cut that starts on a keyframe and has no crop is written by copying the compressed
//! packets, so it takes a moment and loses nothing. Anything else decodes the video,
//! crops it and encodes it again with the same codec. Audio is copied, except a track
//! whose volume changes, and the combined first track when a track in it changes volume
//! or is left out: that one is mixed again from its parts.
//!
//! The decision rule, in order:
//!
//! 1. A crop that changes the picture means re-encode.
//! 2. A start that is not on a keyframe (within half a frame) means re-encode.
//! 3. Video with B-frames means re-encode, because a cut in the middle of reordered
//!    frames cannot be exact. Umbra's own clips have none.
//! 4. Otherwise the packets are copied.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::error::EAGAIN;
use ffmpeg_next::ffi::{
    AV_CODEC_FLAG_GLOBAL_HEADER, AV_FRAME_CROP_UNALIGNED, AV_NOPTS_VALUE, AVCodecContext,
    AVCodecID, AVColorPrimaries, AVColorRange, AVColorSpace, AVColorTransferCharacteristic,
    AVERROR_EOF, AVFrame, AVPictureType, AVPixelFormat, AVRational, AVRounding,
    AVSEEK_FLAG_BACKWARD, av_frame_apply_cropping, av_frame_copy_props, av_frame_unref,
    av_hwframe_transfer_data, av_opt_set, av_rescale_q_rnd, av_seek_frame, avcodec_alloc_context3,
    avcodec_find_encoder_by_name, avcodec_free_context, avcodec_open2,
    avcodec_parameters_from_context, avcodec_receive_packet, avcodec_send_frame,
};
use ffmpeg_next::packet::Mut as _;
use ffmpeg_next::software::scaling;
use ffmpeg_next::threading::{Config as ThreadConfig, Type as ThreadType};
use ffmpeg_next::util::format::Pixel;
use ffmpeg_next::{Error as AvError, Packet, Rational, codec, format, frame, media};
use serde::{Deserialize, Serialize};

use crate::aac::AacEncoder;
use crate::align::{CHUNK, SAMPLE_RATE};
pub use crate::burn::{CameraBurn, InputBurn, LogoBurn};
use crate::encode::StreamInfo;
use crate::library::{MixRole, init_ffmpeg, mix_roles, stream_title};
use crate::mux::Mp4Writer;
use crate::unmix::{AppInOthers, find_apps_in_others};

const AVERROR_EAGAIN: i32 = -11;
/// The smallest crop edge accepted.
const MIN_CROP: u32 = 16;
/// Re-encoded video never gets less than this, in bit/s.
const MIN_BITRATE: i64 = 8_000_000;
/// Progress is reported at most this often (and at most once per percent otherwise).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------------------

/// A rectangle of the video in source pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct ExportJob {
    pub input: PathBuf,
    /// Where to write. Must not be `input`; the caller picks a temporary name to replace
    /// the original. The file appears atomically, so nothing half-written ever has this name.
    pub output: PathBuf,
    /// Seconds from the start of the clip. Clamped to the clip.
    pub start_secs: f64,
    pub end_secs: f64,
    pub crop: Option<Crop>,
    /// Audio tracks to keep, as indices among the file's audio streams (the `index` of
    /// `library::AudioTrackMeta`). Empty keeps no audio. Unknown indices are an error.
    pub audio_tracks: Vec<usize>,
    /// Volume for kept tracks, as (track index, factor). Tracks not listed keep their
    /// volume. A factor other than 1 decodes and re-encodes that track.
    pub audio_gains: Vec<(usize, f32)>,
    /// Ranges to leave out, as (start, end) in seconds from the start of the clip. The parts
    /// on either side are joined. Overlapping ranges are merged; anything outside the cut
    /// between `start_secs` and `end_secs` is ignored. Any range means a re-encode, unless
    /// `keyframe_cuts` is set.
    pub cuts: Vec<(f64, f64)>,
    /// Move the start and the end of each cut to the nearest keyframe, so the packets can be
    /// copied instead of re-encoded: minutes instead of hours for a long recording, at the
    /// price of cuts up to half a keyframe interval off. Ignored when the video is
    /// re-encoded anyway (crop, size, frame rate, bitrate, codec).
    pub keyframe_cuts: bool,
    /// Re-encode even when the packets could be copied.
    pub force_reencode: bool,
    /// Use this video encoder (for example `libopenh264`) instead of picking one. It has
    /// to produce the same codec as the input.
    pub encoder: Option<String>,
    /// Processor threads for decoding the video, 0 for one per core.
    pub threads: u32,
    /// Scale the picture (after cropping) to this size. Both even.
    pub scale: Option<(u32, u32)>,
    /// At most this many frames per second; frames in between are left out.
    pub max_fps: Option<u32>,
    /// Video bitrate in bit/s, for example to fit a size limit. Otherwise it follows the source.
    pub video_bitrate: Option<i64>,
    /// Encode to this codec instead of the source's.
    pub codec: Option<crate::encode::VideoCodec>,
    /// Write only the combined first track, with the tracks that are normally only on their
    /// own (the microphone) mixed in, for players and sites that play only one track.
    pub mix_only: bool,
    /// Encode with a faster setting of the hardware encoder: about twice as fast, at a
    /// slightly lower quality for the bitrate. For work done in the background on long files.
    pub quick: bool,
    /// Draw the clip's camera over the picture (after the crop and scale). Means a re-encode.
    pub camera: Option<CameraBurn>,
    /// Draw a picture (a logo) over the video. Means a re-encode.
    pub logo: Option<LogoBurn>,
    /// Draw the clip's keyboard and mouse input over the video (when it has an input file).
    /// Means a re-encode.
    pub input_overlay: Option<InputBurn>,
    /// Copy the camera stream as it is, and keep the clip's camera tag, for an export that
    /// stays the same clip (shrinking it, burning in other layers). Otherwise the camera is
    /// left out. Meant for whole clips: cuts do not apply to it.
    pub keep_camera: bool,
    /// Keep the clip's logo tag, so the logo still shows over it in Umbra. Otherwise it is
    /// drawn in (`logo`) or left out.
    pub keep_logo: bool,
}

impl ExportJob {
    /// Keeps the whole frame and no audio track; fill in `crop` and `audio_tracks`.
    pub fn new(
        input: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        start_secs: f64,
        end_secs: f64,
    ) -> Self {
        Self {
            input: input.into(),
            output: output.into(),
            start_secs,
            end_secs,
            crop: None,
            audio_tracks: Vec::new(),
            audio_gains: Vec::new(),
            cuts: Vec::new(),
            keyframe_cuts: false,
            force_reencode: false,
            encoder: None,
            threads: 0,
            scale: None,
            max_fps: None,
            video_bitrate: None,
            codec: None,
            mix_only: false,
            quick: false,
            camera: None,
            logo: None,
            input_overlay: None,
            keep_camera: false,
            keep_logo: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportMode {
    /// Packets are copied; no quality loss.
    StreamCopy,
    /// Video is decoded, cropped and encoded again.
    Reencode,
}

/// How a job will be carried out.
#[derive(Clone, Debug)]
pub struct ExportPlan {
    pub mode: ExportMode,
    /// Why the video has to be re-encoded. `None` for a stream copy.
    pub reason: Option<String>,
    /// Where the output really starts: the first kept frame, in seconds from the start of
    /// the clip. A cut is rounded to the nearest frame, and a copy to the nearest keyframe
    /// within half a frame.
    pub start_secs: f64,
    /// The requested end after clamping to the clip.
    pub end_secs: f64,
    /// The crop after rounding to even values inside the frame; `None` when it keeps the
    /// whole frame.
    pub crop: Option<Crop>,
    /// The ranges left out after merging and rounding to frames, in seconds from the start
    /// of the clip.
    pub cuts: Vec<(f64, f64)>,
}

/// What an export did.
#[derive(Clone, Debug)]
pub struct ExportReport {
    pub plan: ExportPlan,
    /// Video encoder used, for a re-encode.
    pub encoder: Option<String>,
    pub video_frames: u64,
    /// Target video bitrate of a re-encode.
    pub video_bitrate_kbps: Option<u32>,
    pub elapsed_secs: f64,
    pub output_bytes: u64,
}

/// The error of an export that the progress callback stopped. Test for it with
/// [`is_cancelled`]. A cancelled export leaves no output file.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("export cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_cancelled(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Cancelled>().is_some()
}

/// Rounds a crop to what the video format allows and checks it against the frame:
/// `x`, `y`, `width` and `height` become even (down), the rectangle is pulled inside the
/// frame, and a crop that covers the whole frame becomes `None`.
pub fn normalize_crop(crop: Crop, frame_width: u32, frame_height: u32) -> Result<Option<Crop>> {
    if crop.width == 0 || crop.height == 0 {
        bail!("the crop is empty");
    }
    if crop.x >= frame_width || crop.y >= frame_height {
        bail!(
            "the crop starts at {},{} outside the {frame_width}x{frame_height} video",
            crop.x,
            crop.y
        );
    }
    let x = crop.x & !1;
    let y = crop.y & !1;
    let width = crop.width.min(frame_width - x) & !1;
    let height = crop.height.min(frame_height - y) & !1;
    if width < MIN_CROP || height < MIN_CROP {
        bail!("the crop is too small ({width}x{height}); the smallest is {MIN_CROP}x{MIN_CROP}");
    }
    if (x, y, width, height) == (0, 0, frame_width & !1, frame_height & !1) {
        return Ok(None);
    }
    Ok(Some(Crop {
        x,
        y,
        width,
        height,
    }))
}

// ---------------------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------------------

/// Decides how `job` would be exported, without writing anything. The UI can use it to
/// tell the user whether a cut is lossless. Reads only the input file.
pub fn plan(job: &ExportJob) -> Result<ExportPlan> {
    init_ffmpeg()?;
    let (mut input, source) = open_source(&job.input)?;
    Ok(make_plan(job, &source, &mut input)?.plan)
}

/// Exports `job`. `progress` gets the fraction done (0 to 1) every so often and returns
/// `false` to cancel; a cancelled export returns an error for which [`is_cancelled`] is
/// true and leaves no output. The output is written as `<output>.part` and renamed when
/// complete, so a failure or a crash never leaves a broken file under the real name.
pub fn export(job: &ExportJob, progress: &mut dyn FnMut(f32) -> bool) -> Result<()> {
    export_with_report(job, progress).map(|_| ())
}

/// Like [`export`], and says what was done.
pub fn export_with_report(
    job: &ExportJob,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<ExportReport> {
    init_ffmpeg()?;
    let began = Instant::now();
    if same_file(&job.input, &job.output) {
        bail!("the output must be a different file from the input");
    }
    let (mut input, source) = open_source(&job.input)?;
    let tracks = resolve_tracks(&source, &job.audio_tracks)?;
    let mut planned = make_plan(job, &source, &mut input)?;

    if let Some(parent) = job.output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let part = {
        let mut name = job.output.clone().into_os_string();
        name.push(".part");
        PathBuf::from(name)
    };

    let mut ctl = Progress::new(progress);
    let result = ctl.report(0.0).and_then(|()| {
        let mut run = Run {
            input: &mut input,
            source: &source,
            tracks: &tracks,
            part: &part,
            job,
        };
        if planned.plan.mode == ExportMode::StreamCopy {
            match run.copy(&planned, &mut ctl) {
                Err(err) if err.is::<NeedsReencode>() => {
                    // Video with B-frames that the first look at the start did not show.
                    log::info!("{} has reordered frames; re-encoding", job.input.display());
                    let _ = fs::remove_file(&part);
                    planned.plan.mode = ExportMode::Reencode;
                    planned.plan.reason = Some("the video has B-frames".into());
                }
                other => return other,
            }
        }
        run.reencode(&planned, &mut ctl)
    });

    let stats = match result {
        Ok(stats) => stats,
        Err(err) => {
            let _ = fs::remove_file(&part);
            return Err(err);
        }
    };
    if let Err(err) = fs::rename(&part, &job.output) {
        let _ = fs::remove_file(&part);
        return Err(err).with_context(|| format!("cannot write {}", job.output.display()));
    }
    let _ = ctl.report(1.0);

    Ok(ExportReport {
        plan: planned.plan,
        encoder: stats.encoder,
        video_frames: stats.frames,
        video_bitrate_kbps: stats.bitrate.map(|b| (b / 1000) as u32),
        elapsed_secs: began.elapsed().as_secs_f64(),
        output_bytes: fs::metadata(&job.output).map_or(0, |m| m.len()),
    })
}

/// True when both paths name the same file, existing or not.
fn same_file(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| -> Option<String> {
        let full = match fs::canonicalize(p) {
            Ok(full) => full,
            Err(_) => {
                let parent = p
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                fs::canonicalize(parent).ok()?.join(p.file_name()?)
            }
        };
        Some(full.to_string_lossy().to_lowercase())
    };
    match (resolve(a), resolve(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------
// Progress and cancel
// ---------------------------------------------------------------------------------------

struct Progress<'a> {
    callback: &'a mut dyn FnMut(f32) -> bool,
    last_call: Option<Instant>,
    last_value: f32,
}

impl<'a> Progress<'a> {
    fn new(callback: &'a mut dyn FnMut(f32) -> bool) -> Self {
        Self {
            callback,
            last_call: None,
            last_value: -1.0,
        }
    }

    /// Reports `fraction` (0 to 1), unless it was reported a moment ago. Fails with
    /// [`Cancelled`] when the callback asks to stop.
    fn report(&mut self, fraction: f64) -> Result<()> {
        let value = fraction.clamp(0.0, 1.0) as f32;
        let now = Instant::now();
        if let Some(last) = self.last_call
            && now.duration_since(last) < PROGRESS_INTERVAL
            && (value - self.last_value).abs() < 0.01
        {
            return Ok(());
        }
        self.last_call = Some(now);
        self.last_value = value;
        if (self.callback)(value) {
            Ok(())
        } else {
            Err(Cancelled.into())
        }
    }
}

// ---------------------------------------------------------------------------------------
// Reading the input
// ---------------------------------------------------------------------------------------

struct AudioSource {
    stream_index: usize,
    time_base: Rational,
    params: codec::Parameters,
    title: Option<String>,
}

struct Source {
    video_index: usize,
    time_base: Rational,
    params: codec::Parameters,
    codec_id: AVCodecID,
    width: u32,
    height: u32,
    /// Timestamp of the first frame, in `time_base` ticks. Times in a job count from here.
    origin: i64,
    /// Timestamp just after the last frame.
    end: i64,
    /// Length of one frame in ticks (at least 1).
    frame_ticks: i64,
    fps: Rational,
    /// Video bitrate in bit/s; 0 if unknown.
    bit_rate: i64,
    stream_count: usize,
    audio: Vec<AudioSource>,
    /// Umbra's file tags (how the clip was recorded), carried over to the export.
    tags: Vec<(String, String)>,
}

fn rescale(value: i64, from: Rational, to: Rational) -> i64 {
    unsafe { av_rescale_q_rnd(value, from.into(), to.into(), AVRounding::AV_ROUND_NEAR_INF) }
}

fn secs_to_ticks(secs: f64, time_base: Rational) -> i64 {
    (secs * time_base.denominator() as f64 / time_base.numerator() as f64).round() as i64
}

fn ticks_to_secs(ticks: i64, time_base: Rational) -> f64 {
    ticks as f64 * time_base.numerator() as f64 / time_base.denominator() as f64
}

fn open_source(path: &Path) -> Result<(format::context::Input, Source)> {
    let input = format::input(path).with_context(|| format!("cannot open {}", path.display()))?;
    let source = {
        let video = crate::library::screen_stream(&input)
            .ok_or_else(|| anyhow!("{} has no video stream", path.display()))?;
        let time_base = video.time_base();
        if time_base.numerator() <= 0 || time_base.denominator() <= 0 {
            bail!("the video stream has no usable time base");
        }
        let params = video.parameters();
        let (width, height, codec_id, mut bit_rate) = unsafe {
            let raw = params.as_ptr();
            (
                (*raw).width.max(0) as u32,
                (*raw).height.max(0) as u32,
                (*raw).codec_id,
                (*raw).bit_rate,
            )
        };
        let start = video.start_time();
        let origin = if start == AV_NOPTS_VALUE { 0 } else { start };
        let end = if video.duration() > 0 {
            origin + video.duration()
        } else if input.duration() > 0 {
            origin + rescale(input.duration(), Rational::new(1, 1_000_000), time_base)
        } else {
            i64::MAX
        };

        let valid = |r: Rational| r.numerator() > 0 && r.denominator() > 0;
        let mut fps = crate::library::frame_rate(&input, &video);
        if !valid(fps) {
            fps = video.rate();
        }
        let mut frame_ticks = if valid(fps) {
            (time_base.denominator() as f64 * fps.denominator() as f64
                / (time_base.numerator() as f64 * fps.numerator() as f64))
                .round() as i64
        } else {
            0
        };
        if frame_ticks < 1 {
            // No frame rate in the file; assume 30 fps.
            frame_ticks = (time_base.denominator() / time_base.numerator() / 30).max(1) as i64;
            fps = Rational::new(
                time_base.denominator(),
                time_base.numerator() * frame_ticks as i32,
            );
        }
        if bit_rate <= 0 {
            // Whole file minus nothing: good enough as a target for a rough bitrate.
            let seconds = ticks_to_secs(end - origin, time_base);
            if let Ok(meta) = fs::metadata(path)
                && seconds > 0.0
                && end != i64::MAX
            {
                bit_rate = (meta.len() as f64 * 8.0 / seconds) as i64;
            }
        }

        let audio = input
            .streams()
            .filter(|s| s.parameters().medium() == media::Type::Audio)
            .map(|s| AudioSource {
                stream_index: s.index(),
                time_base: s.time_base(),
                params: s.parameters().clone(),
                title: stream_title(&s.metadata()),
            })
            .collect();

        Source {
            video_index: video.index(),
            time_base,
            params: params.clone(),
            codec_id,
            width,
            height,
            origin,
            end,
            frame_ticks,
            fps,
            bit_rate,
            stream_count: input.nb_streams() as usize,
            audio,
            tags: input
                .metadata()
                .iter()
                .filter(|(k, _)| k.starts_with(crate::mux::TAG_PREFIX))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    };
    Ok((input, source))
}

/// The requested audio tracks, sorted and without repeats, checked against the file.
fn resolve_tracks(source: &Source, requested: &[usize]) -> Result<Vec<usize>> {
    let mut tracks = requested.to_vec();
    tracks.sort_unstable();
    tracks.dedup();
    if let Some(bad) = tracks.iter().find(|&&t| t >= source.audio.len()) {
        bail!(
            "the clip has no audio track {bad} (it has {})",
            source.audio.len()
        );
    }
    Ok(tracks)
}

/// Seeks to the keyframe at or before `ts` (in the video stream's time base).
fn seek_to(input: &mut format::context::Input, stream: usize, ts: i64) -> Result<()> {
    let code =
        unsafe { av_seek_frame(input.as_mut_ptr(), stream as i32, ts, AVSEEK_FLAG_BACKWARD) };
    if code < 0 {
        bail!("cannot seek in the input: {}", AvError::from(code));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------------------

/// The range of video timestamps a job keeps, in ticks of the video time base.
struct Window {
    /// Requested end (exclusive), clamped to the clip. Audio is cut here.
    end: i64,
    /// A frame is kept if `first_pts <= pts < hi`, which rounds both cuts to the nearest
    /// frame boundary.
    hi: i64,
}

struct FirstFrame {
    pts: i64,
    key: bool,
    reordered: bool,
}

struct Planned {
    plan: ExportPlan,
    window: Window,
    first: FirstFrame,
    /// The left-out ranges as video timestamps `[lo, hi)`, on frame boundaries.
    cuts: Vec<(i64, i64)>,
}

fn make_plan(
    job: &ExportJob,
    source: &Source,
    input: &mut format::context::Input,
) -> Result<Planned> {
    if !job.start_secs.is_finite() || !job.end_secs.is_finite() {
        bail!("the start and end have to be numbers");
    }
    let crop = match job.crop {
        Some(crop) => normalize_crop(crop, source.width, source.height)?,
        None => None,
    };

    let start = source.origin + secs_to_ticks(job.start_secs.max(0.0), source.time_base);
    let end = source
        .origin
        .saturating_add(secs_to_ticks(job.end_secs, source.time_base))
        .min(source.end);
    if start >= source.end {
        bail!(
            "the start ({:.3} s) is past the end of the video ({:.3} s)",
            job.start_secs,
            ticks_to_secs(source.end - source.origin, source.time_base)
        );
    }
    if end <= start {
        bail!(
            "the end ({:.3} s) has to be after the start ({:.3} s)",
            job.end_secs,
            job.start_secs.max(0.0)
        );
    }
    let half = source.frame_ticks / 2;
    let (lo, hi) = (start - half, end - half);
    let reshape = job.scale.is_some()
        || job.video_bitrate.is_some()
        || job.codec.is_some_and(|c| codec_id_of(c) != source.codec_id)
        || job.max_fps.is_some_and(|max| {
            source.fps.numerator() as f64 / source.fps.denominator().max(1) as f64
                > max as f64 + 0.5
        });
    // An input overlay is drawn only where the clip has input to draw.
    let drawn = job.camera.is_some()
        || job.logo.is_some()
        || (job.input_overlay.is_some()
            && crate::inputfile::existing_sidecar(&job.input).is_some());
    // Copying with cuts on keyframes: the start and every cut's end land on one.
    let snap = job.keyframe_cuts && !job.force_reencode && !reshape && !drawn && crop.is_none();
    let first = if snap {
        let key = nearest_keyframe(input, source, lo.max(source.origin))?;
        find_first_frame(input, source, key.min(hi - 1))?
    } else {
        find_first_frame(input, source, lo)?
    };
    if first.pts >= hi {
        bail!("the selection is shorter than one frame");
    }

    let mut cuts = plan_cuts(job, source, first.pts, hi)?;
    if snap && !first.reordered {
        let mut snapped = Vec::new();
        for (lo_cut, hi_cut) in cuts {
            // The part after a cut has to begin on a keyframe; a cut to the end needs none.
            let resume = if hi_cut >= hi {
                hi_cut
            } else {
                nearest_keyframe(input, source, hi_cut)?
            };
            if resume > lo_cut {
                snapped.push((lo_cut, resume.min(hi)));
            }
        }
        if snapped.iter().map(|(a, b)| b - a).sum::<i64>() >= hi - first.pts {
            bail!("the cuts leave nothing of the clip");
        }
        cuts = snapped;
    }

    let (mode, reason) = if job.force_reencode {
        (
            ExportMode::Reencode,
            Some("re-encoding was requested".to_string()),
        )
    } else if reshape {
        (
            ExportMode::Reencode,
            Some("the size, frame rate, bitrate or codec changes".to_string()),
        )
    } else if drawn {
        (
            ExportMode::Reencode,
            Some("the camera, logo or input is drawn in".to_string()),
        )
    } else if crop.is_some() {
        (
            ExportMode::Reencode,
            Some("the video is cropped".to_string()),
        )
    } else if !cuts.is_empty() && !snap {
        (ExportMode::Reencode, Some("parts are cut out".to_string()))
    } else if first.reordered {
        (
            ExportMode::Reencode,
            Some("the video has B-frames".to_string()),
        )
    } else if !first.key {
        (
            ExportMode::Reencode,
            Some("the start is not on a keyframe".to_string()),
        )
    } else {
        (ExportMode::StreamCopy, None)
    };

    Ok(Planned {
        plan: ExportPlan {
            mode,
            reason,
            start_secs: ticks_to_secs(first.pts - source.origin, source.time_base),
            end_secs: ticks_to_secs(end - source.origin, source.time_base),
            crop,
            cuts: cuts
                .iter()
                .map(|&(lo, hi)| {
                    (
                        ticks_to_secs(lo - source.origin, source.time_base),
                        ticks_to_secs(hi - source.origin, source.time_base),
                    )
                })
                .collect(),
        },
        window: Window { end, hi },
        first,
        cuts,
    })
}

/// The job's cuts as frame-aligned video timestamps inside `[first, hi)`, sorted and merged.
/// Fails when nothing would be left.
fn plan_cuts(job: &ExportJob, source: &Source, first: i64, hi: i64) -> Result<Vec<(i64, i64)>> {
    // Frame boundaries sit half a frame before each frame's timestamp, as for the trim.
    let half = source.frame_ticks / 2;
    let snap = |secs: f64| -> i64 {
        let ticks = secs_to_ticks(secs.max(0.0), source.time_base);
        let frames = (ticks as f64 / source.frame_ticks as f64).round() as i64;
        source.origin + frames * source.frame_ticks - half
    };
    let mut cuts: Vec<(i64, i64)> = Vec::new();
    for &(a, b) in &job.cuts {
        if !a.is_finite() || !b.is_finite() {
            bail!("a cut has to be two numbers");
        }
        let (lo, hi_cut) = (snap(a.min(b)).max(first), snap(a.max(b)).min(hi));
        if hi_cut > lo {
            cuts.push((lo, hi_cut));
        }
    }
    cuts.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (lo, hi_cut) in cuts {
        match merged.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi_cut),
            _ => merged.push((lo, hi_cut)),
        }
    }
    let removed: i64 = merged.iter().map(|(a, b)| b - a).sum();
    if removed >= hi - first {
        bail!("the cuts leave nothing of the clip");
    }
    Ok(merged)
}

/// Maps positions on a timeline with ranges taken out to positions on the joined result.
#[derive(Clone, Debug, Default)]
struct CutMap {
    /// Sorted, non-overlapping `[lo, hi)` ranges.
    cuts: Vec<(i64, i64)>,
}

impl CutMap {
    /// The joined position of `p`, or `None` when `p` falls in a cut.
    fn map(&self, p: i64) -> Option<i64> {
        let mut removed = 0;
        for &(lo, hi) in &self.cuts {
            if p < lo {
                break;
            }
            if p < hi {
                return None;
            }
            removed += hi - lo;
        }
        Some(p - removed)
    }

    /// Like `map`, with positions inside a cut moved to where the cut starts.
    fn map_floor(&self, p: i64) -> i64 {
        let mut removed = 0;
        for &(lo, hi) in &self.cuts {
            if p < lo {
                break;
            }
            if p < hi {
                return lo - removed;
            }
            removed += hi - lo;
        }
        p - removed
    }

    fn is_empty(&self) -> bool {
        self.cuts.is_empty()
    }
}

/// The keyframe nearest to `ts` (the earlier one on a tie), as a video timestamp.
fn nearest_keyframe(input: &mut format::context::Input, source: &Source, ts: i64) -> Result<i64> {
    seek_to(input, source.video_index, ts.max(source.origin))?;
    let (mut before, mut after) = (None::<i64>, None::<i64>);
    let mut seen = 0;
    for (stream, packet) in input.packets() {
        if stream.index() != source.video_index {
            continue;
        }
        seen += 1;
        let Some(pts) = packet.pts().or(packet.dts()) else {
            continue;
        };
        if packet.is_key() {
            if pts <= ts {
                before = Some(pts);
            } else {
                after = Some(pts);
                break;
            }
        }
        // Keyframes come every second or two; this many frames without one means none.
        if seen > 4000 {
            break;
        }
    }
    match (before, after) {
        (Some(b), Some(a)) => Ok(if a - ts < ts - b { a } else { b }),
        (Some(b), None) => Ok(b),
        (None, Some(a)) => Ok(a),
        (None, None) => bail!(
            "no keyframe near {:.1} s",
            ticks_to_secs(ts - source.origin, source.time_base)
        ),
    }
}

/// Finds the first frame a cut starting at `lo` keeps: the first video packet at or after
/// `lo`. Also says whether it is a keyframe and whether the video reorders frames.
fn find_first_frame(
    input: &mut format::context::Input,
    source: &Source,
    lo: i64,
) -> Result<FirstFrame> {
    seek_to(input, source.video_index, lo.max(source.origin))?;
    let mut found: Option<FirstFrame> = None;
    let mut reordered = false;
    let mut lookahead = 0;
    for (stream, packet) in input.packets() {
        if stream.index() != source.video_index {
            continue;
        }
        let Some(pts) = packet.pts().or(packet.dts()) else {
            continue;
        };
        if packet.dts().is_some_and(|dts| dts != pts) {
            reordered = true;
        }
        if pts >= lo && found.as_ref().is_none_or(|f| pts < f.pts) {
            found = Some(FirstFrame {
                pts,
                key: packet.is_key(),
                reordered,
            });
        }
        if found.is_some() {
            // With reordered frames an earlier timestamp can still follow in the stream.
            lookahead += 1;
            if !reordered || lookahead > 16 {
                break;
            }
        }
    }
    let mut first =
        found.ok_or_else(|| anyhow!("the start is past the last frame of the video"))?;
    first.reordered = reordered;
    Ok(first)
}

// ---------------------------------------------------------------------------------------
// Running an export
// ---------------------------------------------------------------------------------------

/// The copy path found video it cannot cut exactly; the job goes to the re-encode path.
#[derive(Debug)]
struct NeedsReencode;

impl fmt::Display for NeedsReencode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the video has to be re-encoded")
    }
}

impl std::error::Error for NeedsReencode {}

#[derive(Default)]
struct Stats {
    frames: u64,
    encoder: Option<String>,
    bitrate: Option<i64>,
}

/// An input stream that is copied, and where its packets go.
struct Route {
    /// Stream in the output file.
    out: usize,
    /// Subtracted from timestamps so the output starts at zero, in the stream's time base.
    offset: i64,
    /// Packets before `lo` or from `hi` on are left out.
    lo: i64,
    hi: i64,
    done: bool,
    /// Set when the track's volume changes: packets are decoded, scaled and encoded again.
    gain: Option<GainStage>,
}

struct Run<'a> {
    input: &'a mut format::context::Input,
    source: &'a Source,
    tracks: &'a [usize],
    part: &'a Path,
    job: &'a ExportJob,
}

impl Run<'_> {
    /// The clip's tags for the output: where the camera and the logo show only while they
    /// stay layers of their own, since otherwise they are drawn in or left out.
    fn out_tags(&self) -> Vec<(String, String)> {
        self.source
            .tags
            .iter()
            .filter(|(k, _)| match k.strip_prefix(crate::mux::TAG_PREFIX) {
                Some("camera") => self.job.keep_camera,
                Some("logo") => self.job.keep_logo,
                _ => true,
            })
            .cloned()
            .collect()
    }

    /// Routes for the kept audio tracks, indexed by input stream, and the output streams
    /// they go to (output stream 0 is the video, which the caller adds).
    fn audio_routes(
        &self,
        planned: &Planned,
        streams: &mut Vec<StreamInfo>,
    ) -> Result<(Vec<Option<Route>>, Vec<MixStage>)> {
        let source = self.source;
        let mut routes: Vec<Option<Route>> = (0..source.stream_count).map(|_| None).collect();
        // Timestamp offset and the kept range of one audio track, in its own time base.
        let window = |audio: &AudioSource| {
            let offset = rescale(planned.first.pts, source.time_base, audio.time_base);
            // A cut that starts at the very beginning keeps everything, including audio
            // that starts slightly before the first frame.
            let lo = if planned.first.pts <= source.origin {
                i64::MIN
            } else {
                offset
            };
            let hi = rescale(planned.window.end, source.time_base, audio.time_base);
            (offset, lo, hi)
        };
        // Volume of every track in the output; a track left out counts as silent.
        let factor_of = |track: usize| -> f32 {
            if !self.tracks.contains(&track) {
                return 0.0;
            }
            self.job
                .audio_gains
                .iter()
                .find(|(index, _)| *index == track)
                .map_or(1.0, |(_, factor)| *factor)
        };
        let titles: Vec<String> = source
            .audio
            .iter()
            .enumerate()
            .map(|(i, a)| {
                a.title
                    .clone()
                    .unwrap_or_else(|| format!("Track {}", i + 1))
            })
            .collect();
        let mix_tag = source
            .tags
            .iter()
            .find(|(k, _)| k.strip_prefix(crate::mux::TAG_PREFIX) == Some("mix"))
            .map(|(_, v)| v.as_str());
        let roles = mix_roles(
            &titles.iter().map(String::as_str).collect::<Vec<_>>(),
            mix_tag,
        );
        // An app with its own track is also inside "Other apps". When its volume differs from
        // that of "Other apps", its copy there is taken out at the offset found by measuring
        // and put back at the app's own volume (see `unmix`).
        let others = titles
            .iter()
            .position(|t| t.eq_ignore_ascii_case("Other apps"));
        let differs =
            |app: usize| others.is_some_and(|o| (factor_of(app) - factor_of(o)).abs() > 0.001);
        let corrections: Vec<AppInOthers> = if roles
            .iter()
            .enumerate()
            .any(|(t, r)| *r == MixRole::InOthers && differs(t))
        {
            match find_apps_in_others(&self.job.input) {
                Ok(found) => found.into_iter().filter(|c| differs(c.app)).collect(),
                Err(err) => {
                    log::warn!("cannot find the apps inside Other apps: {err:#}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        // A correction part: the app's own track, shifted onto "Other apps", at the volume
        // difference times `scale`.
        let correction_parts = |scale: f32| -> Vec<MixPart<'_>> {
            corrections
                .iter()
                .map(|c| {
                    let part = &source.audio[c.app];
                    let (offset, lo, hi) = window(part);
                    MixPart {
                        audio: part,
                        gain: (factor_of(c.app) - factor_of(c.others)) * scale,
                        offset,
                        lo,
                        hi,
                        shift: c.lag,
                    }
                })
                .collect()
        };

        // With `mix_only`, tracks normally only on their own go into the combined one.
        let mix_only = self.job.mix_only;
        let in_combined =
            |role: MixRole| role == MixRole::InMix || (mix_only && role == MixRole::Separate);
        // The combined track is rebuilt when a track in it changes volume or is left out, or
        // when a track of its own is to be mixed in.
        let remix = roles.first() == Some(&MixRole::Mix)
            && self.tracks.contains(&0)
            && (!corrections.is_empty()
                || roles.iter().enumerate().any(|(t, role)| {
                    *role == MixRole::InMix && (factor_of(t) - 1.0).abs() > 0.001
                })
                || (mix_only
                    && roles
                        .iter()
                        .enumerate()
                        .any(|(t, role)| *role == MixRole::Separate && factor_of(t) > 0.0)));

        // The cuts on the audio timeline: 48 kHz samples from the first kept frame.
        let samples = Rational::new(1, SAMPLE_RATE as i32);
        let cuts = CutMap {
            cuts: planned
                .cuts
                .iter()
                .map(|&(lo, hi)| {
                    (
                        rescale(lo - planned.first.pts, source.time_base, samples),
                        rescale(hi - planned.first.pts, source.time_base, samples),
                    )
                })
                .collect(),
        };

        let mut mixers = Vec::new();
        for &track in self.tracks {
            // Only the first track is written; the others still count towards it above.
            if mix_only && track != 0 {
                continue;
            }
            let audio = &source.audio[track];
            let (offset, lo, hi) = window(audio);
            let factor = factor_of(track);
            if track == 0 && remix {
                let mut parts: Vec<MixPart> = roles
                    .iter()
                    .enumerate()
                    .filter(|(_, role)| in_combined(**role))
                    .map(|(t, _)| {
                        let part = &source.audio[t];
                        let (offset, lo, hi) = window(part);
                        MixPart {
                            audio: part,
                            gain: factor_of(t) * factor,
                            offset,
                            lo,
                            hi,
                            shift: 0,
                        }
                    })
                    .collect();
                parts.extend(correction_parts(factor));
                let (stage, info) = MixStage::new(audio, parts, cuts.clone(), streams.len())
                    .context("cannot rebuild the combined track")?;
                streams.push(info);
                mixers.push(stage);
                continue;
            }
            if Some(track) == others && !corrections.is_empty() {
                let mut parts = vec![MixPart {
                    audio,
                    gain: factor,
                    offset,
                    lo,
                    hi,
                    shift: 0,
                }];
                parts.extend(correction_parts(1.0));
                let (stage, info) = MixStage::new(audio, parts, cuts.clone(), streams.len())
                    .context("cannot take the apps out of Other apps")?;
                streams.push(info);
                mixers.push(stage);
                continue;
            }
            // Cutting parts out needs the samples, so the track is decoded and encoded again.
            let gain = if (factor - 1.0).abs() > 0.001 || !cuts.is_empty() {
                let (stage, info) = GainStage::new(audio, factor, cuts.clone())
                    .with_context(|| format!("cannot change the volume of track {}", track + 1))?;
                streams.push(info);
                Some(stage)
            } else {
                streams.push(StreamInfo {
                    params: audio.params.clone(),
                    time_base: audio.time_base,
                    title: audio.title.clone(),
                });
                None
            };
            routes[audio.stream_index] = Some(Route {
                out: streams.len() - 1,
                offset,
                lo,
                hi,
                done: false,
                gain,
            });
        }
        if self.job.keep_camera
            && let Some(camera) = self.input.streams().find(crate::library::is_camera_stream)
        {
            let audio = AudioSource {
                stream_index: camera.index(),
                time_base: camera.time_base(),
                params: camera.parameters(),
                title: Some(crate::camera::CAMERA_TITLE.to_string()),
            };
            let (offset, lo, hi) = window(&audio);
            streams.push(StreamInfo {
                params: audio.params,
                time_base: audio.time_base,
                title: audio.title,
            });
            routes[audio.stream_index] = Some(Route {
                out: streams.len() - 1,
                offset,
                lo,
                hi,
                done: false,
                gain: None,
            });
        }
        Ok((routes, mixers))
    }

    /// Copies packets. Fails with [`NeedsReencode`] if the video turns out to have B-frames.
    fn copy(&mut self, planned: &Planned, ctl: &mut Progress) -> Result<Stats> {
        let source = self.source;
        let mut streams = vec![StreamInfo {
            params: source.params.clone(),
            time_base: source.time_base,
            title: None,
        }];
        let (mut routes, mut mixers) = self.audio_routes(planned, &mut streams)?;
        let mut writer = Mp4Writer::create_with_tags(self.part, &streams, &self.out_tags())?;

        let (first, hi) = (planned.first.pts, planned.window.hi);
        let cut_map = CutMap {
            cuts: planned.cuts.clone(),
        };
        seek_to(self.input, source.video_index, first)?;
        let mut video_done = false;
        let mut frames = 0u64;
        // A part after a cut must begin on a keyframe.
        let mut resuming = true;
        for (_, mut packet) in self.input.packets() {
            let index = packet.stream();
            if index == source.video_index {
                if video_done {
                    continue;
                }
                let Some(pts) = packet.pts().or(packet.dts()) else {
                    continue;
                };
                if packet.dts().is_some_and(|dts| dts != pts) {
                    return Err(NeedsReencode.into());
                }
                if pts < first {
                    continue;
                }
                if pts >= hi {
                    video_done = true;
                } else if let Some(out) = cut_map.map(pts) {
                    if resuming && !packet.is_key() {
                        if frames == 0 {
                            bail!("the cut does not start on a keyframe");
                        }
                        return Err(NeedsReencode.into());
                    }
                    resuming = false;
                    packet.set_pts(Some(out));
                    packet.set_dts(Some(out));
                    writer.write(0, &packet, first)?;
                    frames += 1;
                    ctl.report((pts - first) as f64 / (hi - first) as f64)?;
                } else {
                    resuming = true;
                }
            } else {
                if let Some(Some(route)) = routes.get_mut(index) {
                    forward_audio(route, &packet, &mut writer)?;
                }
                for mixer in &mut mixers {
                    mixer.feed(&packet, &mut writer)?;
                }
            }
            if video_done
                && routes.iter().flatten().all(|r| r.done)
                && mixers.iter().all(MixStage::done)
            {
                break;
            }
        }
        finish_audio(&mut routes, &mut mixers, &mut writer)?;
        writer.finish()?;
        Ok(Stats {
            frames,
            ..Stats::default()
        })
    }

    /// Decodes, crops and encodes the video; copies the audio.
    fn reencode(&mut self, planned: &Planned, ctl: &mut Progress) -> Result<Stats> {
        let source = self.source;
        let crop = planned.plan.crop;
        let (out_width, out_height) = self
            .job
            .scale
            .unwrap_or_else(|| crop.map_or((source.width, source.height), |c| (c.width, c.height)));

        // Frames per second out: the source's, or fewer when capped.
        let source_fps = source.fps.numerator() as f64 / source.fps.denominator().max(1) as f64;
        let out_fps = match self.job.max_fps {
            Some(max) if source_fps > max as f64 + 0.5 => max,
            _ => 0,
        };
        // Ticks between kept frames when frames are dropped (0 keeps every frame).
        let keep_every = if out_fps > 0 {
            (source.time_base.denominator() as f64
                / source.time_base.numerator().max(1) as f64
                / out_fps as f64)
                .round() as i64
        } else {
            0
        };

        // Quality: the bitrate asked for, or the source's bits per pixel and never below the
        // minimum.
        let bit_rate = match self.job.video_bitrate {
            Some(rate) => rate.clamp(150_000, 800_000_000),
            None => {
                let area = (u64::from(out_width) * u64::from(out_height)) as f64
                    / (u64::from(source.width) * u64::from(source.height)).max(1) as f64;
                let frames = if out_fps > 0 {
                    out_fps as f64 / source_fps
                } else {
                    1.0
                };
                ((source.bit_rate as f64 * area * frames.max(0.35)) as i64)
                    .clamp(MIN_BITRATE, 800_000_000)
            }
        };
        let target = self.job.codec.map(codec_id_of).unwrap_or(source.codec_id);

        let mut context = codec::context::Context::from_parameters(source.params.clone())?;
        context.set_threading(ThreadConfig {
            kind: ThreadType::Frame,
            count: self.job.threads as usize,
        });
        // Decoding on the graphics card leaves the processor to the rest; frames are copied
        // back for cropping and scaling. Without a card that decodes this video, software.
        let hardware = unsafe { hw_decode::attach(context.as_mut_ptr()) };
        if hardware {
            log::info!("decoding on the graphics card");
        }
        let mut decoder = context
            .decoder()
            .video()
            .context("cannot open the video decoder")?;
        let mut downloaded = frame::Video::empty();

        let mut encoder = ExportEncoder::open(
            source,
            self.job.encoder.as_deref(),
            out_width,
            out_height,
            bit_rate,
            EncodeShape {
                codec: target,
                fps: (out_fps > 0).then_some(out_fps),
                capped: self.job.video_bitrate.is_some(),
                quick: self.job.quick,
                // Frames from the graphics card are NV12, which hardware encoders take as is.
                nv12: hardware,
            },
        )?;
        log::info!(
            "re-encoding {out_width}x{out_height} with {} at {:.1} Mbit/s",
            encoder.name,
            bit_rate as f64 / 1e6
        );

        let mut streams = vec![encoder.stream_info()?];
        let (mut routes, mut mixers) = self.audio_routes(planned, &mut streams)?;
        let mut tags = self.out_tags();
        if out_fps > 0 {
            // A capped export of a file that left out still frames: its rate is the cap now.
            for (name, value) in &mut tags {
                if name.strip_prefix(crate::mux::TAG_PREFIX) == Some("fps") {
                    *value = out_fps.to_string();
                }
            }
        }
        let mut writer = Mp4Writer::create_with_tags(self.part, &streams, &tags)?;

        let mut prep = FramePrep::new(crop, self.job.scale, encoder.pix);
        let (first, hi) = (planned.first.pts, planned.window.hi);
        let mut burn = crate::burn::Burn::open(
            &self.job.input,
            self.job.camera.as_ref(),
            self.job.logo.as_ref(),
            self.job.input_overlay.as_ref(),
            out_width,
            out_height,
            ticks_to_secs(first, source.time_base),
        )?;
        let cut_map = CutMap {
            cuts: planned.cuts.clone(),
        };
        let frame_ticks = if keep_every > 0 {
            keep_every
        } else {
            source.frame_ticks
        };
        // With a frame-rate cap: the output time the next kept frame may have.
        let mut next_kept = i64::MIN;
        seek_to(self.input, source.video_index, first)?;

        let mut video_done = false;
        let mut frames = 0u64;
        let mut decoded = frame::Video::empty();

        // Handles every frame the decoder has ready. Returns true once the cut's end is reached.
        let mut pump = |decoder: &mut ffmpeg_next::decoder::Video,
                        writer: &mut Mp4Writer,
                        ctl: &mut Progress|
         -> Result<bool> {
            loop {
                match decoder.receive_frame(&mut decoded) {
                    Ok(()) => {}
                    Err(AvError::Other { errno }) if errno == EAGAIN => return Ok(false),
                    Err(AvError::Eof) => return Ok(false),
                    Err(err) => return Err(err).context("video decode failed"),
                }
                let Some(pts) = decoded.timestamp().or(decoded.pts()) else {
                    continue;
                };
                if pts < first {
                    continue;
                }
                if pts >= hi {
                    return Ok(true);
                }
                let Some(out_pts) = cut_map.map(pts) else {
                    continue;
                };
                if keep_every > 0 {
                    // Half a frame of slack keeps the spacing even when timestamps jitter.
                    if out_pts < next_kept.saturating_sub(keep_every / 2) {
                        continue;
                    }
                    next_kept = next_kept.max(out_pts - keep_every / 2) + keep_every;
                }
                // A frame decoded on the graphics card comes back to memory first.
                let source_frame = if unsafe { (*decoded.as_ptr()).format }
                    == AVPixelFormat::AV_PIX_FMT_D3D11 as i32
                {
                    unsafe {
                        av_frame_unref(downloaded.as_mut_ptr());
                        let code =
                            av_hwframe_transfer_data(downloaded.as_mut_ptr(), decoded.as_ptr(), 0);
                        if code < 0 {
                            bail!(
                                "cannot copy a decoded frame from the graphics card: {}",
                                AvError::from(code)
                            );
                        }
                        av_frame_copy_props(downloaded.as_mut_ptr(), decoded.as_ptr());
                    }
                    &mut downloaded
                } else {
                    &mut decoded
                };
                let raw = prep.prepare(source_frame)?;
                if let Some(burn) = burn.as_mut() {
                    burn.draw(
                        raw,
                        ticks_to_secs(pts, source.time_base),
                        ticks_to_secs(pts - source.origin, source.time_base),
                    )?;
                }
                unsafe {
                    (*raw).pts = out_pts - first;
                    (*raw).pict_type = AVPictureType::AV_PICTURE_TYPE_NONE;
                }
                encoder.send(raw, frame_ticks, &mut |packet| writer.write(0, &packet, 0))?;
                frames += 1;
                ctl.report((pts - first) as f64 / (hi - first) as f64)?;
            }
        };

        for (_, packet) in self.input.packets() {
            let index = packet.stream();
            if index == source.video_index {
                if video_done {
                    continue;
                }
                match decoder.send_packet(&packet) {
                    Ok(()) => {}
                    Err(AvError::InvalidData) => {
                        log::warn!("skipping a corrupt video packet");
                        continue;
                    }
                    Err(err) => return Err(err).context("video decode failed"),
                }
                video_done = pump(&mut decoder, &mut writer, ctl)?;
            } else {
                if let Some(Some(route)) = routes.get_mut(index) {
                    forward_audio(route, &packet, &mut writer)?;
                }
                for mixer in &mut mixers {
                    mixer.feed(&packet, &mut writer)?;
                }
            }
            if video_done
                && routes.iter().flatten().all(|r| r.done)
                && mixers.iter().all(MixStage::done)
            {
                break;
            }
        }
        if !video_done {
            // Input ended (or only audio was left to do): let the decoder hand out what it holds.
            let _ = decoder.send_eof();
            pump(&mut decoder, &mut writer, ctl)?;
        }
        encoder.flush(frame_ticks, &mut |packet| writer.write(0, &packet, 0))?;
        finish_audio(&mut routes, &mut mixers, &mut writer)?;
        writer.finish()?;
        Ok(Stats {
            frames,
            encoder: Some(encoder.name.clone()),
            bitrate: Some(bit_rate),
        })
    }
}

/// Copies an audio packet if it lies in the cut.
fn forward_audio(route: &mut Route, packet: &Packet, writer: &mut Mp4Writer) -> Result<()> {
    if route.done {
        return Ok(());
    }
    let Some(pts) = packet.pts().or(packet.dts()) else {
        return Ok(());
    };
    if pts < route.lo {
        return Ok(());
    }
    if pts >= route.hi {
        route.done = true;
        return Ok(());
    }
    if let Some(stage) = route.gain.as_mut() {
        return stage.feed(packet, pts, route.offset, route.out, writer);
    }
    if packet.pts().is_some() && packet.dts().is_some() {
        writer.write(route.out, packet, route.offset)
    } else {
        let mut fixed = packet.clone();
        fixed.set_pts(Some(pts));
        fixed.set_dts(Some(packet.dts().unwrap_or(pts)));
        writer.write(route.out, &fixed, route.offset)
    }
}

/// Flushes the tracks whose volume was changed and the rebuilt combined track.
fn finish_audio(
    routes: &mut [Option<Route>],
    mixers: &mut [MixStage],
    writer: &mut Mp4Writer,
) -> Result<()> {
    for route in routes.iter_mut().flatten() {
        if let Some(stage) = route.gain.as_mut() {
            stage.finish(route.out, writer)?;
        }
    }
    for mixer in mixers {
        mixer.finish(writer)?;
    }
    Ok(())
}

/// Decodes an AAC track, scales its volume and encodes it again.
struct GainStage {
    decoder: ffmpeg_next::decoder::Audio,
    encoder: AacEncoder,
    factor: f32,
    time_base: Rational,
    /// Interleaved stereo samples waiting for a full encoder frame.
    pending: Vec<f32>,
    next_chunk: u64,
    /// Output timestamp (in samples) of the first kept sample.
    start: Option<i64>,
    /// Ranges left out, in samples from the first kept frame.
    cuts: CutMap,
    /// Position of the next decoded sample on that timeline, before the cuts.
    next: Option<i64>,
}

impl GainStage {
    fn new(audio: &AudioSource, factor: f32, cuts: CutMap) -> Result<(Self, StreamInfo)> {
        let context = codec::context::Context::from_parameters(audio.params.clone())?;
        let decoder = context
            .decoder()
            .audio()
            .context("cannot open the audio decoder")?;
        if decoder.rate() != SAMPLE_RATE || decoder.channels() != 2 {
            bail!("changing the volume needs 48 kHz stereo audio, as Umbra records");
        }
        let bit_rate = unsafe { (*audio.params.as_ptr()).bit_rate };
        let kbps = if bit_rate > 0 {
            (bit_rate / 1000) as u32
        } else {
            192
        };
        let title = audio.title.clone().unwrap_or_else(|| "Audio".into());
        let (encoder, info) = AacEncoder::new(kbps.clamp(64, 512), &title)?;
        Ok((
            Self {
                decoder,
                encoder,
                factor: factor.clamp(0.0, 4.0),
                time_base: audio.time_base,
                pending: Vec::with_capacity(CHUNK * 4),
                next_chunk: 0,
                start: None,
                cuts,
                next: None,
            },
            info,
        ))
    }

    fn feed(
        &mut self,
        packet: &Packet,
        pts: i64,
        offset: i64,
        out: usize,
        writer: &mut Mp4Writer,
    ) -> Result<()> {
        if self.start.is_none() {
            let position = rescale(
                pts - offset,
                self.time_base,
                Rational::new(1, SAMPLE_RATE as i32),
            );
            self.next = Some(position);
            self.start = Some(self.cuts.map_floor(position));
        }
        match self.decoder.send_packet(packet) {
            Ok(()) => {}
            Err(AvError::InvalidData) => return Ok(()),
            Err(err) => return Err(err).context("audio decode failed"),
        }
        self.decode_ready()?;
        self.encode_ready(out, writer, false)
    }

    fn decode_ready(&mut self) -> Result<()> {
        let mut frame = frame::Audio::empty();
        loop {
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => {}
                Err(AvError::Other { errno }) if errno == EAGAIN => return Ok(()),
                Err(AvError::Eof) => return Ok(()),
                Err(err) => return Err(err).context("audio decode failed"),
            }
            unsafe {
                let raw = frame.as_ptr();
                let count = (*raw).nb_samples.max(0) as usize;
                if (*raw).format != ffmpeg_next::ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as i32 {
                    bail!("unexpected audio sample format from the decoder");
                }
                let left = std::slice::from_raw_parts((*raw).data[0] as *const f32, count);
                let right = std::slice::from_raw_parts((*raw).data[1] as *const f32, count);
                let first = self.next.unwrap_or(0);
                self.next = Some(first + count as i64);
                for (i, (l, r)) in left.iter().zip(right).enumerate() {
                    if !self.cuts.is_empty() && self.cuts.map(first + i as i64).is_none() {
                        continue;
                    }
                    self.pending.push((l * self.factor).clamp(-1.0, 1.0));
                    self.pending.push((r * self.factor).clamp(-1.0, 1.0));
                }
            }
        }
    }

    fn encode_ready(&mut self, out: usize, writer: &mut Mp4Writer, all: bool) -> Result<()> {
        if all && self.pending.len() % (CHUNK * 2) != 0 {
            // Pad the last partial frame with silence.
            let full = self.pending.len().div_ceil(CHUNK * 2) * CHUNK * 2;
            self.pending.resize(full, 0.0);
        }
        let start = self.start.unwrap_or(0);
        let mut failure = None;
        let mut sink = |mut packet: Packet| {
            packet.set_pts(packet.pts().map(|p| p + start));
            packet.set_dts(packet.dts().map(|d| d + start));
            if let Err(err) = writer.write(out, &packet, 0) {
                failure.get_or_insert(err);
            }
        };
        let mut used = 0;
        while self.pending.len() - used >= CHUNK * 2 {
            self.encoder.encode(
                self.next_chunk,
                &self.pending[used..used + CHUNK * 2],
                &mut sink,
            )?;
            self.next_chunk += 1;
            used += CHUNK * 2;
        }
        if all {
            self.encoder.finish(&mut sink)?;
        }
        self.pending.drain(..used);
        match failure {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn finish(&mut self, out: usize, writer: &mut Mp4Writer) -> Result<()> {
        let _ = self.decoder.send_eof();
        self.decode_ready()?;
        self.encode_ready(out, writer, true)
    }
}

/// A track going into a [`MixStage`].
struct MixPart<'a> {
    audio: &'a AudioSource,
    /// Volume factor; negative takes the track out of the sum.
    gain: f32,
    offset: i64,
    lo: i64,
    hi: i64,
    /// Samples to move the track by before adding it.
    shift: i64,
}

/// One track summed into a rebuilt combined track.
struct MixInput {
    stream_index: usize,
    decoder: ffmpeg_next::decoder::Audio,
    gain: f32,
    time_base: Rational,
    offset: i64,
    lo: i64,
    hi: i64,
    done: bool,
    /// Output sample position (per channel) of the next decoded sample, before `shift`.
    next: Option<i64>,
    /// Samples added to every position of this input.
    shift: i64,
}

/// Rebuilds the combined first track from the tracks it is made of, each at its own
/// volume, so a volume change reaches the track that most players play.
struct MixStage {
    inputs: Vec<MixInput>,
    encoder: AacEncoder,
    out: usize,
    /// Interleaved stereo sums; `mixed[0]` is output sample `base`.
    mixed: Vec<f32>,
    base: Option<i64>,
    next_chunk: u64,
    /// Ranges left out, in samples; `mixed` and `base` are on the joined timeline.
    cuts: CutMap,
}

impl MixStage {
    fn new(
        mix: &AudioSource,
        parts: Vec<MixPart>,
        cuts: CutMap,
        out: usize,
    ) -> Result<(Self, StreamInfo)> {
        let bit_rate = unsafe { (*mix.params.as_ptr()).bit_rate };
        let kbps = if bit_rate > 0 {
            (bit_rate / 1000) as u32
        } else {
            192
        };
        let title = mix.title.clone().unwrap_or_else(|| "Mix".into());
        let (encoder, info) = AacEncoder::new(kbps.clamp(64, 512), &title)?;
        let mut inputs = Vec::new();
        for MixPart {
            audio,
            gain,
            offset,
            lo,
            hi,
            shift,
        } in parts
        {
            let decoder = codec::context::Context::from_parameters(audio.params.clone())?
                .decoder()
                .audio()
                .context("cannot open the audio decoder")?;
            if decoder.rate() != SAMPLE_RATE || decoder.channels() != 2 {
                bail!("rebuilding the mix needs 48 kHz stereo audio, as Umbra records");
            }
            inputs.push(MixInput {
                stream_index: audio.stream_index,
                decoder,
                gain: gain.clamp(-4.0, 4.0),
                time_base: audio.time_base,
                offset,
                lo,
                hi,
                done: false,
                next: None,
                shift,
            });
        }
        Ok((
            Self {
                inputs,
                encoder,
                out,
                mixed: Vec::with_capacity(CHUNK * 8),
                base: None,
                next_chunk: 0,
                cuts,
            },
            info,
        ))
    }

    fn done(&self) -> bool {
        self.inputs.iter().all(|i| i.done)
    }

    /// Takes a packet of any stream; packets of streams outside the mix are ignored.
    fn feed(&mut self, packet: &Packet, writer: &mut Mp4Writer) -> Result<()> {
        let Some(slot) = self
            .inputs
            .iter()
            .position(|i| i.stream_index == packet.stream())
        else {
            return Ok(());
        };
        let input = &mut self.inputs[slot];
        if input.done {
            return Ok(());
        }
        let Some(pts) = packet.pts().or(packet.dts()) else {
            return Ok(());
        };
        if pts < input.lo {
            return Ok(());
        }
        if pts >= input.hi {
            input.done = true;
            let _ = input.decoder.send_eof();
            self.decode(slot)?;
            return self.emit(writer, false);
        }
        if input.next.is_none() {
            input.next = Some(rescale(
                pts - input.offset,
                input.time_base,
                Rational::new(1, SAMPLE_RATE as i32),
            ));
        }
        match input.decoder.send_packet(packet) {
            Ok(()) => {}
            Err(AvError::InvalidData) => return Ok(()),
            Err(err) => return Err(err).context("audio decode failed"),
        }
        self.decode(slot)?;
        self.emit(writer, false)
    }

    /// Adds every frame input `slot` has ready into the sums.
    fn decode(&mut self, slot: usize) -> Result<()> {
        let mut frame = frame::Audio::empty();
        loop {
            let input = &mut self.inputs[slot];
            match input.decoder.receive_frame(&mut frame) {
                Ok(()) => {}
                Err(AvError::Other { errno }) if errno == EAGAIN => return Ok(()),
                Err(AvError::Eof) => return Ok(()),
                Err(err) => return Err(err).context("audio decode failed"),
            }
            let gain = input.gain;
            let raw = input.next.unwrap_or(0);
            let count = unsafe { (*frame.as_ptr()).nb_samples.max(0) as usize };
            input.next = Some(raw + count as i64);
            let start = raw + input.shift;
            let base = *self.base.get_or_insert(self.cuts.map_floor(start));
            // `mixed` begins after the chunks already encoded.
            let first = base + self.next_chunk as i64 * CHUNK as i64;
            unsafe {
                let raw = frame.as_ptr();
                if (*raw).format != ffmpeg_next::ffi::AVSampleFormat::AV_SAMPLE_FMT_FLTP as i32 {
                    bail!("unexpected audio sample format from the decoder");
                }
                let left = std::slice::from_raw_parts((*raw).data[0] as *const f32, count);
                let right = std::slice::from_raw_parts((*raw).data[1] as *const f32, count);
                for (i, (l, r)) in left.iter().zip(right).enumerate() {
                    let Some(position) = self.cuts.map(start + i as i64) else {
                        continue;
                    };
                    // Samples from before that are too late and are dropped.
                    let Ok(at) = usize::try_from(position - first) else {
                        continue;
                    };
                    let at = at * 2;
                    if at + 1 >= self.mixed.len() {
                        self.mixed.resize(at + 2, 0.0);
                    }
                    self.mixed[at] += l * gain;
                    self.mixed[at + 1] += r * gain;
                }
            }
        }
    }

    /// Encodes the chunks every input has reached; with `all`, everything left.
    fn emit(&mut self, writer: &mut Mp4Writer, all: bool) -> Result<()> {
        let Some(base) = self.base else {
            return Ok(());
        };
        let reached = if all {
            i64::MAX
        } else {
            self.inputs
                .iter()
                .filter(|i| !i.done)
                .map(|i| i.next.map_or(base, |n| self.cuts.map_floor(n + i.shift)))
                .min()
                .unwrap_or(i64::MAX)
        };
        if all && self.mixed.len() % (CHUNK * 2) != 0 {
            let full = self.mixed.len().div_ceil(CHUNK * 2) * CHUNK * 2;
            self.mixed.resize(full, 0.0);
        }
        let mut failure = None;
        let out = self.out;
        let mut sink = |mut packet: Packet| {
            packet.set_pts(packet.pts().map(|p| p + base));
            packet.set_dts(packet.dts().map(|d| d + base));
            if let Err(err) = writer.write(out, &packet, 0) {
                failure.get_or_insert(err);
            }
        };
        let mut used = 0;
        while self.mixed.len() - used >= CHUNK * 2 {
            let chunk_end = base + (self.next_chunk as i64 + 1) * CHUNK as i64;
            if chunk_end > reached {
                break;
            }
            let chunk = &mut self.mixed[used..used + CHUNK * 2];
            for sample in chunk.iter_mut() {
                *sample = sample.clamp(-1.0, 1.0);
            }
            self.encoder.encode(self.next_chunk, chunk, &mut sink)?;
            self.next_chunk += 1;
            used += CHUNK * 2;
        }
        if all {
            self.encoder.finish(&mut sink)?;
        }
        self.mixed.drain(..used);
        match failure {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn finish(&mut self, writer: &mut Mp4Writer) -> Result<()> {
        for slot in 0..self.inputs.len() {
            if !self.inputs[slot].done {
                self.inputs[slot].done = true;
                let _ = self.inputs[slot].decoder.send_eof();
                self.decode(slot)?;
            }
        }
        self.emit(writer, true)
    }
}

/// The audio of a clip as separate files, for the editor's preview.
#[derive(Clone, Debug)]
pub struct SplitTracks {
    /// (track index, file) for every audio track.
    pub tracks: Vec<(usize, PathBuf)>,
    /// "Other apps" with the separately recorded apps taken out, when any were found in it,
    /// as (index of "Other apps", file, apps taken out). The preview plays this in place of
    /// "Other apps" and the apps at their own volume, the way the export rebuilds them.
    pub others_without_apps: Option<(usize, PathBuf, Vec<usize>)>,
}

/// Writes each audio track of `input` to its own `.m4a` file in `cache_dir`, copying the
/// packets, plus "Other apps" without the separately recorded apps when there are any.
/// Files are reused while the clip is unchanged. The editor plays these side by side to
/// preview tracks at different volumes.
pub fn split_audio_tracks(input: &Path, cache_dir: &Path) -> Result<SplitTracks> {
    let tracks = split_tracks(input, cache_dir)?;
    let others_without_apps = match others_without_apps(input, &tracks) {
        Ok(found) => found,
        Err(err) => {
            log::warn!("cannot take the apps out of Other apps for the preview: {err:#}");
            None
        }
    };
    Ok(SplitTracks {
        tracks,
        others_without_apps,
    })
}

/// Renders "Other apps" minus the apps found inside it, next to the split track files.
fn others_without_apps(
    input: &Path,
    tracks: &[(usize, PathBuf)],
) -> Result<Option<(usize, PathBuf, Vec<usize>)>> {
    let found = find_apps_in_others(input)?;
    let Some(first) = found.first() else {
        return Ok(None);
    };
    let others = first.others;
    let apps: Vec<usize> = found.iter().map(|f| f.app).collect();
    let Some((_, others_file)) = tracks.iter().find(|(i, _)| *i == others) else {
        return Ok(None);
    };
    let target = others_file.with_file_name(format!(
        "{}-noapps.m4a",
        others_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("others")
    ));
    if std::fs::metadata(&target).is_ok_and(|m| m.len() > 0) {
        return Ok(Some((others, target, apps)));
    }

    let (mut context, source) = open_source(input)?;
    fn whole(audio: &AudioSource, gain: f32, shift: i64) -> MixPart<'_> {
        MixPart {
            audio,
            gain,
            offset: 0,
            lo: i64::MIN,
            hi: i64::MAX,
            shift,
        }
    }
    let mut parts = vec![whole(&source.audio[others], 1.0, 0)];
    parts.extend(
        found
            .iter()
            .map(|f| whole(&source.audio[f.app], -1.0, f.lag)),
    );
    let (mut stage, info) = MixStage::new(&source.audio[others], parts, CutMap::default(), 0)?;
    let part = target.with_extension("part");
    let mut writer = Mp4Writer::create(&part, &[info])?;
    for (_, packet) in context.packets() {
        stage.feed(&packet, &mut writer)?;
    }
    stage.finish(&mut writer)?;
    writer.finish()?;
    std::fs::rename(&part, &target)
        .with_context(|| format!("cannot write {}", target.display()))?;
    Ok(Some((others, target, apps)))
}

/// Names a clip's files in a cache: FNV-1a of its path, modification time and size, so an
/// edited or replaced clip gets new ones.
fn cache_key(input: &Path) -> Result<String> {
    let meta =
        std::fs::metadata(input).with_context(|| format!("cannot read {}", input.display()))?;
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    for byte in input
        .to_string_lossy()
        .to_lowercase()
        .bytes()
        .chain(modified.to_le_bytes())
        .chain(meta.len().to_le_bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    Ok(format!("{hash:016x}"))
}

/// A clip's webcam as a file of its own, for the player to show over the clip.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraTrack {
    pub path: PathBuf,
    /// Where the camera was filming, in seconds of the clip; it was off in between. The file
    /// starts at the first one: its time 0 is the clip's `stretches[0].0`.
    pub stretches: Vec<(f64, f64)>,
}

/// A gap this long between camera frames means the camera was off. A frame shows until the
/// next one, for at most this long; the player and the export both go by it.
pub(crate) const CAMERA_GAP_SECS: f64 = 0.5;

/// Copies the camera stream of a clip into `cache_dir` (once; later calls reuse the file).
/// `None` for a clip without camera.
pub fn split_camera(input: &Path, cache_dir: &Path) -> Result<Option<CameraTrack>> {
    init_ffmpeg()?;
    let mut context =
        format::input(&input).with_context(|| format!("cannot open {}", input.display()))?;
    let Some(stream) = context.streams().find(crate::library::is_camera_stream) else {
        return Ok(None);
    };
    let (index, time_base) = (stream.index(), stream.time_base());
    let info = StreamInfo {
        params: stream.parameters(),
        time_base,
        title: Some(crate::camera::CAMERA_TITLE.to_string()),
    };
    let key = cache_key(input)?;
    let target = cache_dir.join(format!("{key}-camera.mp4"));
    // Named apart from the stretches of before, which bridged every gap.
    let stretches_file = cache_dir.join(format!("{key}-camera-shown.txt"));
    if let (Ok(meta), Ok(text)) = (
        std::fs::metadata(&target),
        std::fs::read_to_string(&stretches_file),
    ) {
        if meta.len() > 0 {
            return Ok(Some(CameraTrack {
                path: target,
                stretches: parse_stretches(&text),
            }));
        }
    }
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("cannot create {}", cache_dir.display()))?;
    let part = target.with_extension("part");
    let mut writer = Mp4Writer::create(&part, &[info])?;
    let seconds = time_base.numerator() as f64 / time_base.denominator() as f64;
    let mut stretches: Vec<(f64, f64)> = Vec::new();
    // Started at zero: whether a player honours a file's start offset varies.
    let mut first: Option<i64> = None;
    for (stream, packet) in context.packets() {
        if stream.index() != index {
            continue;
        }
        let Some(pts) = packet.pts() else {
            continue;
        };
        writer.write(0, &packet, *first.get_or_insert(pts))?;
        // The file gives each frame the time until the next one, gap or not.
        let start = pts as f64 * seconds;
        let end = start + (packet.duration().max(1) as f64 * seconds).min(CAMERA_GAP_SECS);
        match stretches.last_mut() {
            Some(last) if start <= last.1 + 1e-6 => last.1 = last.1.max(end),
            _ => stretches.push((start, end)),
        }
    }
    writer.finish()?;
    let text: String = stretches
        .iter()
        .map(|(a, b)| format!("{a:.3} {b:.3}\n"))
        .collect();
    std::fs::write(&stretches_file, text)
        .with_context(|| format!("cannot write {}", stretches_file.display()))?;
    std::fs::rename(&part, &target)
        .with_context(|| format!("cannot write {}", target.display()))?;
    Ok(Some(CameraTrack {
        path: target,
        stretches,
    }))
}

fn parse_stretches(text: &str) -> Vec<(f64, f64)> {
    text.lines()
        .filter_map(|line| {
            let (a, b) = line.split_once(' ')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect()
}

fn split_tracks(input: &Path, cache_dir: &Path) -> Result<Vec<(usize, PathBuf)>> {
    crate::library::init_ffmpeg()?;
    let (mut context, source) = open_source(input)?;
    let key = cache_key(input)?;
    let targets: Vec<(usize, PathBuf)> = (0..source.audio.len())
        .map(|i| (i, cache_dir.join(format!("{key}-{i}.m4a"))))
        .collect();
    if targets
        .iter()
        .all(|(_, p)| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
    {
        return Ok(targets);
    }
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("cannot create {}", cache_dir.display()))?;

    let mut writers = Vec::new();
    let mut by_stream: Vec<Option<usize>> = vec![None; source.stream_count];
    for (i, audio) in source.audio.iter().enumerate() {
        let part = targets[i].1.with_extension("part");
        let writer = Mp4Writer::create(
            &part,
            &[StreamInfo {
                params: audio.params.clone(),
                time_base: audio.time_base,
                title: audio.title.clone(),
            }],
        )?;
        by_stream[audio.stream_index] = Some(writers.len());
        writers.push((writer, part, targets[i].1.clone()));
    }
    for (_, packet) in context.packets() {
        if let Some(Some(slot)) = by_stream.get(packet.stream()) {
            if packet.pts().is_some() {
                writers[*slot].0.write(0, &packet, 0)?;
            }
        }
    }
    for (writer, part, target) in writers {
        writer.finish()?;
        std::fs::rename(&part, &target)
            .with_context(|| format!("cannot write {}", target.display()))?;
    }
    Ok(targets)
}

// ---------------------------------------------------------------------------------------
// Preparing frames for the encoder
// ---------------------------------------------------------------------------------------

/// Crops decoded frames and converts them to the encoder's pixel format.
struct FramePrep {
    crop: Option<Crop>,
    /// The picture format the encoder takes.
    target: Pixel,
    /// Output size when the picture is scaled.
    scale: Option<(u32, u32)>,
    converter: Option<(scaling::Context, Pixel, u32, u32)>,
    converted: frame::Video,
}

impl FramePrep {
    fn new(crop: Option<Crop>, scale: Option<(u32, u32)>, target: Pixel) -> Self {
        Self {
            crop,
            target,
            scale,
            converter: None,
            converted: frame::Video::empty(),
        }
    }

    /// Crops `decoded` in place (by moving its plane pointers; no pixels are copied) and
    /// returns the frame to encode: `decoded` itself when it is already planar 4:2:0,
    /// otherwise a converted copy.
    fn prepare(&mut self, decoded: &mut frame::Video) -> Result<*mut AVFrame> {
        unsafe {
            let raw = decoded.as_mut_ptr();
            if let Some(crop) = self.crop {
                let (width, height) = ((*raw).width as usize, (*raw).height as usize);
                let (x, y, w, h) = (
                    crop.x as usize,
                    crop.y as usize,
                    crop.width as usize,
                    crop.height as usize,
                );
                if x + w > width || y + h > height {
                    bail!("the video frame changed size to {width}x{height} during the export");
                }
                (*raw).crop_left = x;
                (*raw).crop_top = y;
                (*raw).crop_right = width - x - w;
                (*raw).crop_bottom = height - y - h;
                let code = av_frame_apply_cropping(raw, AV_FRAME_CROP_UNALIGNED as i32);
                if code < 0 {
                    bail!("cannot crop the frame: {}", AvError::from(code));
                }
            }
            if decoded.format() == self.target && self.scale.is_none() {
                return Ok(decoded.as_mut_ptr());
            }
            let (out_w, out_h) = self.scale.unwrap_or((decoded.width(), decoded.height()));
            let (format, width, height) = (decoded.format(), decoded.width(), decoded.height());
            let stale = self
                .converter
                .as_ref()
                .is_none_or(|(_, f, w, h)| (*f, *w, *h) != (format, width, height));
            let flags = if out_w < width {
                scaling::Flags::LANCZOS
            } else {
                scaling::Flags::BICUBIC
            };
            if stale {
                let context =
                    scaling::Context::get(format, width, height, self.target, out_w, out_h, flags)
                        .with_context(|| format!("cannot convert {format:?} video to YUV 4:2:0"))?;
                self.converter = Some((context, format, width, height));
                self.converted = frame::Video::empty();
            }
            let (context, ..) = self.converter.as_mut().unwrap();
            context
                .run(decoded, &mut self.converted)
                .context("cannot convert the frame")?;
            Ok(self.converted.as_mut_ptr())
        }
    }
}

// ---------------------------------------------------------------------------------------
// The video encoder
// ---------------------------------------------------------------------------------------

/// Decoding on the graphics card through D3D11VA.
pub(crate) mod hw_decode {
    use ffmpeg_next::ffi::{
        AVBufferRef, AVCodecContext, AVHWDeviceType, AVPixelFormat, av_buffer_ref, av_buffer_unref,
        av_hwdevice_ctx_create,
    };

    /// Picks the D3D11 surface format when the decoder offers it, else the first (software).
    unsafe extern "C" fn pick(
        _ctx: *mut AVCodecContext,
        formats: *const AVPixelFormat,
    ) -> AVPixelFormat {
        unsafe {
            let mut at = formats;
            while *at != AVPixelFormat::AV_PIX_FMT_NONE {
                if *at == AVPixelFormat::AV_PIX_FMT_D3D11 {
                    return *at;
                }
                at = at.add(1);
            }
            *formats
        }
    }

    /// Sets up `ctx` (not yet opened) to decode on the graphics card. False when it cannot,
    /// and the decoder stays a software one.
    pub(crate) unsafe fn attach(ctx: *mut AVCodecContext) -> bool {
        unsafe {
            let mut device: *mut AVBufferRef = std::ptr::null_mut();
            let code = av_hwdevice_ctx_create(
                &mut device,
                AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            );
            if code < 0 || device.is_null() {
                return false;
            }
            (*ctx).hw_device_ctx = av_buffer_ref(device);
            av_buffer_unref(&mut device);
            (*ctx).get_format = Some(pick);
            !(*ctx).hw_device_ctx.is_null()
        }
    }

    /// `decoded` in memory: copied from the graphics card into `scratch` when it is there.
    pub(crate) fn download<'a>(
        decoded: &'a ffmpeg_next::frame::Video,
        scratch: &'a mut ffmpeg_next::frame::Video,
    ) -> anyhow::Result<&'a ffmpeg_next::frame::Video> {
        use ffmpeg_next::ffi::{av_frame_copy_props, av_frame_unref, av_hwframe_transfer_data};
        unsafe {
            if (*decoded.as_ptr()).format != AVPixelFormat::AV_PIX_FMT_D3D11 as i32 {
                return Ok(decoded);
            }
            av_frame_unref(scratch.as_mut_ptr());
            let code = av_hwframe_transfer_data(scratch.as_mut_ptr(), decoded.as_ptr(), 0);
            if code < 0 {
                anyhow::bail!(
                    "cannot copy a decoded frame from the graphics card: {}",
                    ffmpeg_next::Error::from(code)
                );
            }
            av_frame_copy_props(scratch.as_mut_ptr(), decoded.as_ptr());
            Ok(scratch)
        }
    }
}

/// Hardware encoders first, then software ones, per codec.
fn encoder_candidates(codec_id: AVCodecID) -> Option<&'static [&'static str]> {
    match codec_id {
        AVCodecID::AV_CODEC_ID_H264 => Some(&["h264_nvenc", "h264_amf", "h264_qsv", "libopenh264"]),
        AVCodecID::AV_CODEC_ID_HEVC => Some(&["hevc_nvenc", "hevc_amf", "hevc_qsv", "libkvazaar"]),
        AVCodecID::AV_CODEC_ID_AV1 => Some(&["av1_nvenc", "av1_amf", "av1_qsv", "libsvtav1"]),
        _ => None,
    }
}

fn check(code: i32, what: &str) -> Result<i32> {
    if code < 0 {
        Err(anyhow!("{what}: {}", AvError::from(code)))
    } else {
        Ok(code)
    }
}

struct ExportEncoder {
    ctx: *mut AVCodecContext,
    name: String,
    /// The picture format it takes.
    pix: Pixel,
}

/// What the export encodes to, beyond size and bitrate.
#[derive(Clone, Copy)]
struct EncodeShape {
    codec: AVCodecID,
    /// Frames per second when fewer than the source's.
    fps: Option<u32>,
    /// The bitrate is a limit to keep to (a file size), not a guide.
    capped: bool,
    /// Frames arrive as NV12: a hardware encoder is fed that, skipping a conversion.
    nv12: bool,
    /// A faster encoder setting (`ExportJob::quick`).
    quick: bool,
}

fn codec_id_of(codec: crate::encode::VideoCodec) -> AVCodecID {
    match codec {
        crate::encode::VideoCodec::H264 => AVCodecID::AV_CODEC_ID_H264,
        crate::encode::VideoCodec::Hevc => AVCodecID::AV_CODEC_ID_HEVC,
        crate::encode::VideoCodec::Av1 => AVCodecID::AV_CODEC_ID_AV1,
    }
}

impl ExportEncoder {
    fn open(
        source: &Source,
        forced: Option<&str>,
        width: u32,
        height: u32,
        bit_rate: i64,
        shape: EncodeShape,
    ) -> Result<Self> {
        let candidates: Vec<&str> = match forced {
            Some(name) => vec![name],
            None => encoder_candidates(shape.codec)
                .ok_or_else(|| {
                    anyhow!("re-encoding this video format is not supported; only H.264, HEVC and AV1 are")
                })?
                .to_vec(),
        };
        let mut failures = Vec::new();
        for name in candidates {
            match unsafe { Self::try_open(name, source, width, height, bit_rate, shape) } {
                Ok(encoder) => return Ok(encoder),
                Err(err) => {
                    log::info!("video encoder {name} cannot be used: {err:#}");
                    failures.push(format!("{name}: {err:#}"));
                }
            }
        }
        bail!("no video encoder could be opened ({})", failures.join("; "))
    }

    unsafe fn try_open(
        name: &str,
        source: &Source,
        width: u32,
        height: u32,
        bit_rate: i64,
        shape: EncodeShape,
    ) -> Result<Self> {
        unsafe {
            let c_name = std::ffi::CString::new(name)?;
            let codec = avcodec_find_encoder_by_name(c_name.as_ptr());
            if codec.is_null() {
                bail!("this FFmpeg build has no such encoder");
            }
            if (*codec).id != shape.codec {
                bail!("it does not produce the codec asked for");
            }
            let raw = avcodec_alloc_context3(codec);
            if raw.is_null() {
                bail!("avcodec_alloc_context3 failed");
            }
            // From here on Drop frees the context, including when opening fails.
            let hardware = ["_nvenc", "_amf", "_qsv"].iter().any(|h| name.ends_with(h));
            let pix = if shape.nv12 && hardware {
                Pixel::NV12
            } else {
                Pixel::YUV420P
            };
            let this = Self {
                ctx: raw,
                name: name.to_string(),
                pix,
            };
            let ctx = raw;

            (*ctx).width = width as i32;
            (*ctx).height = height as i32;
            (*ctx).pix_fmt = pix.into();
            (*ctx).time_base = AVRational {
                num: source.time_base.numerator(),
                den: source.time_base.denominator(),
            };
            (*ctx).framerate = match shape.fps {
                Some(fps) => AVRational {
                    num: fps as i32,
                    den: 1,
                },
                None => AVRational {
                    num: source.fps.numerator(),
                    den: source.fps.denominator(),
                },
            };
            // One keyframe per second like the recorder, so the export can be cut losslessly again.
            let fps = match shape.fps {
                Some(fps) => fps as f64,
                None => source.fps.numerator() as f64 / source.fps.denominator() as f64,
            };
            (*ctx).gop_size = (fps.round() as i32).max(1);
            (*ctx).max_b_frames = 0;
            (*ctx).bit_rate = bit_rate;
            (*ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            let par = source.params.as_ptr();
            (*ctx).sample_aspect_ratio = (*par).sample_aspect_ratio;
            (*ctx).color_range = if (*par).color_range == AVColorRange::AVCOL_RANGE_UNSPECIFIED {
                AVColorRange::AVCOL_RANGE_MPEG
            } else {
                (*par).color_range
            };
            if (*par).color_space != AVColorSpace::AVCOL_SPC_UNSPECIFIED {
                (*ctx).colorspace = (*par).color_space;
            }
            if (*par).color_primaries != AVColorPrimaries::AVCOL_PRI_UNSPECIFIED {
                (*ctx).color_primaries = (*par).color_primaries;
            }
            if (*par).color_trc != AVColorTransferCharacteristic::AVCOL_TRC_UNSPECIFIED {
                (*ctx).color_trc = (*par).color_trc;
            }

            if name.ends_with("_nvenc") {
                // P3 encodes 1440p about 2.3 times as fast as P4 on an RTX 50, a few percent
                // larger for the same look.
                set_opt(ctx, "preset", if shape.quick { "p3" } else { "p4" });
                set_opt(ctx, "tune", "hq");
                set_opt(ctx, "rc", "vbr");
                set_opt(ctx, "forced-idr", "1");
                if shape.capped {
                    // A size limit: never above the bitrate, and a first pass so the bits
                    // that are there go where they show.
                    set_opt(ctx, "multipass", "qres");
                    set_opt(ctx, "spatial-aq", "1");
                    (*ctx).rc_max_rate = bit_rate;
                    (*ctx).rc_buffer_size = (bit_rate * 2).min(i32::MAX as i64) as i32;
                } else {
                    (*ctx).rc_max_rate = bit_rate * 3 / 2;
                }
            } else if name.ends_with("_amf") && shape.quick {
                set_opt(ctx, "quality", "speed");
            } else if name.ends_with("_qsv") && shape.quick {
                set_opt(ctx, "preset", "faster");
            } else if name == "libopenh264" {
                set_opt(ctx, "rc_mode", "bitrate");
                set_opt(ctx, "allow_skip_frames", "0");
                set_opt(ctx, "profile", "high");
            } else if name == "libsvtav1" {
                set_opt(ctx, "preset", "8");
            } else if name == "libkvazaar" {
                set_opt(ctx, "preset", "fast");
            }

            check(
                avcodec_open2(ctx, codec, ptr::null_mut()),
                "cannot open the encoder",
            )?;
            Ok(this)
        }
    }

    fn stream_info(&self) -> Result<StreamInfo> {
        let mut params = codec::Parameters::new();
        unsafe {
            check(
                avcodec_parameters_from_context(params.as_mut_ptr(), self.ctx),
                "avcodec_parameters_from_context",
            )?;
            let time_base = Rational::new((*self.ctx).time_base.num, (*self.ctx).time_base.den);
            Ok(StreamInfo {
                params,
                time_base,
                title: None,
            })
        }
    }

    /// Encodes one frame (`frame` null flushes) and hands finished packets to `sink`.
    fn send(
        &mut self,
        frame: *const AVFrame,
        frame_ticks: i64,
        sink: &mut dyn FnMut(Packet) -> Result<()>,
    ) -> Result<()> {
        unsafe {
            loop {
                let code = avcodec_send_frame(self.ctx, frame);
                if code == AVERROR_EAGAIN {
                    // The output queue is full; make room and try again.
                    self.drain(frame_ticks, sink)?;
                    continue;
                }
                if code == AVERROR_EOF && frame.is_null() {
                    break;
                }
                check(code, "avcodec_send_frame")?;
                break;
            }
            self.drain(frame_ticks, sink)
        }
    }

    fn flush(
        &mut self,
        frame_ticks: i64,
        sink: &mut dyn FnMut(Packet) -> Result<()>,
    ) -> Result<()> {
        self.send(ptr::null(), frame_ticks, sink)
    }

    unsafe fn drain(
        &mut self,
        frame_ticks: i64,
        sink: &mut dyn FnMut(Packet) -> Result<()>,
    ) -> Result<()> {
        unsafe {
            loop {
                let mut packet = Packet::empty();
                let code = avcodec_receive_packet(self.ctx, packet.as_mut_ptr());
                if code == AVERROR_EAGAIN || code == AVERROR_EOF {
                    return Ok(());
                }
                check(code, "avcodec_receive_packet")?;
                if packet.duration() == 0 {
                    packet.set_duration(frame_ticks);
                }
                sink(packet)?;
            }
        }
    }
}

impl Drop for ExportEncoder {
    fn drop(&mut self) {
        unsafe { avcodec_free_context(&mut self.ctx) };
    }
}

/// Sets an encoder-specific option; encoders that do not know it simply ignore it.
unsafe fn set_opt(ctx: *mut AVCodecContext, key: &str, value: &str) {
    unsafe {
        if (*ctx).priv_data.is_null() {
            return;
        }
        let (Ok(k), Ok(v)) = (std::ffi::CString::new(key), std::ffi::CString::new(value)) else {
            return;
        };
        if av_opt_set((*ctx).priv_data, k.as_ptr(), v.as_ptr(), 0) < 0 {
            log::debug!("encoder ignored option {key}={value}");
        }
    }
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::probe;
    use crate::library::testutil::{TempDir, make_clip};

    /// Only the combined track, with the microphone (normally only on its own) mixed in.
    #[test]
    fn mix_only_keeps_the_voice_in_the_one_track() {
        let dir = std::env::temp_dir().join("umbra-export-mix-only");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.mp4");
        make_mixed_clip(&input, 3);
        // Mark the clip as Umbra does: Mix holds Game only, the microphone is on its own.
        let tagged = dir.join("tagged.mp4");
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-i"])
            .arg(&input)
            .args(["-map", "0", "-c", "copy", "-movflags", "+use_metadata_tags"])
            .args(["-metadata", "umbra_mix=Game"])
            .arg(&tagged)
            .status()
            .unwrap();
        assert!(status.success());

        let output = dir.join("out.mp4");
        let mut job = ExportJob::new(&tagged, &output, 0.0, 3.0);
        job.audio_tracks = vec![0, 1, 2];
        job.mix_only = true;
        export(&job, &mut |_| true).unwrap();

        let out = format::input(&output).unwrap();
        let audio = out
            .streams()
            .filter(|s| s.parameters().medium() == media::Type::Audio)
            .count();
        assert_eq!(audio, 1, "only the combined track is written");
        // The 880 Hz microphone tone is in it, at about the level it was recorded.
        let mixed = tone_level(&output, 0, 880.0);
        let mic = tone_level(&tagged, 2, 880.0);
        assert!(
            mixed > 0.5 * mic,
            "microphone tone {mixed} in the export, {mic} recorded"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A preset-style export: another codec, a smaller picture, fewer frames and a bitrate
    /// cap, all at once.
    #[test]
    fn exports_to_another_codec_size_frame_rate_and_bitrate() {
        let dir = std::env::temp_dir().join("umbra-export-shape");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.mp4");
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=1280x720:rate=60:duration=6")
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=6:sample_rate=48000",
            ])
            .args(["-c:v", "libopenh264", "-b:v", "8M", "-g", "60", "-bf", "0"])
            .args(["-c:a", "aac", "-b:a", "128k", "-ac", "2"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());

        let output = dir.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 6.0);
        job.audio_tracks = vec![0];
        job.codec = Some(crate::encode::VideoCodec::Hevc);
        job.scale = Some((640, 360));
        job.max_fps = Some(30);
        job.video_bitrate = Some(1_500_000);
        job.mix_only = true;
        let report = export_with_report(&job, &mut |_| true).unwrap();
        assert_eq!(report.plan.mode, ExportMode::Reencode);

        let out = format::input(&output).unwrap();
        let video = out.streams().best(media::Type::Video).unwrap();
        let params = video.parameters();
        assert_eq!(params.id(), codec::Id::HEVC);
        let (width, height) = unsafe { ((*params.as_ptr()).width, (*params.as_ptr()).height) };
        assert_eq!((width, height), (640, 360));
        let frames = video.frames();
        assert!(
            (170..=185).contains(&frames),
            "{frames} frames for 6 s at 30 fps"
        );
        // 1.5 Mbit/s of video and 128 kbit/s of audio for 6 s, with a little room.
        let bytes = fs::metadata(&output).unwrap().len();
        assert!(bytes < 1_400_000, "{bytes} bytes");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crop_is_rounded_and_checked() {
        let crop = |x, y, width, height| Crop {
            x,
            y,
            width,
            height,
        };
        // Odd values round down to even.
        assert_eq!(
            normalize_crop(crop(101, 51, 301, 201), 640, 360).unwrap(),
            Some(crop(100, 50, 300, 200))
        );
        // A rectangle sticking out of the frame is pulled back in.
        assert_eq!(
            normalize_crop(crop(600, 300, 500, 500), 640, 360).unwrap(),
            Some(crop(600, 300, 40, 60))
        );
        // The whole frame is no crop at all.
        assert_eq!(
            normalize_crop(crop(0, 0, 640, 360), 640, 360).unwrap(),
            None
        );
        assert_eq!(
            normalize_crop(crop(0, 0, 9999, 9999), 640, 360).unwrap(),
            None
        );
        assert_eq!(
            normalize_crop(crop(1, 1, 639, 359), 640, 360).unwrap(),
            Some(crop(0, 0, 638, 358))
        );
        // Nothing left, outside, or too small is an error.
        assert!(normalize_crop(crop(0, 0, 0, 100), 640, 360).is_err());
        assert!(normalize_crop(crop(640, 0, 100, 100), 640, 360).is_err());
        assert!(normalize_crop(crop(0, 0, 8, 8), 640, 360).is_err());
        assert!(normalize_crop(crop(630, 0, 100, 100), 640, 360).is_err());
    }

    /// Frame timestamps of the video stream, in seconds, and how many frames decode.
    struct VideoCheck {
        first: f64,
        last: f64,
        packets: usize,
        decoded: usize,
        width: u32,
        height: u32,
    }

    fn check_video(path: &Path) -> VideoCheck {
        let mut input = format::input(path).unwrap();
        let (index, tb, params) = {
            let stream = input.streams().best(media::Type::Video).unwrap();
            (stream.index(), stream.time_base(), stream.parameters())
        };
        let mut decoder = codec::context::Context::from_parameters(params)
            .unwrap()
            .decoder()
            .video()
            .unwrap();
        let (width, height) = (decoder.width(), decoder.height());
        let mut times = Vec::new();
        let mut decoded = 0;
        let mut frame = frame::Video::empty();
        let mut drain = |decoder: &mut ffmpeg_next::decoder::Video| {
            while decoder.receive_frame(&mut frame).is_ok() {
                decoded += 1;
            }
        };
        for (stream, packet) in input.packets() {
            if stream.index() == index {
                times.push(
                    packet.pts().unwrap() as f64 * tb.numerator() as f64 / tb.denominator() as f64,
                );
                decoder.send_packet(&packet).expect("packet decodes");
                drain(&mut decoder);
            }
        }
        decoder.send_eof().unwrap();
        drain(&mut decoder);
        VideoCheck {
            first: times.iter().cloned().fold(f64::MAX, f64::min),
            last: times.iter().cloned().fold(f64::MIN, f64::max),
            packets: times.len(),
            decoded,
            width,
            height,
        }
    }

    fn titles(path: &Path) -> Vec<String> {
        probe(path)
            .unwrap()
            .audio_tracks
            .into_iter()
            .map(|t| t.title)
            .collect()
    }

    fn run(job: &ExportJob) -> ExportReport {
        let mut last = 0.0;
        let report = export_with_report(job, &mut |p| {
            assert!(p >= last, "progress went backwards: {p} after {last}");
            last = p;
            true
        })
        .unwrap();
        assert!(last > 0.99, "progress ended at {last}");
        let leftover = {
            let mut part = job.output.clone().into_os_string();
            part.push(".part");
            PathBuf::from(part)
        };
        assert!(!leftover.exists(), "the .part file is left behind");
        report
    }

    fn setup(name: &str, secs: u32) -> (TempDir, PathBuf) {
        let dir = TempDir::new(name);
        let clip = dir.0.join("clip.mp4");
        make_clip(&clip, secs);
        (dir, clip)
    }

    #[test]
    fn a_cut_on_a_keyframe_is_copied() {
        let (dir, clip) = setup("copy", 6);
        let out = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&clip, &out, 1.0, 4.0);
        job.audio_tracks = vec![0, 1];
        let report = run(&job);
        assert_eq!(
            report.plan.mode,
            ExportMode::StreamCopy,
            "{:?}",
            report.plan
        );
        assert_eq!(report.encoder, None);

        let video = check_video(&out);
        // 3 s at 60 fps, exactly, starting at zero.
        assert_eq!(video.packets, 180);
        assert_eq!(video.decoded, 180);
        assert!(video.first.abs() < 1e-6, "starts at {}", video.first);
        assert!(
            (video.last - (3.0 - 1.0 / 60.0)).abs() < 1e-3,
            "last {}",
            video.last
        );
        assert_eq!((video.width, video.height), (640, 360));
        let meta = probe(&out).unwrap();
        assert!(
            (meta.duration_secs - 3.0).abs() < 0.05,
            "duration {}",
            meta.duration_secs
        );
        assert_eq!(titles(&out), ["Game", "Discord"]);
        assert_eq!(meta.codec, "h264");
        // The packets are the source's: the size is about the same share of the source.
        assert!(meta.size_bytes < probe(&clip).unwrap().size_bytes);
    }

    #[test]
    fn tracks_can_be_dropped_and_reordered_out() {
        let (dir, clip) = setup("tracks", 4);
        let mut job = ExportJob::new(&clip, dir.0.join("one.mp4"), 0.0, 2.0);
        job.audio_tracks = vec![1];
        run(&job);
        assert_eq!(titles(&job.output), ["Discord"]);

        job.output = dir.0.join("none.mp4");
        job.audio_tracks = vec![];
        run(&job);
        assert!(titles(&job.output).is_empty());
        assert_eq!(check_video(&job.output).packets, 120);

        job.output = dir.0.join("dup.mp4");
        job.audio_tracks = vec![1, 0, 1];
        run(&job);
        assert_eq!(
            titles(&job.output),
            ["Game", "Discord"],
            "sorted, no repeats"
        );

        job.output = dir.0.join("bad.mp4");
        job.audio_tracks = vec![2];
        let err = export(&job, &mut |_| true).unwrap_err();
        assert!(err.to_string().contains("no audio track 2"), "{err:#}");
        assert!(!job.output.exists());
    }

    #[test]
    fn a_cut_off_a_keyframe_is_reencoded() {
        let (dir, clip) = setup("reencode", 6);
        for encoder in [None, Some("libopenh264".to_string())] {
            let out = dir.0.join("out.mp4");
            let mut job = ExportJob::new(&clip, &out, 1.5, 4.5);
            job.audio_tracks = vec![0, 1];
            job.encoder = encoder;
            let report = run(&job);
            assert_eq!(report.plan.mode, ExportMode::Reencode);
            assert!(report.plan.reason.as_deref().unwrap().contains("keyframe"));
            assert!(report.video_bitrate_kbps.unwrap() >= 8000);
            eprintln!("re-encoded with {:?}", report.encoder);

            let video = check_video(&out);
            assert_eq!(video.packets, 180);
            assert_eq!(video.decoded, 180);
            assert!(video.first.abs() < 1e-6);
            assert_eq!((video.width, video.height), (640, 360));
            let meta = probe(&out).unwrap();
            assert!(
                (meta.duration_secs - 3.0).abs() < 0.05,
                "duration {}",
                meta.duration_secs
            );
            assert_eq!(meta.codec, "h264");
            assert_eq!(titles(&out), ["Game", "Discord"]);
            assert!((meta.fps - 60.0).abs() < 0.01);
            std::fs::remove_file(&out).unwrap();
        }
    }

    #[test]
    fn a_crop_is_reencoded_to_the_cropped_size() {
        let (dir, clip) = setup("crop", 4);
        let out = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&clip, &out, 0.0, 2.0);
        job.crop = Some(Crop {
            x: 101,
            y: 51,
            width: 301,
            height: 201,
        });
        job.audio_tracks = vec![0];
        job.encoder = Some("libopenh264".into());
        let report = run(&job);
        assert_eq!(report.plan.mode, ExportMode::Reencode);
        assert_eq!(
            report.plan.crop,
            Some(Crop {
                x: 100,
                y: 50,
                width: 300,
                height: 200
            })
        );
        let video = check_video(&out);
        assert_eq!((video.width, video.height), (300, 200));
        assert_eq!(video.packets, 120);
        assert_eq!(video.decoded, 120);
        let meta = probe(&out).unwrap();
        assert_eq!((meta.width, meta.height), (300, 200));
        assert_eq!(titles(&out), ["Game"]);

        // A crop that covers the frame is no crop: the cut is copied.
        job.crop = Some(Crop {
            x: 0,
            y: 0,
            width: 640,
            height: 360,
        });
        job.output = dir.0.join("full.mp4");
        assert_eq!(run(&job).plan.mode, ExportMode::StreamCopy);
    }

    #[test]
    fn times_are_clamped_and_bad_ranges_fail() {
        let (dir, clip) = setup("clamp", 3);
        // Past the end and before the start: the whole clip.
        let mut job = ExportJob::new(&clip, dir.0.join("all.mp4"), -5.0, 99.0);
        job.audio_tracks = vec![0];
        let report = run(&job);
        assert_eq!(report.plan.mode, ExportMode::StreamCopy);
        assert_eq!(check_video(&job.output).packets, 180);

        let fails = |start: f64, end: f64| {
            let job = ExportJob::new(&clip, dir.0.join("never.mp4"), start, end);
            let err = export(&job, &mut |_| true).unwrap_err();
            assert!(!dir.0.join("never.mp4").exists());
            format!("{err:#}")
        };
        assert!(fails(2.0, 1.0).contains("after the start"));
        assert!(fails(1.0, 1.0).contains("after the start"));
        assert!(fails(10.0, 20.0).contains("past the end"));
        assert!(fails(f64::NAN, 2.0).contains("numbers"));
        assert!(fails(1.0, 1.0 + 0.001).contains("shorter than one frame"));

        // The output must not be the input, however it is spelled.
        let job = ExportJob::new(&clip, &clip, 0.0, 1.0);
        assert!(
            export(&job, &mut |_| true)
                .unwrap_err()
                .to_string()
                .contains("different file")
        );
        let spelled = dir.0.join(".").join("CLIP.MP4");
        let job = ExportJob::new(&clip, spelled, 0.0, 1.0);
        assert!(
            export(&job, &mut |_| true)
                .unwrap_err()
                .to_string()
                .contains("different file")
        );
    }

    #[test]
    fn cancelling_leaves_no_output() {
        let (dir, clip) = setup("cancel", 6);
        // Both paths, and both early and late.
        for (start, reencode) in [(0.0, false), (0.5, false), (0.0, true)] {
            let out = dir.0.join("out.mp4");
            let mut job = ExportJob::new(&clip, &out, start, 6.0);
            job.audio_tracks = vec![0, 1];
            job.force_reencode = reencode;
            job.encoder = Some("libopenh264".into());
            let mut calls = 0;
            let err = export(&job, &mut |p| {
                calls += 1;
                p < 0.3
            })
            .unwrap_err();
            assert!(is_cancelled(&err), "{err:#}");
            assert!(calls >= 2, "cancelled after {calls} calls");
            assert!(!out.exists());
            let mut part = out.clone().into_os_string();
            part.push(".part");
            assert!(!PathBuf::from(part).exists());
        }
        // Cancelling on the very first call.
        let job = ExportJob::new(&clip, dir.0.join("first.mp4"), 0.0, 1.0);
        assert!(is_cancelled(&export(&job, &mut |_| false).unwrap_err()));
        assert!(!dir.0.join("first.mp4").exists());
    }

    #[test]
    fn video_with_b_frames_is_never_copied() {
        // Only NVENC among the encoders here makes B-frames, so this needs an NVIDIA GPU.
        let dir = TempDir::new("bframes");
        let clip = dir.0.join("b.mp4");
        let made = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=60:duration=4",
            ])
            .args([
                "-c:v",
                "h264_nvenc",
                "-bf",
                "2",
                "-g",
                "60",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&clip)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !made {
            eprintln!("no NVENC here; skipping the B-frame test");
            return;
        }
        // Start on a keyframe: copying looks possible, but the frames are reordered.
        let out = dir.0.join("out.mp4");
        let report = run(&ExportJob::new(&clip, &out, 1.0, 3.0));
        assert_eq!(report.plan.mode, ExportMode::Reencode, "{:?}", report.plan);
        assert!(report.plan.reason.as_deref().unwrap().contains("B-frames"));
        let video = check_video(&out);
        assert_eq!((video.packets, video.decoded), (120, 120));
        assert!(video.first.abs() < 1e-6);
    }

    #[test]
    fn plan_says_what_would_happen() {
        let (_dir, clip) = setup("plan", 4);
        let mut job = ExportJob::new(&clip, "unused.mp4", 1.0, 3.0);
        let p = plan(&job).unwrap();
        assert_eq!(p.mode, ExportMode::StreamCopy);
        assert!((p.start_secs - 1.0).abs() < 1e-6 && (p.end_secs - 3.0).abs() < 1e-6);
        // Half a frame either side of a keyframe still counts as on it.
        job.start_secs = 1.0 + 1.0 / 130.0;
        assert_eq!(plan(&job).unwrap().mode, ExportMode::StreamCopy);
        job.start_secs = 1.0 - 1.0 / 130.0;
        let p = plan(&job).unwrap();
        assert_eq!(p.mode, ExportMode::StreamCopy);
        assert!((p.start_secs - 1.0).abs() < 1e-6);
        // More than half a frame away is not.
        job.start_secs = 1.0 + 1.0 / 100.0;
        assert_eq!(plan(&job).unwrap().mode, ExportMode::Reencode);
        job.start_secs = 1.0 - 1.0 / 100.0;
        assert_eq!(plan(&job).unwrap().mode, ExportMode::Reencode);
    }

    /// A clip laid out like the recorder's: `Mix` (440 Hz + 880 Hz), `Game` (440 Hz) and
    /// `Microphone` (880 Hz), 48 kHz stereo.
    fn make_mixed_clip(path: &Path, secs: u32) {
        let d = secs.to_string();
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size=320x180:rate=30:duration={d}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={d}:sample_rate=48000"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=880:duration={d}:sample_rate=48000"))
            .args([
                "-filter_complex",
                "[1:a]aformat=channel_layouts=stereo,asplit[g1][g2];[2:a]aformat=channel_layouts=stereo,asplit[m1][m2];[g1][m1]amix=inputs=2:normalize=0[mix]",
            ])
            .args(["-map", "0:v", "-map", "[mix]", "-map", "[g2]", "-map", "[m2]"])
            .args(["-c:v", "libopenh264", "-b:v", "1M", "-g", "30", "-bf", "0"])
            .args(["-c:a", "aac", "-b:a", "128k", "-ar", "48000"])
            .args(["-metadata:s:a:0", "title=Mix", "-metadata:s:a:0", "handler_name=Mix"])
            .args(["-metadata:s:a:1", "title=Game", "-metadata:s:a:1", "handler_name=Game"])
            .args([
                "-metadata:s:a:2",
                "title=Microphone",
                "-metadata:s:a:2",
                "handler_name=Microphone",
            ])
            .arg(path)
            .status()
            .expect("cannot run ffmpeg.exe");
        assert!(status.success());
    }

    /// The left channel of audio track `track`, decoded.
    fn decode_track(path: &Path, track: usize) -> Vec<f32> {
        let mut input = format::input(path).unwrap();
        let stream = input
            .streams()
            .filter(|s| s.parameters().medium() == media::Type::Audio)
            .nth(track)
            .unwrap();
        let index = stream.index();
        let mut decoder = codec::context::Context::from_parameters(stream.parameters())
            .unwrap()
            .decoder()
            .audio()
            .unwrap();
        let mut samples = Vec::new();
        let mut frame = frame::Audio::empty();
        let mut drain = |decoder: &mut ffmpeg_next::decoder::Audio, samples: &mut Vec<f32>| {
            while decoder.receive_frame(&mut frame).is_ok() {
                let n = frame.samples();
                samples.extend_from_slice(&frame.plane::<f32>(0)[..n]);
            }
        };
        for (s, packet) in input.packets() {
            if s.index() == index {
                decoder.send_packet(&packet).unwrap();
                drain(&mut decoder, &mut samples);
            }
        }
        decoder.send_eof().unwrap();
        drain(&mut decoder, &mut samples);
        samples
    }

    /// Strength of `freq` in `samples`, by Goertzel.
    fn goertzel(samples: &[f32], freq: f64) -> f64 {
        let k = 2.0 * (2.0 * std::f64::consts::PI * freq / SAMPLE_RATE as f64).cos();
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for &x in samples {
            let c = x as f64 + k * a - b;
            b = a;
            a = c;
        }
        (a * a + b * b - k * a * b).sqrt() / samples.len().max(1) as f64
    }

    /// Strength of `freq` in the left channel of audio track `track`.
    fn tone_level(path: &Path, track: usize, freq: f64) -> f64 {
        goertzel(&decode_track(path, track), freq)
    }

    #[test]
    fn leaving_out_a_track_rebuilds_the_mix_without_it() {
        let dir = TempDir::new("remix");
        let input = dir.0.join("in.mp4");
        make_mixed_clip(&input, 3);
        let meta = probe(&input).unwrap();
        assert_eq!(
            meta.audio_tracks.iter().map(|t| t.role).collect::<Vec<_>>(),
            vec![MixRole::Mix, MixRole::InMix, MixRole::InMix]
        );

        let before = (tone_level(&input, 0, 440.0), tone_level(&input, 0, 880.0));

        // Game left out, microphone at half volume.
        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 3.0);
        job.audio_tracks = vec![0, 2];
        job.audio_gains = vec![(2, 0.5)];
        export(&job, &mut |_| true).unwrap();

        let out = probe(&output).unwrap();
        assert_eq!(
            out.audio_tracks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Mix", "Microphone"]
        );
        let game = tone_level(&output, 0, 440.0);
        let mic = tone_level(&output, 0, 880.0);
        assert!(
            game < before.0 * 0.01,
            "game still in the mix: {game} vs {}",
            before.0
        );
        let ratio = mic / before.1;
        assert!(
            (0.4..0.6).contains(&ratio),
            "microphone level ratio {ratio}"
        );
        // The mix lasts as long as the clip.
        assert!(
            (out.duration_secs - 3.0).abs() < 0.1,
            "{}",
            out.duration_secs
        );
    }

    #[test]
    fn untouched_mix_is_copied() {
        let dir = TempDir::new("remix-copy");
        let input = dir.0.join("in.mp4");
        make_mixed_clip(&input, 2);
        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 2.0);
        job.audio_tracks = vec![0, 1, 2];
        export(&job, &mut |_| true).unwrap();
        let ratio = tone_level(&output, 0, 440.0) / tone_level(&input, 0, 440.0);
        assert!((ratio - 1.0).abs() < 0.02, "{ratio}");
    }

    #[test]
    fn a_trimmed_cut_rebuilds_the_mix_for_the_kept_range() {
        let dir = TempDir::new("remix-trim");
        let input = dir.0.join("in.mp4");
        make_mixed_clip(&input, 4);
        let output = dir.0.join("out.mp4");
        // Starts off a keyframe, so the video is re-encoded too.
        let mut job = ExportJob::new(&input, &output, 1.3, 3.0);
        job.audio_tracks = vec![0, 1, 2];
        job.audio_gains = vec![(1, 0.0)];
        export(&job, &mut |_| true).unwrap();
        let out = probe(&output).unwrap();
        assert!(
            (out.duration_secs - 1.7).abs() < 0.1,
            "{}",
            out.duration_secs
        );
        assert!(tone_level(&output, 0, 440.0) < tone_level(&input, 0, 440.0) * 0.01);
        assert!(tone_level(&output, 0, 880.0) > tone_level(&input, 0, 880.0) * 0.8);
    }

    /// One `Game` track playing 440 Hz, except 880 Hz from 1 s to 2 s; 30 fps video.
    fn make_marked_clip(path: &Path) {
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x180:rate=30:duration=4")
            .args(["-f", "lavfi", "-i"])
            .arg(r"aevalsrc=sin(2*PI*if(between(t\,1\,2)\,880\,440)*t)|sin(2*PI*if(between(t\,1\,2)\,880\,440)*t):s=48000:d=4")
            .args(["-map", "0:v", "-map", "1:a"])
            .args(["-c:v", "libopenh264", "-b:v", "1M", "-g", "30", "-bf", "0"])
            .args(["-c:a", "aac", "-b:a", "128k"])
            .args(["-metadata:s:a:0", "title=Game"])
            .arg(path)
            .status()
            .expect("cannot run ffmpeg.exe");
        assert!(status.success());
    }

    #[test]
    fn a_cut_range_is_left_out_of_video_and_audio() {
        let dir = TempDir::new("cut");
        let input = dir.0.join("in.mp4");
        make_marked_clip(&input);
        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 4.0);
        job.audio_tracks = vec![0];
        job.cuts = vec![(1.0, 2.0)];
        let report = export_with_report(&job, &mut |_| true).unwrap();
        assert_eq!(report.plan.mode, ExportMode::Reencode);
        assert_eq!(report.plan.cuts.len(), 1);
        assert_eq!(report.video_frames, 90);

        let video = check_video(&output);
        assert_eq!(video.decoded, 90);
        assert!((video.last - video.first - 89.0 / 30.0).abs() < 0.01);

        let samples = decode_track(&output, 0);
        let secs = samples.len() as f64 / SAMPLE_RATE as f64;
        assert!((secs - 3.0).abs() < 0.05, "audio lasts {secs} s");
        let before = tone_level(&input, 0, 880.0);
        let after = goertzel(&samples, 880.0);
        assert!(
            after < before * 0.05,
            "880 Hz still there: {after} vs {before}"
        );
    }

    #[test]
    fn keyframe_cuts_copy_the_packets() {
        let dir = TempDir::new("cut-key");
        let input = dir.0.join("in.mp4");
        make_marked_clip(&input);
        let output = dir.0.join("out.mp4");
        // Keyframes every second: the start moves to 0, the cut's end from 2.2 s to 2 s.
        let mut job = ExportJob::new(&input, &output, 0.4, 4.0);
        job.audio_tracks = vec![0];
        job.cuts = vec![(1.1, 2.2)];
        job.keyframe_cuts = true;
        let report = export_with_report(&job, &mut |_| true).unwrap();
        assert_eq!(report.plan.mode, ExportMode::StreamCopy);
        assert!(
            report.plan.start_secs.abs() < 0.02,
            "starts at {}",
            report.plan.start_secs
        );
        assert_eq!(report.plan.cuts.len(), 1);
        let (lo, hi) = report.plan.cuts[0];
        assert!(
            (hi - 2.0).abs() < 0.02 && (lo - 1.1).abs() < 0.04,
            "cut {lo}..{hi}"
        );

        let video = check_video(&output);
        let kept = 4.0 - (hi - lo);
        assert!(
            (video.decoded as f64 - kept * 30.0).abs() <= 1.0,
            "{} frames",
            video.decoded
        );
        let secs = decode_track(&output, 0).len() as f64 / SAMPLE_RATE as f64;
        assert!(
            (secs - kept).abs() < 0.05,
            "audio lasts {secs} s, video {kept} s"
        );

        // A start closer to the next keyframe moves forward to it.
        let mut job = ExportJob::new(&input, dir.0.join("late.mp4"), 0.7, 4.0);
        job.keyframe_cuts = true;
        let report = export_with_report(&job, &mut |_| true).unwrap();
        assert_eq!(report.plan.mode, ExportMode::StreamCopy);
        assert!((report.plan.start_secs - 1.0).abs() < 0.02);
    }

    #[test]
    fn cuts_also_apply_to_a_rebuilt_mix() {
        let dir = TempDir::new("cut-mix");
        let input = dir.0.join("in.mp4");
        make_mixed_clip(&input, 4);
        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 4.0);
        job.audio_tracks = vec![0, 1, 2];
        job.audio_gains = vec![(1, 0.0)];
        job.cuts = vec![(0.5, 1.5), (2.5, 3.0)];
        export(&job, &mut |_| true).unwrap();
        for track in 0..3 {
            let secs = decode_track(&output, track).len() as f64 / SAMPLE_RATE as f64;
            assert!((secs - 2.5).abs() < 0.05, "track {track} lasts {secs} s");
        }
        assert!(tone_level(&output, 0, 440.0) < tone_level(&input, 0, 440.0) * 0.01);
    }

    #[test]
    fn cuts_that_remove_everything_are_refused() {
        let dir = TempDir::new("cut-all");
        let input = dir.0.join("in.mp4");
        make_clip(&input, 2);
        let mut job = ExportJob::new(&input, dir.0.join("out.mp4"), 0.5, 1.5);
        job.cuts = vec![(0.0, 1.0), (0.9, 2.0)];
        assert!(export(&job, &mut |_| true).is_err());
    }

    /// Like the recorder's layout with an app next to "Other apps": `Mix`, `Game` (220 Hz),
    /// `Spotify` (pink noise) and `Other apps` (the same noise 91 samples late, plus 440 Hz).
    fn make_app_clip(path: &Path) {
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x180:rate=30:duration=6")
            .args(["-f", "lavfi", "-i", "anoisesrc=color=pink:amplitude=0.2:duration=6:sample_rate=48000:seed=42"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000"])
            .args(["-f", "lavfi", "-i", "sine=frequency=220:duration=6:sample_rate=48000"])
            .args([
                "-filter_complex",
                "[1:a]aformat=channel_layouts=stereo,asplit[sp1][sp2];[sp2]adelay=91S:all=1[spd];\
                 [2:a]aformat=channel_layouts=stereo[t];[spd][t]amix=inputs=2:normalize=0,asplit[o1][o2];\
                 [3:a]aformat=channel_layouts=stereo,asplit[g1][g2];[g1][o1]amix=inputs=2:normalize=0[mix]",
            ])
            .args(["-map", "0:v", "-map", "[mix]", "-map", "[g2]", "-map", "[sp1]", "-map", "[o2]"])
            .args(["-c:v", "libopenh264", "-b:v", "1M", "-g", "30", "-bf", "0"])
            .args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000"])
            .args(["-metadata:s:a:0", "handler_name=Mix", "-metadata:s:a:1", "handler_name=Game"])
            .args(["-metadata:s:a:2", "handler_name=Spotify"])
            .args(["-metadata:s:a:3", "handler_name=Other apps"])
            .arg(path)
            .status()
            .expect("cannot run ffmpeg.exe");
        assert!(status.success());
    }

    fn db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }

    #[test]
    fn a_removed_app_is_also_taken_out_of_other_apps_and_mix() {
        let dir = TempDir::new("unmix");
        let input = dir.0.join("in.mp4");
        make_app_clip(&input);
        let found = crate::unmix::find_apps_in_others(&input).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].app, found[0].others, found[0].lag), (2, 3, 91));

        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 6.0);
        job.audio_tracks = vec![0, 1, 3];
        export(&job, &mut |_| true).unwrap();
        let out = probe(&output).unwrap();
        assert_eq!(
            out.audio_tracks
                .iter()
                .map(|t| t.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Mix", "Game", "Other apps"]
        );
        // The noise (measured over 1 to 5 kHz, where only it has energy) drops; the tones stay.
        let band = |x: &[f32]| -> f64 {
            (0..40)
                .map(|i| goertzel(x, 1000.0 + 100.0 * i as f64).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        for (out_track, in_track) in [(2usize, 3usize), (0, 0)] {
            let before = decode_track(&input, in_track);
            let after = decode_track(&output, out_track);
            let drop = db(band(&before)) - db(band(&after));
            assert!(
                drop > 15.0,
                "track {out_track}: noise only {drop:.1} dB lower"
            );
            let tone = db(goertzel(&after, 440.0)) - db(goertzel(&before, 440.0));
            assert!(
                tone.abs() < 1.0,
                "track {out_track}: 440 Hz changed by {tone:.1} dB"
            );
        }
    }

    #[test]
    fn a_louder_app_is_louder_inside_other_apps_too() {
        let dir = TempDir::new("unmix-up");
        let input = dir.0.join("in.mp4");
        make_app_clip(&input);
        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 6.0);
        job.audio_tracks = vec![0, 1, 2, 3];
        job.audio_gains = vec![(2, 2.0)];
        export(&job, &mut |_| true).unwrap();
        let rise = db(goertzel(&decode_track(&output, 3), 3000.0))
            - db(goertzel(&decode_track(&input, 3), 3000.0));
        assert!(
            (rise - 6.0).abs() < 1.5,
            "noise in Other apps rose {rise:.1} dB"
        );
    }

    /// Luma of one frame at `secs`, as rows of `width` bytes.
    fn luma_at(path: &Path, secs: f64) -> Vec<u8> {
        let out = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-v", "error", "-ss", &secs.to_string(), "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gray",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    /// The clip's input drawn in where it is placed: W lit while held. Without an input file
    /// there is nothing to draw, and the packets are copied.
    #[test]
    fn draws_the_input_in() {
        use crate::inputfile::{Block, Event, Key, Pressed};
        let dir = TempDir::new("umbra-export-input");
        let input = dir.0.join("in.mp4");
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=black:size=320x180:rate=30:duration=3")
            .args(["-c:v", "libopenh264", "-bf", "0", "-g", "30"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());
        let overlay = crate::settings::InputOverlayPrefs {
            keys: crate::settings::OverlayKeys::Wasd,
            mouse: false,
            ticker: false,
            placement: crate::settings::Placement {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                ..Default::default()
            },
            opacity: 1.0,
            highlight: "#ffffff".into(),
            ..Default::default()
        };
        let mut job = ExportJob::new(&input, dir.0.join("plain.mp4"), 0.0, 3.0);
        job.input_overlay = Some(InputBurn {
            overlay: overlay.clone(),
        });
        assert_eq!(plan(&job).unwrap().mode, ExportMode::StreamCopy);

        let w = Key {
            vk: 0x57,
            scan: 0x11,
            ext: false,
        };
        crate::inputfile::write(
            &crate::inputfile::sidecar(&input),
            &[Block {
                start: 0.0,
                pressed: Pressed::default(),
                events: vec![(1.0, Event::KeyDown(w)), (2.0, Event::KeyUp(w))],
            }],
        )
        .unwrap();
        let output = dir.0.join("out.mp4");
        job.output = output.clone();
        assert_eq!(plan(&job).unwrap().mode, ExportMode::Reencode);
        run(&job);
        // Inside W, beside its letter: 1.7 key widths in, 0.2 down, at 160 px for the
        // layout's width.
        let unit = 160.0 / crate::inputdraw::layouts().wasd.width;
        let at = (unit * 0.2) as usize * 320 + (unit * 1.7) as usize;
        let before = luma_at(&output, 0.5)[at];
        let held = luma_at(&output, 1.5)[at];
        let after = luma_at(&output, 2.5)[at];
        assert!(held > 180, "W held: {held}");
        assert!(before < 80 && after < 80, "W up: {before}, {after}");
        // A key drawn, but not over the whole picture.
        assert!(luma_at(&output, 1.5)[170 * 320 + 300] < 30);

        // With 1.2 s to 1.8 s cut out, W shows for its 0.4 s left: 1.0 to 1.4 in the output.
        let cut = dir.0.join("cut.mp4");
        job.output = cut.clone();
        job.cuts = vec![(1.2, 1.8)];
        run(&job);
        assert!(luma_at(&cut, 1.1)[at] > 180, "before the cut");
        assert!(luma_at(&cut, 1.3)[at] > 180, "after the cut");
        assert!(luma_at(&cut, 1.6)[at] < 80, "released");
    }

    /// The camera (white) and a logo (red) drawn into a black screen where they are placed,
    /// the camera hidden where asked; shrinking keeps the camera as its own stream.
    #[test]
    fn draws_the_camera_and_logo_in() {
        let dir = TempDir::new("umbra-export-burn");
        let input = dir.0.join("in.mp4");
        let logo = dir.0.join("logo.png");
        let ffmpeg = crate::library::testutil::ffmpeg_exe();
        let status = std::process::Command::new(&ffmpeg)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=black:size=320x180:rate=30:duration=3")
            .args([
                "-f",
                "lavfi",
                "-i",
                "color=white:size=160x90:rate=30:duration=3",
            ])
            .args([
                "-map",
                "0",
                "-map",
                "1",
                "-c:v",
                "libopenh264",
                "-bf",
                "0",
                "-g",
                "30",
            ])
            .args(["-metadata:s:v:1", "handler_name=Camera"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=red:size=64x64",
            ])
            .args(["-frames:v", "1"])
            .arg(&logo)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(
            probe(&input).unwrap().camera.is_some(),
            "the test clip has a camera"
        );

        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 3.0);
        let rect = crate::settings::Placement {
            shape: crate::settings::OverlayShape::Rectangle,
            ..Default::default()
        };
        job.camera = Some(CameraBurn {
            placement: crate::settings::Placement {
                x: 0.5,
                y: 0.5,
                width: 0.25,
                ..rect.clone()
            },
            hidden: vec![(0.0, 1.0)],
        });
        job.logo = Some(LogoBurn {
            path: logo,
            placement: crate::settings::Placement {
                x: 0.0,
                y: 0.0,
                width: 0.25,
                ..rect
            },
        });
        let report = export_with_report(&job, &mut |_| true).unwrap();
        assert_eq!(report.plan.mode, ExportMode::Reencode);
        let out = format::input(&output).unwrap();
        assert_eq!(
            out.streams()
                .filter(crate::library::is_camera_stream)
                .count(),
            0
        );

        let at = |frame: &[u8], x: usize, y: usize| frame[y * 320 + x];
        let shown = luma_at(&output, 2.0);
        assert!(
            at(&shown, 200, 112) > 200,
            "camera drawn: {}",
            at(&shown, 200, 112)
        );
        assert!(
            at(&shown, 100, 150) < 40,
            "screen elsewhere: {}",
            at(&shown, 100, 150)
        );
        let red = at(&shown, 40, 40);
        assert!((45..90).contains(&red), "logo drawn: {red}");
        let hidden = luma_at(&output, 0.5);
        assert!(
            at(&hidden, 200, 112) < 40,
            "camera hidden: {}",
            at(&hidden, 200, 112)
        );
        assert!((45..90).contains(&at(&hidden, 40, 40)), "logo stays");

        let shrunk = dir.0.join("shrunk.mp4");
        let mut job = ExportJob::new(&input, &shrunk, 0.0, 4.0);
        job.force_reencode = true;
        job.keep_camera = true;
        export(&job, &mut |_| true).unwrap();
        let meta = probe(&shrunk).unwrap();
        assert_eq!(meta.camera, Some((160, 90)));
        assert!(meta.duration_secs > 2.9, "{}", meta.duration_secs);
    }

    /// A camera that stopped for a while: the editor's stretches and the export both leave
    /// it out there, and show it either side.
    #[test]
    fn camera_gaps_agree_in_editor_and_export() {
        let dir = TempDir::new("umbra-export-camera-gap");
        let input = dir.0.join("in.mp4");
        let status = std::process::Command::new(crate::library::testutil::ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=black:size=320x180:rate=30:duration=4")
            .args([
                "-f",
                "lavfi",
                "-i",
                "color=white:size=160x90:rate=30:duration=4",
            ])
            .args([
                "-map",
                "0",
                "-map",
                "1",
                "-c:v",
                "libopenh264",
                "-bf",
                "0",
                "-g",
                "30",
            ])
            .args([
                "-filter:v:1",
                "select='not(between(t,1,2.5))'",
                "-fps_mode:v:1",
                "passthrough",
            ])
            .args(["-metadata:s:v:1", "handler_name=Camera"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success());

        let track = split_camera(&input, &dir.0.join("cache")).unwrap().unwrap();
        assert_eq!(track.stretches.len(), 2, "{:?}", track.stretches);
        let ((a0, a1), (b0, b1)) = (track.stretches[0], track.stretches[1]);
        assert!(
            // The last frame before the gap (at 0.967 s) holds for the gap time.
            a0 < 0.05 && (a1 - (0.967 + CAMERA_GAP_SECS)).abs() < 0.05,
            "{:?}",
            track.stretches
        );
        assert!((b0 - 2.5).abs() < 0.05 && b1 > 3.9, "{:?}", track.stretches);

        let output = dir.0.join("out.mp4");
        let mut job = ExportJob::new(&input, &output, 0.0, 4.0);
        job.camera = Some(CameraBurn {
            placement: crate::settings::Placement {
                x: 0.5,
                y: 0.5,
                width: 0.25,
                shape: crate::settings::OverlayShape::Rectangle,
                ..Default::default()
            },
            hidden: Vec::new(),
        });
        export(&job, &mut |_| true).unwrap();
        let at = |secs: f64| luma_at(&output, secs)[112 * 320 + 200];
        assert!(at(0.5) > 200, "shown before the gap");
        assert!(at(1.2) > 200, "the last frame holds a moment");
        assert!(at(1.8) < 40, "gone in the gap");
        assert!(at(3.0) > 200, "back after it");
    }
}
