//! The clip library: reads clip metadata, scans a folder for clips and makes thumbnails.
//!
//! Everything here only reads clips. Nothing is written except the thumbnail cache.

use std::ffi::CStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::error::EAGAIN;
use ffmpeg_next::ffi::{
    AV_CODEC_FLAG_QSCALE, AVCodecContext, AVColorRange, AVColorSpace, AVPixelFormat, AVRational,
    AVSEEK_FLAG_BACKWARD, FF_QP2LAMBDA, SWS_CS_BT2020, SWS_CS_ITU601, SWS_CS_ITU709, av_seek_frame,
    avcodec_alloc_context3, avcodec_find_encoder_by_name, avcodec_free_context, avcodec_get_name,
    avcodec_open2, avcodec_receive_packet, avcodec_send_frame, sws_getCoefficients,
    sws_setColorspaceDetails,
};
use ffmpeg_next::packet::Mut as _;
use ffmpeg_next::software::scaling;
use ffmpeg_next::threading::{Config as ThreadConfig, Type as ThreadType};
use ffmpeg_next::util::format::Pixel;
use ffmpeg_next::{DictionaryRef, Error as AvError, Packet, codec, format, frame, media};
use serde::{Deserialize, Serialize};

/// One audio track of a clip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackMeta {
    /// Position among the file's audio streams, counting from 0. This is the number the
    /// export takes in `ExportJob::audio_tracks`.
    pub index: usize,
    /// Name the recorder gave the track (`Game`, `Microphone`, ...), or `Track N`.
    pub title: String,
    pub role: MixRole,
}

/// How a track relates to the combined first track that most players play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixRole {
    /// The combined track itself.
    Mix,
    /// Summed into the combined track.
    InMix,
    /// An app with its own track whose sound is also inside "Other apps", as Umbra recorded
    /// before "Other apps" left such apps out. Not summed into the combined track directly.
    InOthers,
    /// Only on its own track: the microphone, or any track of a file without a combined track.
    Separate,
}

/// The role of each track. `mix_tag` is the `mix` tag newer clips carry: the titles of the
/// tracks in the combined first track, separated by `|`. Without it the layout of older
/// recordings is assumed: a first track called `Mix` sums all others, except app tracks
/// recorded next to `Other apps`, which already contains them.
pub fn mix_roles(titles: &[&str], mix_tag: Option<&str>) -> Vec<MixRole> {
    let has_mix = titles.len() > 1 && titles[0].eq_ignore_ascii_case("Mix");
    if !has_mix {
        return vec![MixRole::Separate; titles.len()];
    }
    if let Some(tag) = mix_tag {
        let parts: Vec<&str> = tag.split('|').collect();
        return titles
            .iter()
            .enumerate()
            .map(|(i, title)| {
                if i == 0 {
                    MixRole::Mix
                } else if parts.iter().any(|p| p.eq_ignore_ascii_case(title)) {
                    MixRole::InMix
                } else {
                    MixRole::Separate
                }
            })
            .collect();
    }
    let has_others = titles.iter().any(|t| t.eq_ignore_ascii_case("Other apps"));
    titles
        .iter()
        .enumerate()
        .map(|(i, title)| {
            let base = ["Game", "Other apps", "System", "Microphone"]
                .iter()
                .any(|b| title.eq_ignore_ascii_case(b));
            if i == 0 {
                MixRole::Mix
            } else if has_others && !base {
                MixRole::InOthers
            } else {
                MixRole::InMix
            }
        })
        .collect()
}

/// What the library knows about one clip file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMeta {
    pub path: PathBuf,
    pub file_name: String,
    /// When the file was last modified, as ISO 8601 in UTC, for example `2026-09-30T18:33:45.123Z`.
    pub created_at: String,
    /// Length of the whole file, audio included.
    pub duration_secs: f64,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// `h264`, `hevc` or `av1`; FFmpeg's own name for anything else.
    pub codec: String,
    pub audio_tracks: Vec<AudioTrackMeta>,
    /// A screenshot (PNG or JPEG) rather than a video.
    pub is_image: bool,
    /// Average bitrate of the whole file in bit/s; 0 if unknown.
    #[serde(default)]
    pub bit_rate: u64,
    /// How the clip was recorded, as Umbra wrote it into the file (`game`, `game_exe`,
    /// `detected`, `encoder`, `rate`, ...). Empty for clips from older versions or elsewhere.
    #[serde(default)]
    pub details: Vec<(String, String)>,
    /// Size of the webcam stream recorded next to the screen, if the clip has one.
    #[serde(default)]
    pub camera: Option<(u32, u32)>,
}

/// Screenshots live next to the clips as PNG or JPEG files.
pub fn is_image_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        ["png", "jpg", "jpeg"]
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    })
}

fn is_library_path(path: &Path) -> bool {
    is_image_path(path)
        || path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
}

/// Initialises FFmpeg once, however many threads ask at the same time. (`ffmpeg_next::init`
/// fills a global table of error strings on every call.)
pub(crate) fn init_ffmpeg() -> Result<()> {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let _ = ffmpeg_next::init();
    });
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Probing
// ---------------------------------------------------------------------------------------

/// Reads the metadata of one clip file.
pub fn probe(path: &Path) -> Result<ClipMeta> {
    init_ffmpeg()?;
    let file = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    let input = format::input(path).with_context(|| format!("cannot open {}", path.display()))?;
    let video =
        screen_stream(&input).ok_or_else(|| anyhow!("{} has no video stream", path.display()))?;
    let camera = input.streams().find(is_camera_stream).map(|s| unsafe {
        let raw = s.parameters().as_ptr();
        ((*raw).width.max(0) as u32, (*raw).height.max(0) as u32)
    });

    let params = video.parameters();
    let (width, height, codec_id) = unsafe {
        let raw = params.as_ptr();
        (
            (*raw).width.max(0) as u32,
            (*raw).height.max(0) as u32,
            (*raw).codec_id,
        )
    };
    let codec = unsafe { CStr::from_ptr(avcodec_get_name(codec_id)) }
        .to_string_lossy()
        .into_owned();

    let time_base = video.time_base();
    let duration_secs = if input.duration() > 0 {
        input.duration() as f64 / 1_000_000.0
    } else if video.duration() > 0 && time_base.denominator() != 0 {
        video.duration() as f64 * time_base.numerator() as f64 / time_base.denominator() as f64
    } else {
        0.0
    };

    let is_image = is_image_path(path);
    let mut fps = rational_to_f64(frame_rate(&input, &video));
    if fps <= 0.0 {
        fps = rational_to_f64(video.rate());
    }
    let fps = if is_image {
        0.0
    } else {
        (fps * 1000.0).round() / 1000.0
    };
    let duration_secs = if is_image { 0.0 } else { duration_secs };

    let titles: Vec<String> = input
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Audio)
        .enumerate()
        .map(|(index, stream)| {
            stream_title(&stream.metadata()).unwrap_or_else(|| format!("Track {}", index + 1))
        })
        .collect();
    let mix_tag = input
        .metadata()
        .get(&format!("{}mix", crate::mux::TAG_PREFIX))
        .map(str::to_string);
    let roles = mix_roles(
        &titles.iter().map(String::as_str).collect::<Vec<_>>(),
        mix_tag.as_deref(),
    );
    let audio_tracks = titles
        .into_iter()
        .zip(roles)
        .enumerate()
        .map(|(index, (title, role))| AudioTrackMeta { index, title, role })
        .collect();

    let details = input
        .metadata()
        .iter()
        .filter_map(|(k, v)| {
            k.strip_prefix(crate::mux::TAG_PREFIX)
                .map(|name| (name.to_string(), v.to_string()))
        })
        .collect();

    Ok(ClipMeta {
        path: path.to_path_buf(),
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        created_at: iso8601(recorded_at(
            &input.metadata(),
            file.modified().unwrap_or(UNIX_EPOCH),
            duration_secs,
        )),
        duration_secs,
        size_bytes: file.len(),
        width,
        height,
        fps,
        codec,
        audio_tracks,
        is_image,
        bit_rate: input.bit_rate().max(0) as u64,
        details,
        camera,
    })
}

/// The webcam stream Umbra records next to the screen: a video stream named `Camera`.
pub fn is_camera_stream(stream: &ffmpeg_next::format::stream::Stream) -> bool {
    stream.parameters().medium() == media::Type::Video
        && stream_title(&stream.metadata()).as_deref() == Some(crate::camera::CAMERA_TITLE)
}

/// The recorded screen: the first video stream that is not the camera.
/// The frame rate the screen was recorded at. A recording that leaves out the frames of a
/// still screen averages fewer, so Umbra stores the real rate in its `fps` tag; any other
/// file gives its average.
pub fn frame_rate(
    input: &format::context::Input,
    stream: &format::stream::Stream,
) -> ffmpeg_next::Rational {
    input
        .metadata()
        .get(&format!("{}fps", crate::mux::TAG_PREFIX))
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|&fps| fps > 0)
        .map_or_else(
            || stream.avg_frame_rate(),
            |fps| ffmpeg_next::Rational::new(fps, 1),
        )
}

pub fn screen_stream(
    input: &format::context::Input,
) -> Option<ffmpeg_next::format::stream::Stream<'_>> {
    input
        .streams()
        .find(|s| s.parameters().medium() == media::Type::Video && !is_camera_stream(s))
}

fn rational_to_f64(r: ffmpeg_next::Rational) -> f64 {
    if r.denominator() == 0 {
        0.0
    } else {
        r.numerator() as f64 / r.denominator() as f64
    }
}

/// The name of a track from its metadata, or `None` when the file gives it no real name.
/// The MP4 muxer writes the track name as `handler_name`; FFmpeg fills that with
/// `SoundHandler` or `VideoHandler` when nobody set one, which says nothing.
pub(crate) fn stream_title(metadata: &DictionaryRef) -> Option<String> {
    ["title", "handler_name"].iter().find_map(|key| {
        let value = metadata.get(key)?.trim();
        (!value.is_empty() && !is_generic_handler_name(value)).then(|| value.to_string())
    })
}

fn is_generic_handler_name(name: &str) -> bool {
    let normalized: String = name
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    if matches!(
        normalized.as_str(),
        "soundhandler" | "videohandler" | "audiohandler" | "coremediaaudio" | "coremediavideo"
    ) {
        return true;
    }
    // Other muxers: "GPAC ISO Audio Handler", "Mainconcept MP4 Sound Media Handler", ...
    normalized.ends_with("handler")
        && [
            "sound", "audio", "video", "media", "data", "subtitle", "timecode", "iso",
        ]
        .iter()
        .any(|word| normalized.contains(word))
}

/// `2026-09-30T18:33:45.123Z` for a file time.
/// Video and audio packets in a file, read to the end (which also shows it is all there).
/// `progress` gets the bytes read so far.
pub fn packet_counts(path: &Path, progress: &mut dyn FnMut(u64)) -> Result<(usize, usize)> {
    let mut input =
        format::input(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let kinds: Vec<media::Type> = input.streams().map(|s| s.parameters().medium()).collect();
    let (mut video, mut audio) = (0, 0);
    for (n, (stream, packet)) in input.packets().enumerate() {
        match kinds.get(stream.index()) {
            Some(media::Type::Video) => video += 1,
            Some(media::Type::Audio) => audio += 1,
            _ => {}
        }
        if n % 256 == 0 && packet.position() > 0 {
            progress(packet.position() as u64);
        }
    }
    Ok((video, audio))
}

/// When a clip was recorded: Umbra's own tag (a recording's start), else the file's creation
/// time when it is believable (ShadowPlay, OBS and phones write one), else when the file was
/// written less its length, which is when a clip began since it is written once it is saved.
fn recorded_at(
    metadata: &ffmpeg_next::DictionaryRef,
    modified: SystemTime,
    duration_secs: f64,
) -> SystemTime {
    let tagged = metadata
        .get(&format!("{}recorded", crate::mux::TAG_PREFIX))
        .and_then(parse_iso8601);
    if let Some(at) = tagged {
        return at;
    }
    let believable = |at: &SystemTime| {
        let after_2005 = at
            .duration_since(UNIX_EPOCH)
            .is_ok_and(|d| d.as_secs() > 1_104_537_600);
        let not_after_file = *at <= modified + std::time::Duration::from_secs(86_400);
        after_2005 && not_after_file
    };
    if let Some(at) = metadata
        .get("creation_time")
        .and_then(parse_iso8601)
        .filter(believable)
    {
        return at;
    }
    modified
        .checked_sub(std::time::Duration::from_secs_f64(
            duration_secs.clamp(0.0, 86_400.0),
        ))
        .unwrap_or(modified)
}

/// Reads `2024-01-15T20:30:45.123Z` (fractions and the `Z` optional; no other offsets).
/// True when the video is 10-bit (an HDR10 recording), or cannot be read.
pub fn is_ten_bit(path: &Path) -> bool {
    use ffmpeg_next::ffi::AVPixelFormat;
    let Ok(input) = ffmpeg_next::format::input(&path) else {
        return true;
    };
    screen_stream(&input).is_some_and(|s| unsafe {
        let format = (*s.parameters().as_ptr()).format;
        [
            AVPixelFormat::AV_PIX_FMT_P010LE as i32,
            AVPixelFormat::AV_PIX_FMT_YUV420P10LE as i32,
        ]
        .contains(&format)
    })
}

pub fn parse_iso8601(text: &str) -> Option<SystemTime> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once(['T', ' '])?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (t.next()??, t.next()??, t.next().unwrap_or(Some(0))?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || s > 60 {
        return None;
    }
    // Days from 1970-01-01 to the date (Howard Hinnant's days_from_civil).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + min * 60 + s;
    let digits: String = frac
        .chars()
        .take_while(char::is_ascii_digit)
        .take(9)
        .collect();
    let nanos = if digits.is_empty() {
        0
    } else {
        digits.parse::<u64>().ok()? * 10u64.pow(9 - digits.len() as u32)
    };
    if secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + std::time::Duration::new(secs as u64, nanos as u32))
}

pub fn iso8601(time: SystemTime) -> String {
    let millis: i64 = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_millis() as i64,
        Err(before) => -(before.duration().as_millis() as i64),
    };
    let secs = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let in_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60
    )
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

// ---------------------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------------------

/// Finds every `.mp4` under `folder` (subfolders included) and reads its metadata, newest
/// first. `.part` files (clips still being written) are ignored, and files that cannot be
/// read are skipped with a warning. A folder that does not exist gives an empty list.
pub fn scan(folder: &Path) -> Vec<ClipMeta> {
    if init_ffmpeg().is_err() {
        log::warn!("ffmpeg init failed; the clip list is empty");
        return Vec::new();
    }
    let mut files = Vec::new();
    collect_mp4_files(folder, &mut files);

    // Opening a file costs a few milliseconds, so a big library is probed on several threads.
    let results: Vec<Mutex<Option<ClipMeta>>> = files.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8)
        .min(files.len().div_ceil(4))
        .max(1);
    let work = || {
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(path) = files.get(i) else { break };
            match probe(path) {
                Ok(meta) => *results[i].lock().unwrap() = Some(meta),
                Err(err) => log::warn!("skipping {}: {err:#}", path.display()),
            }
        }
    };
    if workers == 1 {
        work();
    } else {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(work);
            }
        });
    }

    let mut clips: Vec<ClipMeta> = results
        .into_iter()
        .filter_map(|slot| slot.into_inner().unwrap())
        .collect();
    // The ISO strings have a fixed width, so comparing them compares the times.
    clips.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    clips
}

/// Every clip, recording and screenshot file under `folder`.
pub fn library_paths(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_mp4_files(folder, &mut out);
    out
}

fn collect_mp4_files(folder: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::warn!("cannot read folder {}: {err}", folder.display());
            }
            return;
        }
    };
    for entry in entries.flatten() {
        // `file_type` does not follow links, so a link back up the tree cannot loop forever.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect_mp4_files(&path, out);
        } else if kind.is_file() && is_library_path(&path) {
            out.push(path);
        }
    }
}

/// The game a clip belongs to: the name of the first folder under `clip_folder_root` on
/// the way to the clip. A clip directly in the root belongs to no game. Folder names are
/// compared without regard to case, as Windows does.
pub fn game_from_path(clip_folder_root: &Path, path: &Path) -> Option<String> {
    let root = without_verbatim_prefix(clip_folder_root);
    let path = without_verbatim_prefix(path);
    let root: Vec<Component> = root.components().collect();
    let path: Vec<Component> = path.components().collect();
    // The clip has to be at least a folder and a file below the root.
    if path.len() < root.len() + 2 {
        return None;
    }
    let lower = |c: &Component| c.as_os_str().to_string_lossy().to_lowercase();
    if !root.iter().zip(&path).all(|(a, b)| lower(a) == lower(b)) {
        return None;
    }
    match path[root.len()] {
        Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
        _ => None,
    }
}

/// `\\?\D:\Clips` and `D:\Clips` are the same place; make them compare equal.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

// ---------------------------------------------------------------------------------------
// Thumbnails
// ---------------------------------------------------------------------------------------

const THUMBNAIL_WIDTH: u32 = 320;
/// How far into the clip the thumbnail is taken, when the clip is long enough.
const THUMBNAIL_AT_SECS: f64 = 1.0;
/// JPEG quality on FFmpeg's scale (2 best, 31 worst). 3 is about JPEG quality 90.
const THUMBNAIL_QSCALE: i32 = 3;

/// Makes a JPEG thumbnail, 320 pixels wide, of the frame one second into the clip (the
/// first frame if the clip is shorter) and returns its path. The file is cached in
/// `cache_dir` under a name made from the clip's path, modification time and size, so an
/// edited or replaced clip gets a new thumbnail and an unchanged one is not decoded again.
pub fn thumbnail(path: &Path, cache_dir: &Path) -> Result<PathBuf> {
    init_ffmpeg()?;
    let file = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    let target = thumbnail_target(path, &file, cache_dir);
    if fs::metadata(&target).is_ok_and(|m| m.len() > 0) {
        return Ok(target);
    }
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("cannot create {}", cache_dir.display()))?;

    let jpeg = render_thumbnail(path)
        .with_context(|| format!("cannot make a thumbnail of {}", path.display()))?;

    // Write under another name and rename, so a reader never sees half a file.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let temp = cache_dir.join(format!(
        "{}.{}-{}.tmp",
        target.file_stem().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temp, &jpeg).with_context(|| format!("cannot write {}", temp.display()))?;
    if let Err(err) = fs::rename(&temp, &target) {
        let _ = fs::remove_file(&temp);
        // Another thread may have finished the same thumbnail first.
        if !fs::metadata(&target).is_ok_and(|m| m.len() > 0) {
            return Err(err).with_context(|| format!("cannot write {}", target.display()));
        }
    }
    Ok(target)
}

/// The thumbnail of `path` if it is already in `cache_dir`. Never decodes anything: it
/// costs one file lookup for the clip and one for the cache entry, so it is cheap enough
/// to call for every clip in a list.
pub fn cached_thumbnail(path: &Path, cache_dir: &Path) -> Option<PathBuf> {
    let file = fs::metadata(path).ok()?;
    let target = thumbnail_target(path, &file, cache_dir);
    fs::metadata(&target)
        .is_ok_and(|m| m.len() > 0)
        .then_some(target)
}

fn thumbnail_target(path: &Path, file: &fs::Metadata, cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("{}.jpg", thumbnail_key(path, file)))
}

/// 16 hex digits identifying a clip as it is now. FNV-1a, so the names stay the same
/// between builds of the app and a thumbnail cache survives updates.
fn thumbnail_key(path: &Path, file: &fs::Metadata) -> String {
    let modified = file
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    // Windows paths do not care about case; neither should the cache.
    let text = format!(
        "{}\0{modified}\0{}",
        path.to_string_lossy().to_lowercase(),
        file.len()
    );
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn render_thumbnail(path: &Path) -> Result<Vec<u8>> {
    let (decoded, colorspace, range) = decode_thumbnail_frame(path)?;
    let (src_w, src_h) = (decoded.width(), decoded.height());
    if src_w == 0 || src_h == 0 {
        bail!("the decoded frame is empty");
    }
    let width = THUMBNAIL_WIDTH.min(src_w) & !1;
    let height = (((u64::from(src_h) * u64::from(width) + u64::from(src_w) / 2) / u64::from(src_w))
        as u32
        & !1)
        .max(2);

    // Two steps, so the colours come out right. The clip is BT.709 and a JPEG reader
    // assumes BT.601 (JFIF). Converting YUV to YUV would leave the numbers alone and
    // shift the colours, so go through RGB with the clip's real matrix.
    let mut to_rgb = scaling::Context::get(
        decoded.format(),
        src_w,
        src_h,
        Pixel::RGB24,
        width,
        height,
        scaling::Flags::AREA | scaling::Flags::ACCURATE_RND,
    )
    .context("cannot set up the thumbnail scaler")?;
    unsafe {
        let source_table = sws_getCoefficients(match colorspace {
            AVColorSpace::AVCOL_SPC_BT709 => SWS_CS_ITU709,
            AVColorSpace::AVCOL_SPC_BT2020_NCL | AVColorSpace::AVCOL_SPC_BT2020_CL => SWS_CS_BT2020,
            AVColorSpace::AVCOL_SPC_BT470BG | AVColorSpace::AVCOL_SPC_SMPTE170M => SWS_CS_ITU601,
            // Nothing said: HD is 709, SD is 601.
            _ if src_h >= 720 => SWS_CS_ITU709,
            _ => SWS_CS_ITU601,
        });
        sws_setColorspaceDetails(
            to_rgb.as_mut_ptr(),
            source_table,
            i32::from(range == AVColorRange::AVCOL_RANGE_JPEG),
            source_table,
            1,
            0,
            1 << 16,
            1 << 16,
        );
    }
    let mut rgb = frame::Video::empty();
    to_rgb
        .run(&decoded, &mut rgb)
        .context("cannot scale the thumbnail")?;

    // RGB to full-range YUV with JFIF's BT.601 matrix. The format is plain YUV420P with the
    // range set explicitly; the deprecated YUVJ420P makes swscale complain.
    let mut to_yuv = scaling::Context::get(
        Pixel::RGB24,
        width,
        height,
        Pixel::YUV420P,
        width,
        height,
        scaling::Flags::BICUBIC | scaling::Flags::ACCURATE_RND,
    )
    .context("cannot set up the thumbnail colour conversion")?;
    unsafe {
        let jfif_table = sws_getCoefficients(SWS_CS_ITU601);
        sws_setColorspaceDetails(
            to_yuv.as_mut_ptr(),
            jfif_table,
            1,
            jfif_table,
            1,
            0,
            1 << 16,
            1 << 16,
        );
    }
    let mut yuv = frame::Video::empty();
    to_yuv
        .run(&rgb, &mut yuv)
        .context("cannot convert the thumbnail colours")?;

    encode_jpeg(&mut yuv)
}

/// Decodes the frame to use as thumbnail. Returns it with the colour matrix and range
/// the decoder reported.
fn decode_thumbnail_frame(path: &Path) -> Result<(frame::Video, AVColorSpace, AVColorRange)> {
    let mut input =
        format::input(path).with_context(|| format!("cannot open {}", path.display()))?;
    let (index, time_base, start_time, params, frame_ticks) = {
        let stream = screen_stream(&input).ok_or_else(|| anyhow!("no video stream"))?;
        let time_base = stream.time_base();
        let fps = rational_to_f64(frame_rate(&input, &stream));
        let frame_ticks = if fps > 0.0 && time_base.numerator() > 0 {
            (time_base.denominator() as f64 / time_base.numerator() as f64 / fps).round() as i64
        } else {
            0
        };
        let start = stream.start_time();
        (
            stream.index(),
            time_base,
            if start == ffmpeg_next::ffi::AV_NOPTS_VALUE {
                0
            } else {
                start
            },
            stream.parameters(),
            frame_ticks,
        )
    };

    let mut context = codec::context::Context::from_parameters(params)?;
    context.set_threading(ThreadConfig {
        kind: ThreadType::Slice,
        count: 0,
    });
    let mut decoder = context
        .decoder()
        .video()
        .context("cannot open the video decoder")?;

    let duration = input.duration() as f64 / 1_000_000.0;
    let target_secs = if duration > THUMBNAIL_AT_SECS {
        THUMBNAIL_AT_SECS
    } else {
        0.0
    };
    let target_ticks = start_time
        + (target_secs * time_base.denominator() as f64 / time_base.numerator().max(1) as f64)
            .round() as i64;
    if target_secs > 0.0 {
        // To the keyframe at or before the target; a failed seek just decodes from the start.
        let code = unsafe {
            av_seek_frame(
                input.as_mut_ptr(),
                index as i32,
                target_ticks,
                AVSEEK_FLAG_BACKWARD,
            )
        };
        if code < 0 {
            log::debug!("seek failed in {}: {}", path.display(), AvError::from(code));
        }
    }

    // A frame at or after the target, or the last one decoded if the clip ends first.
    const MAX_FRAMES: usize = 600;
    let mut decoded_frames = 0;
    let mut last: Option<frame::Video> = None;
    let wanted = |f: &frame::Video| {
        f.timestamp()
            .or(f.pts())
            .is_none_or(|t| t + frame_ticks / 2 >= target_ticks)
    };
    let mut take = |decoder: &mut ffmpeg_next::decoder::Video| -> Result<Option<frame::Video>> {
        loop {
            let mut f = frame::Video::empty();
            match decoder.receive_frame(&mut f) {
                Ok(()) => {
                    decoded_frames += 1;
                    if wanted(&f) || decoded_frames >= MAX_FRAMES {
                        return Ok(Some(f));
                    }
                    last = Some(f);
                }
                Err(AvError::Other { errno }) if errno == EAGAIN => return Ok(None),
                Err(AvError::Eof) => return Ok(None),
                Err(err) => return Err(err.into()),
            }
        }
    };

    let mut found = None;
    'read: for (stream, packet) in input.packets() {
        if stream.index() != index {
            continue;
        }
        if decoder.send_packet(&packet).is_err() {
            continue;
        }
        if let Some(f) = take(&mut decoder)? {
            found = Some(f);
            break 'read;
        }
    }
    if found.is_none() {
        let _ = decoder.send_eof();
        found = take(&mut decoder)?;
    }
    let decoded = found
        .or(last)
        .ok_or_else(|| anyhow!("the video has no decodable frame"))?;
    let (space, range) = unsafe {
        let raw = decoded.as_ptr();
        ((*raw).colorspace, (*raw).color_range)
    };
    Ok((decoded, space, range))
}

/// Encodes a full-range (JPEG range) 4:2:0 frame as a JPEG with FFmpeg's MJPEG encoder.
fn encode_jpeg(frame: &mut frame::Video) -> Result<Vec<u8>> {
    encode_jpeg_at(frame, THUMBNAIL_QSCALE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    /// With FFmpeg's quality scale: 2 is best, 31 worst.
    Jpeg {
        qscale: i32,
    },
}

/// Encodes tightly packed RGB24 pixels as a PNG or JPEG file.
pub fn encode_image(rgb: &[u8], width: u32, height: u32, format: ImageFormat) -> Result<Vec<u8>> {
    init_ffmpeg()?;
    if rgb.len() != (width * height * 3) as usize {
        bail!(
            "image data is {} bytes, expected {}",
            rgb.len(),
            width * height * 3
        );
    }
    let mut frame = frame::Video::new(Pixel::RGB24, width, height);
    let stride = frame.stride(0);
    let row = width as usize * 3;
    for (y, line) in rgb.chunks_exact(row).enumerate() {
        frame.data_mut(0)[y * stride..y * stride + row].copy_from_slice(line);
    }
    match format {
        ImageFormat::Png => encode_png(&mut frame),
        ImageFormat::Jpeg { qscale } => {
            // JPEG wants even sizes for 4:2:0; drop a last odd row or column.
            let (w, h) = (width & !1, height & !1);
            let mut to_yuv = scaling::Context::get(
                Pixel::RGB24,
                width,
                height,
                Pixel::YUV420P,
                w,
                h,
                scaling::Flags::BICUBIC | scaling::Flags::ACCURATE_RND,
            )
            .context("cannot set up the JPEG colour conversion")?;
            unsafe {
                let jfif_table = sws_getCoefficients(SWS_CS_ITU601);
                sws_setColorspaceDetails(
                    to_yuv.as_mut_ptr(),
                    jfif_table,
                    1,
                    jfif_table,
                    1,
                    0,
                    1 << 16,
                    1 << 16,
                );
            }
            let mut yuv = frame::Video::empty();
            to_yuv
                .run(&frame, &mut yuv)
                .context("cannot convert the image colours")?;
            encode_jpeg_at(&mut yuv, qscale)
        }
    }
}

fn encode_png(frame: &mut frame::Video) -> Result<Vec<u8>> {
    struct Context(*mut AVCodecContext);
    impl Drop for Context {
        fn drop(&mut self) {
            unsafe { avcodec_free_context(&mut self.0) };
        }
    }
    unsafe {
        let encoder = avcodec_find_encoder_by_name(c"png".as_ptr());
        if encoder.is_null() {
            bail!("this FFmpeg build has no PNG encoder");
        }
        let ctx = Context(avcodec_alloc_context3(encoder));
        if ctx.0.is_null() {
            bail!("avcodec_alloc_context3 failed");
        }
        (*ctx.0).width = frame.width() as i32;
        (*ctx.0).height = frame.height() as i32;
        (*ctx.0).pix_fmt = AVPixelFormat::AV_PIX_FMT_RGB24;
        (*ctx.0).time_base = AVRational { num: 1, den: 25 };
        let code = avcodec_open2(ctx.0, encoder, std::ptr::null_mut());
        if code < 0 {
            bail!("cannot open the PNG encoder: {}", AvError::from(code));
        }
        let raw = frame.as_mut_ptr();
        (*raw).pts = 0;
        let code = avcodec_send_frame(ctx.0, raw);
        if code < 0 {
            bail!("PNG encoder rejected the frame: {}", AvError::from(code));
        }
        let _ = avcodec_send_frame(ctx.0, std::ptr::null());
        let mut packet = Packet::empty();
        let code = avcodec_receive_packet(ctx.0, packet.as_mut_ptr());
        if code < 0 {
            bail!("PNG encoder produced no data: {}", AvError::from(code));
        }
        packet
            .data()
            .map(<[u8]>::to_vec)
            .ok_or_else(|| anyhow!("PNG encoder produced an empty packet"))
    }
}

fn encode_jpeg_at(frame: &mut frame::Video, qscale: i32) -> Result<Vec<u8>> {
    struct Context(*mut AVCodecContext);
    impl Drop for Context {
        fn drop(&mut self) {
            unsafe { avcodec_free_context(&mut self.0) };
        }
    }
    unsafe {
        let encoder = avcodec_find_encoder_by_name(c"mjpeg".as_ptr());
        if encoder.is_null() {
            bail!("this FFmpeg build has no MJPEG encoder");
        }
        let ctx = Context(avcodec_alloc_context3(encoder));
        if ctx.0.is_null() {
            bail!("avcodec_alloc_context3 failed");
        }
        (*ctx.0).width = frame.width() as i32;
        (*ctx.0).height = frame.height() as i32;
        (*ctx.0).pix_fmt = AVPixelFormat::AV_PIX_FMT_YUV420P;
        (*ctx.0).time_base = AVRational { num: 1, den: 25 };
        (*ctx.0).color_range = AVColorRange::AVCOL_RANGE_JPEG;
        (*ctx.0).flags |= AV_CODEC_FLAG_QSCALE as i32;
        (*ctx.0).global_quality = FF_QP2LAMBDA * qscale;
        let code = avcodec_open2(ctx.0, encoder, std::ptr::null_mut());
        if code < 0 {
            bail!("cannot open the MJPEG encoder: {}", AvError::from(code));
        }

        let raw = frame.as_mut_ptr();
        (*raw).pts = 0;
        (*raw).quality = FF_QP2LAMBDA * qscale;
        (*raw).pict_type = ffmpeg_next::ffi::AVPictureType::AV_PICTURE_TYPE_NONE;
        let code = avcodec_send_frame(ctx.0, raw);
        if code < 0 {
            bail!("JPEG encoder rejected the frame: {}", AvError::from(code));
        }
        let code = avcodec_send_frame(ctx.0, std::ptr::null());
        if code < 0 && code != ffmpeg_next::ffi::AVERROR_EOF {
            bail!("JPEG encoder flush failed: {}", AvError::from(code));
        }
        let mut packet = Packet::empty();
        let code = avcodec_receive_packet(ctx.0, packet.as_mut_ptr());
        if code < 0 {
            bail!("JPEG encoder produced no data: {}", AvError::from(code));
        }
        packet
            .data()
            .map(<[u8]>::to_vec)
            .ok_or_else(|| anyhow!("JPEG encoder produced an empty packet"))
    }
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

/// Helpers for the tests of this module and of `export`.
#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Folder with `ffmpeg.exe`, from the `FFMPEG_DIR` that `.cargo/config.toml` sets.
    pub fn ffmpeg_exe() -> PathBuf {
        let root = std::env::var_os("FFMPEG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../third_party/ffmpeg")
            });
        root.join("bin").join("ffmpeg.exe")
    }

    /// A fresh empty folder in the system temp folder. Removed when dropped.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "umbra-test-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Makes a clip that looks like the recorder's: H.264, one keyframe per second, no
    /// B-frames, and two named AAC tracks. `secs` long, 640x360 at 60 fps. Uses OpenH264
    /// so it needs no GPU.
    pub fn make_clip(path: &Path, secs: u32) {
        let exe = ffmpeg_exe();
        assert!(exe.exists(), "ffmpeg.exe not found at {}", exe.display());
        let duration = secs.to_string();
        let status = Command::new(exe)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size=640x360:rate=60:duration={duration}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={duration}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=880:duration={duration}"))
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a"])
            .args(["-c:v", "libopenh264", "-b:v", "3M", "-g", "60", "-bf", "0"])
            .args(["-c:a", "aac", "-b:a", "96k"])
            .args([
                "-metadata:s:a:0",
                "title=Game",
                "-metadata:s:a:0",
                "handler_name=Game",
            ])
            .args([
                "-metadata:s:a:1",
                "title=Discord",
                "-metadata:s:a:1",
                "handler_name=Discord",
            ])
            .args(["-movflags", "+faststart"])
            .arg(path)
            .status()
            .expect("cannot run ffmpeg.exe");
        assert!(status.success(), "ffmpeg.exe failed to make the test clip");
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{TempDir, make_clip};
    use super::*;

    #[test]
    fn mix_roles_follow_the_recorder_layout() {
        use MixRole::*;
        // Older clips: no tag, apps recorded next to "Other apps" are inside it.
        assert_eq!(
            mix_roles(
                &["Mix", "Game", "Discord", "Other apps", "Microphone"],
                None
            ),
            vec![Mix, InMix, InOthers, InMix, InMix]
        );
        assert_eq!(
            mix_roles(&["Mix", "Game", "Discord"], None),
            vec![Mix, InMix, InMix]
        );
        assert_eq!(mix_roles(&["System"], None), vec![Separate]);
        assert_eq!(
            mix_roles(&["Game", "Discord"], None),
            vec![Separate, Separate]
        );
        // Newer clips say what is in the combined track; the microphone is not.
        assert_eq!(
            mix_roles(
                &["Mix", "Game", "Discord", "Other apps", "Microphone"],
                Some("Game|Discord|Other apps")
            ),
            vec![Mix, InMix, InMix, InMix, Separate]
        );
    }

    #[test]
    fn iso8601_reads_back() {
        let at = |secs: u64, ms: u32| UNIX_EPOCH + std::time::Duration::new(secs, ms * 1_000_000);
        for t in [at(0, 0), at(1_000_000_000, 0), at(1_790_000_000, 123)] {
            assert_eq!(parse_iso8601(&iso8601(t)), Some(t));
        }
        assert_eq!(
            parse_iso8601("2001-09-09T01:46:40Z"),
            Some(at(1_000_000_000, 0))
        );
        assert_eq!(
            parse_iso8601("2001-09-09 01:46:40.5"),
            Some(at(1_000_000_000, 500))
        );
        assert_eq!(parse_iso8601("2001-13-09T01:46:40Z"), None);
        assert_eq!(parse_iso8601("not a date"), None);
    }

    #[test]
    fn iso8601_formats_known_times() {
        assert_eq!(iso8601(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let at = |secs: u64, ms: u32| UNIX_EPOCH + std::time::Duration::new(secs, ms * 1_000_000);
        assert_eq!(iso8601(at(1_000_000_000, 0)), "2001-09-09T01:46:40.000Z");
        // A leap day, and milliseconds.
        assert_eq!(iso8601(at(951_782_400, 5)), "2000-02-29T00:00:00.005Z");
        assert_eq!(iso8601(at(1_790_793_225, 123)), "2026-09-30T18:33:45.123Z");
    }

    #[test]
    fn game_is_the_first_folder_under_the_root() {
        let root = Path::new(r"D:\Clips");
        let game = |p: &str| game_from_path(root, Path::new(p));
        assert_eq!(
            game(r"D:\Clips\Valorant\2026\a.mp4"),
            Some("Valorant".into())
        );
        assert_eq!(game(r"D:\Clips\Valorant\a.mp4"), Some("Valorant".into()));
        assert_eq!(game(r"d:\clips\Valorant\a.mp4"), Some("Valorant".into()));
        assert_eq!(game(r"D:\Clips\a.mp4"), None);
        assert_eq!(game(r"D:\Other\Valorant\a.mp4"), None);
        assert_eq!(game(r"E:\Clips\Valorant\a.mp4"), None);
        assert_eq!(game(r"D:\Clips"), None);
        assert_eq!(
            game_from_path(Path::new(r"D:\Clips\"), Path::new(r"D:\Clips\X\a.mp4")),
            Some("X".into())
        );
    }

    #[test]
    fn generic_handler_names_are_not_titles() {
        for name in [
            "SoundHandler",
            "VideoHandler",
            "Sound Handler",
            "GPAC ISO Audio Handler",
        ] {
            assert!(is_generic_handler_name(name), "{name}");
        }
        for name in ["Game", "Discord", "Other apps", "Microphone", "Mix"] {
            assert!(!is_generic_handler_name(name), "{name}");
        }
    }

    #[test]
    fn probe_scan_and_thumbnail_work_on_a_real_file() {
        let dir = TempDir::new("library");
        let clips = dir.0.join("clips");
        std::fs::create_dir_all(clips.join("Valorant")).unwrap();
        std::fs::create_dir_all(clips.join("Desktop").join("2026")).unwrap();
        let first = clips.join("Valorant").join("one.mp4");
        make_clip(&first, 3);
        // A clip in a subfolder of a subfolder, written later so it sorts first.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let second = clips.join("Desktop").join("2026").join("two.MP4");
        make_clip(&second, 2);
        // Things the scan must ignore.
        std::fs::write(clips.join("half.mp4.part"), b"not a clip").unwrap();
        std::fs::write(clips.join("notes.txt"), b"hello").unwrap();
        std::fs::write(clips.join("broken.mp4"), b"not really an mp4").unwrap();

        let meta = probe(&first).unwrap();
        assert_eq!(meta.file_name, "one.mp4");
        assert_eq!((meta.width, meta.height), (640, 360));
        assert_eq!(meta.codec, "h264");
        assert!((meta.fps - 60.0).abs() < 0.01, "fps {}", meta.fps);
        assert!(
            (meta.duration_secs - 3.0).abs() < 0.1,
            "duration {}",
            meta.duration_secs
        );
        assert_eq!(meta.size_bytes, std::fs::metadata(&first).unwrap().len());
        assert_eq!(
            meta.audio_tracks,
            vec![
                AudioTrackMeta {
                    index: 0,
                    title: "Game".into(),
                    role: MixRole::Separate,
                },
                AudioTrackMeta {
                    index: 1,
                    title: "Discord".into(),
                    role: MixRole::Separate,
                },
            ]
        );
        assert!(meta.created_at.ends_with('Z') && meta.created_at.contains('T'));

        let found = scan(&clips);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].file_name, "two.MP4", "newest first");
        assert_eq!(found[1].file_name, "one.mp4");
        assert_eq!(
            game_from_path(&clips, &found[0].path),
            Some("Desktop".into())
        );
        assert!(scan(&dir.0.join("missing")).is_empty());

        let cache = dir.0.join("cache");
        assert_eq!(cached_thumbnail(&first, &cache), None, "nothing cached yet");
        let thumb = thumbnail(&first, &cache).unwrap();
        assert_eq!(cached_thumbnail(&first, &cache), Some(thumb.clone()));
        assert_eq!(cached_thumbnail(&second, &cache), None);
        assert_eq!(cached_thumbnail(&dir.0.join("missing.mp4"), &cache), None);
        assert_eq!(thumb.extension().unwrap(), "jpg");
        let bytes = std::fs::read(&thumb).unwrap();
        assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "JPEG signature");
        assert!(bytes.len() > 1000);
        // The same file again reuses the cache entry.
        let modified = std::fs::metadata(&thumb).unwrap().modified().unwrap();
        assert_eq!(thumbnail(&first, &cache).unwrap(), thumb);
        assert_eq!(
            std::fs::metadata(&thumb).unwrap().modified().unwrap(),
            modified
        );
        // A different file gets a different entry; nothing else is left in the cache.
        let other = thumbnail(&second, &cache).unwrap();
        assert_ne!(other, thumb);
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 2);
        // A file that is not a clip is an error, not a panic.
        assert!(thumbnail(&clips.join("broken.mp4"), &cache).is_err());
        assert!(probe(&clips.join("broken.mp4")).is_err());
    }

    #[test]
    fn probe_scan_and_thumbnail_can_run_on_many_threads_at_once() {
        let dir = TempDir::new("threads");
        let clips: Vec<PathBuf> = (0..3)
            .map(|i| {
                let path = dir.0.join(format!("clip{i}.mp4"));
                make_clip(&path, 2);
                path
            })
            .collect();
        let cache = dir.0.join("cache");
        std::thread::scope(|scope| {
            for round in 0..12 {
                let (dir, clips, cache) = (&dir.0, &clips, &cache);
                scope.spawn(move || match round % 3 {
                    0 => assert_eq!(scan(dir).len(), 3),
                    1 => {
                        for clip in clips {
                            assert_eq!(probe(clip).unwrap().audio_tracks.len(), 2);
                        }
                    }
                    // Several threads make the same thumbnails at the same moment.
                    _ => {
                        for clip in clips {
                            let thumb = thumbnail(clip, cache).unwrap();
                            assert!(std::fs::metadata(thumb).unwrap().len() > 1000);
                        }
                    }
                });
            }
        });
        let names: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names.len(),
            3,
            "one thumbnail per clip and no leftovers: {names:?}"
        );
        assert!(names.iter().all(|n| n.ends_with(".jpg")));
    }
}
