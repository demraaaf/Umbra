//! The clip library as the UI sees it: listing, thumbnails, rename, delete, reveal, export,
//! albums and per-track audio. Paths from the UI are only trusted after checking they are
//! clips inside the clip folder.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use umbra_core::albums::Album;
use umbra_core::controller::{ClipEvent, ScreenshotEvent};
use umbra_core::export::{CameraBurn, Crop, ExportJob, InputBurn, LogoBurn};
use umbra_core::library::{self, ClipMeta};
use umbra_core::naming;
use umbra_core::render::{Fades, OutputFormat, RenderJob, RenderPart, SpeedRange};
use umbra_core::settings;
use windows::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    SHFileOperationW,
};

use crate::AppState;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrackDto {
    pub index: usize,
    pub title: String,
    pub role: umbra_core::library::MixRole,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClipDto {
    pub path: String,
    pub kind: &'static str,
    pub file_name: String,
    pub game: Option<String>,
    pub created_at: String,
    pub duration_secs: f64,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
    pub thumbnail_url: Option<String>,
    pub video_url: String,
    pub audio_tracks: Vec<AudioTrackDto>,
    /// Average bitrate of the file in bit/s; 0 if unknown.
    pub bit_rate: u64,
    /// How the clip was recorded (see `ClipMeta::details`).
    pub details: std::collections::BTreeMap<String, String>,
    pub albums: Vec<String>,
    /// For a recording: seconds where the clip hotkey was pressed while it recorded.
    pub bookmarks: Vec<f64>,
    /// The webcam recorded next to the screen, if any.
    pub camera: Option<SizeDto>,
    /// Keyboard and mouse input was recorded with it (it has an input file).
    pub input: bool,
    /// A logo shows over it (one whose picture is still there).
    pub logo: bool,
}

#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SizeDto {
    pub width: u32,
    pub height: u32,
}

/// A clip's camera as a file of its own (see `export::split_camera`).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CameraVideoDto {
    pub url: String,
    /// Where the camera was filming, as [start, end] in seconds of the clip. The file's
    /// time 0 is the first one's start.
    pub stretches: Vec<[f64; 2]>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CropRectDto {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrackVolume {
    pub index: usize,
    pub volume: f32,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub path: String,
    pub start_secs: f64,
    pub end_secs: f64,
    pub crop: Option<CropRectDto>,
    pub audio_tracks: Vec<TrackVolume>,
    /// Ranges to leave out, in seconds from the start of the clip.
    #[serde(default)]
    pub cuts: Vec<CutRangeDto>,
    pub overwrite: bool,
    /// The new file's name without `.mp4`; by default the clip's name with "(edit)".
    #[serde(default)]
    pub name: Option<String>,
    /// What a preset asks of the file: a size limit, a codec, a smaller picture, fewer frames.
    #[serde(default)]
    pub target: Option<ExportTargetDto>,
    /// Stretches of this clip at another speed, in its own seconds.
    #[serde(default)]
    pub speeds: Vec<SpeedRange>,
    /// More clips after this one, each with its own edits.
    #[serde(default)]
    pub more: Vec<ExportItem>,
    #[serde(default)]
    pub format: OutputFormat,
    #[serde(default)]
    pub fades: Fades,
    /// Width of a GIF.
    #[serde(default)]
    pub gif_width: Option<u32>,
    /// Cut exactly on the frame, re-encoding the video. Otherwise the start and the cuts move
    /// to the nearest keyframe and the packets are copied, which is far faster.
    #[serde(default)]
    pub exact_cuts: bool,
    #[serde(default)]
    pub camera: Option<CameraBurnDto>,
    #[serde(default)]
    pub logo: Option<LogoBurnDto>,
    /// The clip's keyboard and mouse input drawn in, as the editor shows it.
    #[serde(default)]
    pub input: Option<settings::InputOverlayPrefs>,
}

/// The clip's camera drawn in where the editor placed it, except for `hidden`.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CameraBurnDto {
    pub placement: settings::Placement,
    #[serde(default)]
    pub hidden: Vec<CutRangeDto>,
}

/// A picture drawn in; `path` has to be one of Umbra's logo copies.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LogoBurnDto {
    pub path: String,
    pub placement: settings::Placement,
}

/// The camera and logo of an export as the exporter takes them.
fn burns(
    camera: &Option<CameraBurnDto>,
    logo: &Option<LogoBurnDto>,
) -> Result<(Option<CameraBurn>, Option<LogoBurn>)> {
    let camera = camera.as_ref().map(|c| CameraBurn {
        placement: c.placement.clone(),
        hidden: c
            .hidden
            .iter()
            .map(|r| (r.start_secs, r.end_secs))
            .collect(),
    });
    let logo = match logo {
        Some(l) => {
            let dir =
                std::fs::canonicalize(settings::logos_dir()).context("the logo is missing")?;
            let path = std::fs::canonicalize(&l.path)
                .with_context(|| format!("the logo {} is missing", l.path))?;
            if !path.starts_with(&dir) {
                bail!("that logo is not one of Umbra's");
            }
            Some(LogoBurn {
                path,
                placement: l.placement.clone(),
            })
        }
        None => None,
    };
    Ok((camera, logo))
}

/// A further clip in an export of several, with its own edits.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportItem {
    pub path: String,
    pub start_secs: f64,
    pub end_secs: f64,
    pub crop: Option<CropRectDto>,
    pub audio_tracks: Vec<TrackVolume>,
    #[serde(default)]
    pub cuts: Vec<CutRangeDto>,
    #[serde(default)]
    pub speeds: Vec<SpeedRange>,
    #[serde(default)]
    pub camera: Option<CameraBurnDto>,
    #[serde(default)]
    pub logo: Option<LogoBurnDto>,
    #[serde(default)]
    pub input: Option<settings::InputOverlayPrefs>,
}

/// What an export made: the new clip when it is an MP4 in the library, otherwise just the file.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportResultDto {
    pub clip: Option<ClipDto>,
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportTargetDto {
    pub codec: Option<umbra_core::settings::CodecChoice>,
    /// The file may be at most this big.
    pub max_bytes: Option<u64>,
    /// Video bitrate in kbit/s, when no size limit sets it.
    pub video_kbps: Option<u32>,
    pub max_height: Option<u32>,
    pub max_fps: Option<u32>,
    /// Only the combined track, with the microphone mixed in.
    pub mix_only: bool,
}

/// The highest picture that still looks clean at `kbps` of video.
fn height_for_bitrate(kbps: f64) -> u32 {
    match kbps {
        k if k >= 8000.0 => 1440,
        k if k >= 4000.0 => 1080,
        k if k >= 2000.0 => 720,
        k if k >= 900.0 => 540,
        _ => 360,
    }
}

/// Turns a preset's wishes into the export's video bitrate and size. `size` is the picture
/// after cropping; `secs` the length after trimming and cuts.
fn shape_export(
    target: &ExportTargetDto,
    size: (u32, u32),
    secs: f64,
    audio_tracks: usize,
    audio_kbps: u32,
) -> Result<(Option<i64>, Option<(u32, u32)>)> {
    let mut max_height = target.max_height;
    let bitrate = match target.max_bytes {
        Some(bytes) => {
            // 8 % for the container and for the encoder overshooting a little.
            let audio_bps = audio_tracks as f64 * audio_kbps as f64 * 1000.0;
            let video_bps = bytes as f64 * 8.0 * 0.92 / secs.max(0.1) - audio_bps;
            if video_bps < 150_000.0 {
                let fits = bytes as f64 * 8.0 * 0.92 / (150_000.0 + audio_bps);
                bail!(
                    "this is too long to fit in {:.0} MB; trim it to about {:.0} seconds or less",
                    bytes as f64 / 1e6,
                    fits.floor()
                );
            }
            let ladder = height_for_bitrate(video_bps / 1000.0);
            max_height = Some(max_height.map_or(ladder, |h| h.min(ladder)));
            Some(video_bps as i64)
        }
        None => target.video_kbps.map(|k| k as i64 * 1000),
    };
    let scale = match max_height {
        Some(h) if h < size.1 => {
            let width = (size.0 as u64 * h as u64 / size.1.max(1) as u64) as u32;
            Some(((width / 2 * 2).max(2), h / 2 * 2))
        }
        _ => None,
    };
    Ok((bitrate, scale))
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CutRangeDto {
    pub start_secs: f64,
    pub end_secs: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WaveformsDto {
    pub slice_secs: f64,
    /// Per audio track, one level per slice: 0 is -60 dB or quieter, 255 is full scale.
    pub tracks: Vec<Vec<u8>>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct ExportProgress {
    path: String,
    fraction: f32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrackAudioDto {
    pub index: usize,
    pub url: String,
    /// `track` for a track as recorded; `othersWithoutApps` for "Other apps" with the
    /// apps in `apps` taken out (its `index` is that of "Other apps").
    pub kind: &'static str,
    pub apps: Vec<usize>,
}

pub fn thumbs_dir() -> PathBuf {
    settings::cache_dir().join("thumbs")
}

pub fn tracks_dir() -> PathBuf {
    settings::cache_dir().join("tracks")
}

/// What JavaScript's `encodeURIComponent` produces, which is how Tauri builds asset URLs.
pub(crate) fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Video and audio come from the media server when it runs (see `media`), else the file
/// protocol.
fn media_url(path: &Path) -> String {
    crate::media::url(path).unwrap_or_else(|| asset_url(path))
}

fn asset_url(path: &Path) -> String {
    format!(
        "http://asset.localhost/{}",
        encode_uri_component(&path.to_string_lossy())
    )
}

fn to_dto(app: &AppHandle, meta: &ClipMeta, root: &Path) -> ClipDto {
    let thumbnail = library::cached_thumbnail(&meta.path, &thumbs_dir()).map(|p| asset_url(&p));
    let path = meta.path.to_string_lossy().into_owned();
    ClipDto {
        albums: app.state::<AppState>().albums.albums_of(&path),
        path,
        kind: if meta.is_image { "image" } else { "video" },
        file_name: meta.file_name.clone(),
        game: library::game_from_path(root, &meta.path),
        created_at: meta.created_at.clone(),
        duration_secs: meta.duration_secs,
        size_bytes: meta.size_bytes,
        width: meta.width,
        height: meta.height,
        fps: meta.fps,
        codec: meta.codec.clone(),
        bit_rate: meta.bit_rate,
        details: meta.details.iter().cloned().collect(),
        // Only recordings have bookmarks, which saves reading the list for every clip.
        bookmarks: if meta.details.iter().any(|(k, _)| k == "recording") {
            umbra_core::recordings::get(&meta.path)
                .map(|info| info.bookmarks)
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        camera: meta.camera.map(|(width, height)| SizeDto { width, height }),
        input: umbra_core::inputfile::existing_sidecar(&meta.path).is_some(),
        logo: umbra_core::burnin::logo_of(meta).is_some(),
        thumbnail_url: thumbnail,
        video_url: media_url(&meta.path),
        audio_tracks: meta
            .audio_tracks
            .iter()
            .map(|t| AudioTrackDto {
                index: t.index,
                title: t.title.clone(),
                role: t.role,
            })
            .collect(),
    }
}

/// Clips being given thumbnails right now, so two lists in a row don't start the work twice.
static IN_PROGRESS: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Makes the missing thumbnails on a background thread, then tells the UI to refresh.
fn generate_thumbnails(app: &AppHandle, clips: Vec<PathBuf>) {
    let wanted: Vec<PathBuf> = {
        let mut guard = IN_PROGRESS.lock().unwrap();
        let busy = guard.get_or_insert_with(HashSet::new);
        clips
            .into_iter()
            .filter(|p| busy.insert(p.clone()))
            .collect()
    };
    if wanted.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for (n, path) in wanted.iter().enumerate() {
            if let Err(err) = library::thumbnail(path, &thumbs_dir()) {
                log::warn!("thumbnail for {} failed: {err:#}", path.display());
            }
            if let Some(busy) = IN_PROGRESS.lock().unwrap().as_mut() {
                busy.remove(path);
            }
            // Refresh every few thumbnails so the grid fills in while the work continues.
            if (n + 1) % 6 == 0 || n + 1 == wanted.len() {
                let _ = app.emit("clips-changed", ());
            }
        }
    });
}

pub fn clip_folder(app: &AppHandle) -> PathBuf {
    PathBuf::from(app.state::<AppState>().controller.settings().storage.folder)
}

pub fn list(app: &AppHandle) -> Vec<ClipDto> {
    let root = clip_folder(app);
    let clips = library::scan(&root);
    let dtos: Vec<ClipDto> = clips.iter().map(|meta| to_dto(app, meta, &root)).collect();
    let missing = dtos
        .iter()
        .filter(|d| d.thumbnail_url.is_none())
        .map(|d| PathBuf::from(&d.path))
        .collect();
    generate_thumbnails(app, missing);
    dtos
}

fn dto_for(app: &AppHandle, path: &Path) -> Result<ClipDto> {
    let root = clip_folder(app);
    let meta = library::probe(path)?;
    let dto = to_dto(app, &meta, &root);
    if dto.thumbnail_url.is_none() {
        generate_thumbnails(app, vec![path.to_path_buf()]);
    }
    Ok(dto)
}

fn announce(app: &AppHandle, path: &Path) {
    match dto_for(app, path) {
        Ok(dto) => {
            let _ = app.emit("clip-saved", dto);
            let _ = app.emit("clips-changed", ());
        }
        Err(err) => log::warn!("cannot read the saved file: {err:#}"),
    }
}

pub fn on_clip_saved(app: &AppHandle, saved: ClipEvent) {
    announce(app, &saved.clip.path);
    enforce_limits_later(app);
}

pub fn on_screenshot_saved(app: &AppHandle, saved: ScreenshotEvent) {
    announce(app, &saved.path);
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSavedDto {
    /// The finished file, or none if nothing could be kept.
    pub clip: Option<ClipDto>,
    pub kind: umbra_core::recordings::RecordingKind,
    pub game: String,
    pub error: Option<String>,
}

pub fn on_recording_saved(app: &AppHandle, event: umbra_core::controller::RecordingEvent) {
    let clip = event
        .saved
        .as_ref()
        .and_then(|saved| match dto_for(app, &saved.path) {
            Ok(dto) => Some(dto),
            Err(err) => {
                log::warn!("cannot read the recording: {err:#}");
                None
            }
        });
    let _ = app.emit(
        "recording-saved",
        RecordingSavedDto {
            clip,
            kind: event.kind,
            game: event.game,
            error: event.error,
        },
    );
    let _ = app.emit("clips-changed", ());
    enforce_limits_later(app);
}

/// Checks that `path` is an existing clip or screenshot inside the clip folder and returns
/// its real path.
/// A file in the clip folder, of any kind (an export can be a GIF or an MP3).
pub(crate) fn inside_clip_folder(app: &AppHandle, path: &str) -> Result<PathBuf> {
    let root = std::fs::canonicalize(clip_folder(app)).context("the clip folder does not exist")?;
    let path = std::fs::canonicalize(path).with_context(|| format!("{path} does not exist"))?;
    if !path.starts_with(&root) {
        bail!("that file is not in the clip folder");
    }
    Ok(PathBuf::from(plain(&path)))
}

pub(crate) fn validate(app: &AppHandle, path: &str) -> Result<PathBuf> {
    let root = clip_folder(app);
    let root = std::fs::canonicalize(&root)
        .with_context(|| format!("{} does not exist", root.display()))?;
    let path = std::fs::canonicalize(path).with_context(|| format!("{path} does not exist"))?;
    if !path.starts_with(&root) {
        bail!("that file is not in the clip folder");
    }
    let is_clip = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
    if !is_clip && !library::is_image_path(&path) {
        bail!("that file is not a clip or screenshot");
    }
    // Without the `\\?\` prefix, so paths look the same as in the clip list.
    Ok(PathBuf::from(plain(&path)))
}

/// Paths as the UI knows them: without the `\\?\` prefix `canonicalize` adds.
fn plain(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string()
}

pub fn save(app: &AppHandle) -> Result<ClipDto> {
    let saved = app.state::<AppState>().controller.save_clip()?;
    dto_for(app, &saved.clip.path)
}

pub fn screenshot(app: &AppHandle) -> Result<ClipDto> {
    let saved = app.state::<AppState>().controller.take_screenshot()?;
    dto_for(app, &saved.path)
}

/// Sends files to the recycle bin, so a mistaken delete can be undone.
/// Deletes `paths`: to the Recycle Bin, or for good when the settings say so.
pub(crate) fn remove(app: &AppHandle, paths: &[PathBuf]) -> Result<()> {
    if app
        .state::<AppState>()
        .controller
        .settings()
        .storage
        .skip_recycle_bin
    {
        // Every file is tried, so one that is open somewhere does not keep the rest.
        let mut first_error = None;
        for path in paths {
            if let Err(err) = std::fs::remove_file(path) {
                first_error.get_or_insert(anyhow!(err).context(format!("cannot delete {}", path.display())));
            }
        }
        first_error.map_or(Ok(()), Err)
    } else {
        recycle(paths)
    }
}

fn recycle(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    // SHFileOperation wants a list of paths, each ended by a zero, and one more zero at the end.
    let mut wide: Vec<u16> = Vec::new();
    for path in paths {
        wide.extend(plain(path).encode_utf16());
        wide.push(0);
    }
    wide.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: windows::core::PCWSTR(wide.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 || op.fAnyOperationsAborted.as_bool() {
        bail!("could not move the files to the recycle bin (code {code})");
    }
    Ok(())
}

pub fn delete_many(app: &AppHandle, paths: &[String]) -> Result<()> {
    let checked: Vec<PathBuf> = paths
        .iter()
        .map(|p| validate(app, p))
        .collect::<Result<_>>()?;
    let removed = remove(app, &umbra_core::inputfile::with_sidecars(&checked));
    // When one cannot go (open in a player), the others that did still leave albums and the list.
    let (deleted, raw): (Vec<&PathBuf>, Vec<&String>) = checked
        .iter()
        .zip(paths)
        .filter(|(p, _)| !p.exists())
        .unzip();
    for path in &deleted {
        umbra_core::recordings::remove(path);
    }
    let gone: Vec<String> = deleted
        .iter()
        .map(|p| plain(p))
        .chain(raw.into_iter().cloned())
        .collect();
    if !gone.is_empty() {
        if app.state::<AppState>().albums.paths_deleted(&gone)? {
            let _ = app.emit("albums-changed", ());
        }
        let _ = app.emit("clips-changed", ());
    }
    removed
}

pub fn rename(app: &AppHandle, path: &str, new_name: &str) -> Result<ClipDto> {
    let path = validate(app, path)?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_lowercase();
    let stem = new_name.trim();
    let stem = stem
        .strip_suffix(&format!(".{extension}"))
        .or_else(|| stem.strip_suffix(&format!(".{}", extension.to_uppercase())))
        .unwrap_or(stem);
    let stem = naming::sanitize_segment(stem);
    let target = path.with_file_name(format!("{stem}.{extension}"));
    if target == path {
        return dto_for(app, &path);
    }
    // Windows names ignore case, so "clip" to "Clip" finds the file itself, not another one.
    let same_file = plain(&target).to_lowercase() == plain(&path).to_lowercase();
    if target.exists() && !same_file {
        bail!("a file named \"{stem}.{extension}\" already exists");
    }
    std::fs::rename(&path, &target).context("cannot rename the file")?;
    if let Some(input) = umbra_core::inputfile::existing_sidecar(&path) {
        let moved = umbra_core::inputfile::sidecar(&target);
        if let Err(err) = std::fs::rename(&input, &moved) {
            log::warn!(
                "the input of {} did not follow its rename: {err}",
                target.display()
            );
        }
    }
    umbra_core::recordings::rename(&path, &target);
    if app
        .state::<AppState>()
        .albums
        .path_renamed(&plain(&path), &plain(&target))?
    {
        let _ = app.emit("albums-changed", ());
    }
    let _ = app.emit("clips-changed", ());
    dto_for(app, &target)
}

pub fn reveal(app: &AppHandle, path: &str) -> Result<()> {
    let path = inside_clip_folder(app, path)?;
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", plain(&path)))
        .spawn()
        .context("cannot open Explorer")?;
    Ok(())
}

pub fn open_settings_folder() -> Result<()> {
    let folder = settings::data_dir();
    std::fs::create_dir_all(&folder)?;
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("\"{}\"", folder.display()))
        .spawn()
        .context("cannot open Explorer")?;
    Ok(())
}

pub fn track_audio(app: &AppHandle, path: &str) -> Result<Vec<TrackAudioDto>> {
    let path = validate(app, path)?;
    let split = umbra_core::export::split_audio_tracks(&path, &tracks_dir())?;
    let mut list: Vec<TrackAudioDto> = split
        .tracks
        .into_iter()
        .map(|(index, file)| TrackAudioDto {
            index,
            url: media_url(&file),
            kind: "track",
            apps: Vec::new(),
        })
        .collect();
    if let Some((index, file, apps)) = split.others_without_apps {
        list.push(TrackAudioDto {
            index,
            url: media_url(&file),
            kind: "othersWithoutApps",
            apps,
        });
    }
    Ok(list)
}

pub fn camera_video(app: &AppHandle, path: &str) -> Result<Option<CameraVideoDto>> {
    let path = validate(app, path)?;
    Ok(
        umbra_core::export::split_camera(&path, &tracks_dir())?.map(|track| CameraVideoDto {
            url: media_url(&track.path),
            stretches: track.stretches.iter().map(|&(a, b)| [a, b]).collect(),
        }),
    )
}

pub fn waveforms(app: &AppHandle, path: &str) -> Result<WaveformsDto> {
    let path = validate(app, path)?;
    let w = umbra_core::waveform::waveforms(&path)?;
    Ok(WaveformsDto {
        slice_secs: w.slice_secs,
        tracks: w.tracks,
    })
}

/// Exports an edit. One MP4 of one clip goes the direct way, which copies what it can; speed
/// changes, fades, several clips or another format are rendered from per-clip exports.
pub fn export(app: &AppHandle, request: ExportRequest) -> Result<ExportResultDto> {
    let fades = request.fades;
    let fading = fades.fade_in > 0.0
        || fades.fade_out > 0.0
        || (fades.between > 0.0 && !request.more.is_empty());
    let rendered = !request.more.is_empty()
        || request.speeds.iter().any(|s| (s.speed - 1.0).abs() > 1e-6)
        || fading
        || request.format != OutputFormat::Mp4;
    if rendered {
        return render_export(app, request);
    }
    let clip = export_direct(app, request)?;
    Ok(ExportResultDto {
        path: clip.path.clone(),
        file_name: clip.file_name.clone(),
        size_bytes: clip.size_bytes,
        clip: Some(clip),
    })
}

fn codec_of(codec: umbra_core::settings::CodecChoice) -> umbra_core::encode::VideoCodec {
    match codec {
        umbra_core::settings::CodecChoice::H264 => umbra_core::encode::VideoCodec::H264,
        umbra_core::settings::CodecChoice::Hevc => umbra_core::encode::VideoCodec::Hevc,
        umbra_core::settings::CodecChoice::Av1 => umbra_core::encode::VideoCodec::Av1,
    }
}

/// A clip's edits as the exporter takes them, writing to `output` with the tracks mixed into
/// one, as the renderer wants its parts.
fn part_job(app: &AppHandle, input: &Path, output: &Path, item: &ExportItem) -> Result<ExportJob> {
    let (camera, logo) = burns(&item.camera, &item.logo)?;
    let kept: Vec<TrackVolume> = item
        .audio_tracks
        .iter()
        .copied()
        .filter(|t| t.volume > 0.0)
        .collect();
    Ok(ExportJob {
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        start_secs: item.start_secs,
        end_secs: item.end_secs,
        crop: item.crop.as_ref().map(|c| Crop {
            x: c.x,
            y: c.y,
            width: c.width,
            height: c.height,
        }),
        audio_tracks: kept.iter().map(|t| t.index).collect(),
        audio_gains: kept
            .iter()
            .filter(|t| (t.volume - 1.0).abs() > 0.001)
            .map(|t| (t.index, t.volume.min(4.0)))
            .collect(),
        cuts: item
            .cuts
            .iter()
            .map(|c| (c.start_secs, c.end_secs))
            .collect(),
        keyframe_cuts: false,
        force_reencode: false,
        encoder: None,
        threads: app.state::<AppState>().controller.settings().export.threads,
        scale: None,
        max_fps: None,
        video_bitrate: None,
        codec: None,
        mix_only: true,
        quick: false,
        camera,
        logo,
        input_overlay: item.input.clone().map(|overlay| InputBurn { overlay }),
        keep_camera: false,
        keep_logo: false,
    })
}

/// Speed ranges in a clip's own seconds, moved to the time of its export: from the trim start,
/// with the parts cut out taken away.
fn speeds_after_cuts(item: &ExportItem) -> Vec<SpeedRange> {
    let cuts: Vec<(f64, f64)> = item
        .cuts
        .iter()
        .map(|c| {
            (
                c.start_secs.max(item.start_secs),
                c.end_secs.min(item.end_secs),
            )
        })
        .filter(|(a, b)| b > a)
        .collect();
    let map = |t: f64| {
        let t = t.clamp(item.start_secs, item.end_secs);
        let removed: f64 = cuts.iter().map(|&(a, b)| (t.min(b) - a).max(0.0)).sum();
        t - item.start_secs - removed
    };
    item.speeds
        .iter()
        .map(|s| SpeedRange {
            start: map(s.start),
            end: map(s.end),
            speed: s.speed,
        })
        .filter(|s| s.end - s.start > 0.001)
        .collect()
}

/// A folder in the cache for one render's intermediate files; removed when dropped.
struct RenderDir(PathBuf);

impl Drop for RenderDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn render_export(app: &AppHandle, request: ExportRequest) -> Result<ExportResultDto> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static RENDERS: AtomicU32 = AtomicU32::new(0);

    let input = validate(app, &request.path)?;
    if request.overwrite && (request.format != OutputFormat::Mp4 || !request.more.is_empty()) {
        bail!(
            "replacing the original works for an MP4 of this one clip; save it as a new file instead"
        );
    }
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip")
        .to_string();
    let first = ExportItem {
        path: request.path.clone(),
        start_secs: request.start_secs,
        end_secs: request.end_secs,
        crop: request.crop.clone(),
        audio_tracks: request.audio_tracks.clone(),
        cuts: request.cuts.clone(),
        speeds: request.speeds.clone(),
        camera: request.camera.clone(),
        logo: request.logo.clone(),
        input: request.input.clone(),
    };
    let items: Vec<ExportItem> = std::iter::once(first)
        .chain(request.more.iter().cloned())
        .collect();
    let inputs: Vec<PathBuf> = items
        .iter()
        .map(|i| validate(app, &i.path))
        .collect::<Result<_>>()?;

    let dir = RenderDir(settings::cache_dir().join(format!(
        "render-{}-{}",
        std::process::id(),
        RENDERS.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(&dir.0).context("cannot make a folder for the render")?;

    let progress_app = app.clone();
    let progress_path = request.path.clone();
    let mut last = -1.0f32;
    let mut report = move |fraction: f32| {
        if fraction - last >= 0.01 || fraction >= 1.0 {
            last = fraction;
            let _ = progress_app.emit(
                "export-progress",
                ExportProgress {
                    path: progress_path.clone(),
                    fraction,
                },
            );
        }
        true
    };

    // Each clip with its edits, mixed into one track: the first half of the progress.
    let count = items.len() as f32;
    let mut parts = Vec::new();
    for (i, (item, input)) in items.iter().zip(&inputs).enumerate() {
        let out = dir.0.join(format!("part{i}.mp4"));
        let job = part_job(app, input, &out, item)?;
        umbra_core::export::export(&job, &mut |f| report((i as f32 + f) / count * 0.5))
            .map_err(|e| anyhow!("export failed: {e:#}"))?;
        parts.push(RenderPart {
            path: out,
            speeds: speeds_after_cuts(item),
        });
    }

    // What the preset or Custom asks of the file.
    let target = request.target.clone().unwrap_or_default();
    let audio_kbps = app
        .state::<AppState>()
        .controller
        .settings()
        .audio
        .bitrate_kbps;
    let first_meta = library::probe(&parts[0].path)?;
    let size = target.max_height.and_then(|h| {
        (h < first_meta.height).then(|| {
            let width =
                (first_meta.width as u64 * h as u64 / first_meta.height.max(1) as u64) as u32;
            ((width / 2 * 2).max(2), h / 2 * 2)
        })
    });
    let video_bitrate = match target.max_bytes {
        Some(bytes) => {
            let secs: f64 = parts
                .iter()
                .map(|p| {
                    let length = library::probe(&p.path).map_or(0.0, |m| m.duration_secs);
                    umbra_core::render::TimeMap::new(&p.speeds).out_of(length)
                })
                .sum::<f64>()
                .max(0.1);
            let video_bps = bytes as f64 * 8.0 * 0.92 / secs - audio_kbps as f64 * 1000.0;
            if video_bps < 150_000.0 {
                bail!("this is too long to fit in {:.0} MB", bytes as f64 / 1e6);
            }
            Some(video_bps as i64)
        }
        None => target.video_kbps.map(|k| k as i64 * 1000),
    };
    let wanted = request
        .name
        .as_deref()
        .map(file_stem_from)
        .filter(|n| !n.is_empty() && !request.overwrite)
        .unwrap_or_else(|| format!("{stem} (edit)"));
    let output = unique_sibling_ext(&input, &wanted, request.format.extension());
    let job = RenderJob {
        parts,
        output: output.clone(),
        format: request.format,
        codec: target.codec.map(codec_of),
        size,
        fps: target.max_fps,
        video_bitrate,
        audio_kbps: audio_kbps.max(128),
        gif_width: request.gif_width.unwrap_or(480),
        fades: request.fades,
        threads: app.state::<AppState>().controller.settings().export.threads,
    };
    umbra_core::render::render(&job, &mut |f| report(0.5 + f * 0.5))
        .map_err(|e| anyhow!("export failed: {e:#}"))?;

    let result = if request.overwrite {
        // The edit draws the input in or leaves it out, so the original's goes with it.
        remove(
            app,
            &umbra_core::inputfile::with_sidecars(std::slice::from_ref(&input)),
        )?;
        std::fs::rename(&output, &input).context("cannot put the edited clip in place")?;
        input
    } else {
        output
    };
    let _ = app.emit("clips-changed", ());
    let clip = if request.format == OutputFormat::Mp4 {
        Some(dto_for(app, &result)?)
    } else {
        None
    };
    Ok(ExportResultDto {
        path: plain(&result),
        file_name: result
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size_bytes: std::fs::metadata(&result).map_or(0, |m| m.len()),
        clip,
    })
}

fn export_direct(app: &AppHandle, request: ExportRequest) -> Result<ClipDto> {
    let input = validate(app, &request.path)?;
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip")
        .to_string();

    // Always write to a new name first; replacing the original happens only after it worked.
    let wanted = request
        .name
        .as_deref()
        .map(file_stem_from)
        .filter(|n| !n.is_empty() && !request.overwrite)
        .unwrap_or_else(|| format!("{stem} (edit)"));
    let edited = unique_sibling(&input, &wanted);
    let kept: Vec<TrackVolume> = request
        .audio_tracks
        .iter()
        .copied()
        .filter(|t| t.volume > 0.0)
        .collect();
    // A preset's size limit, codec, picture size and frame rate.
    let target = request.target.clone().unwrap_or_default();
    let (video_bitrate, scale) = if request.target.is_some() {
        let meta = library::probe(&input)?;
        let size = request
            .crop
            .as_ref()
            .map_or((meta.width, meta.height), |c| (c.width, c.height));
        let cut_secs: f64 = request
            .cuts
            .iter()
            .map(|c| {
                (c.end_secs.min(request.end_secs) - c.start_secs.max(request.start_secs)).max(0.0)
            })
            .sum();
        let secs = (request.end_secs - request.start_secs - cut_secs).max(0.1);
        let tracks = if target.mix_only { 1 } else { kept.len() };
        let audio_kbps = app
            .state::<AppState>()
            .controller
            .settings()
            .audio
            .bitrate_kbps;
        shape_export(&target, size, secs, tracks, audio_kbps)?
    } else {
        (None, None)
    };
    let (camera, logo) = burns(&request.camera, &request.logo)?;
    let job = ExportJob {
        input: input.clone(),
        output: edited.clone(),
        start_secs: request.start_secs,
        end_secs: request.end_secs,
        crop: request.crop.as_ref().map(|c| Crop {
            x: c.x,
            y: c.y,
            width: c.width,
            height: c.height,
        }),
        audio_tracks: kept.iter().map(|t| t.index).collect(),
        audio_gains: kept
            .iter()
            .filter(|t| (t.volume - 1.0).abs() > 0.001)
            .map(|t| (t.index, t.volume.min(4.0)))
            .collect(),
        cuts: request
            .cuts
            .iter()
            .map(|c| (c.start_secs, c.end_secs))
            .collect(),
        keyframe_cuts: !request.exact_cuts,
        force_reencode: false,
        encoder: None,
        threads: app.state::<AppState>().controller.settings().export.threads,
        scale,
        max_fps: target.max_fps,
        video_bitrate,
        codec: target.codec.map(|c| match c {
            umbra_core::settings::CodecChoice::H264 => umbra_core::encode::VideoCodec::H264,
            umbra_core::settings::CodecChoice::Hevc => umbra_core::encode::VideoCodec::Hevc,
            umbra_core::settings::CodecChoice::Av1 => umbra_core::encode::VideoCodec::Av1,
        }),
        mix_only: target.mix_only,
        quick: false,
        camera,
        logo,
        input_overlay: request.input.clone().map(|overlay| InputBurn { overlay }),
        keep_camera: false,
        keep_logo: false,
    };

    let progress_app = app.clone();
    let progress_path = request.path.clone();
    let mut last = -1.0f32;
    umbra_core::export::export(&job, &mut |fraction| {
        // A few updates per second are plenty for a progress bar.
        if fraction - last >= 0.01 || fraction >= 1.0 {
            last = fraction;
            let _ = progress_app.emit(
                "export-progress",
                ExportProgress {
                    path: progress_path.clone(),
                    fraction,
                },
            );
        }
        true
    })
    .map_err(|e| anyhow!("export failed: {e:#}"))?;

    let result = if request.overwrite {
        // Keep the original recoverable in the recycle bin (unless the settings say to skip
        // it), then take its name. Albums keep pointing at the same path. The edit draws the
        // input in or leaves it out, so the original's input goes with it.
        remove(
            app,
            &umbra_core::inputfile::with_sidecars(std::slice::from_ref(&input)),
        )?;
        std::fs::rename(&edited, &input).context("cannot put the edited clip in place")?;
        input
    } else {
        edited
    };
    let _ = app.emit("clips-changed", ());
    dto_for(app, &result)
}

/// A name typed for a file, made safe for Windows: no path parts or reserved characters, no
/// `.mp4` (it is added), no trailing dots or spaces.
fn file_stem_from(name: &str) -> String {
    let name = name.trim();
    let name = name
        .strip_suffix(".mp4")
        .or_else(|| name.strip_suffix(".MP4"))
        .unwrap_or(name);
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    cleaned
        .trim_end_matches(['.', ' '])
        .chars()
        .take(150)
        .collect()
}

/// Puts clips on the clipboard as files, ready to paste into Discord or a folder.
pub fn copy_to_clipboard(app: &AppHandle, paths: &[String]) -> Result<()> {
    // Any file the editor wrote into the clip folder: a GIF or an MP3 copies just as well.
    let files: Vec<PathBuf> = paths
        .iter()
        .map(|p| inside_clip_folder(app, p))
        .collect::<Result<_>>()?;
    let copies = settings::cache_dir().join("copies");
    // Only the latest copy is on the clipboard; earlier ones can go.
    let _ = std::fs::remove_dir_all(&copies);
    let files: Vec<PathBuf> = files
        .iter()
        .enumerate()
        .map(|(i, f)| with_overlays_drawn(app, f, &copies.join(i.to_string())))
        .collect::<Result<_>>()?;
    let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    crate::clipboard::copy_files(&refs)
}

/// A clip with a camera or logo exported with them drawn in, into `dir`, so the copy plays as
/// Umbra shows it; anything else as it is.
fn with_overlays_drawn(app: &AppHandle, path: &Path, dir: &Path) -> Result<PathBuf> {
    let Ok(meta) = library::probe(path) else {
        return Ok(path.to_path_buf());
    };
    let pictures = umbra_core::burnin::Layers {
        camera: true,
        logo: true,
        input: false,
    }
    .and(umbra_core::burnin::layers(&meta));
    if !pictures.any() {
        return Ok(path.to_path_buf());
    }
    let output = dir.join(path.file_name().context("the clip has no name")?);
    let settings = app.state::<AppState>().controller.settings();
    let job = umbra_core::burnin::job(&meta, pictures, &settings, &output);
    umbra_core::export::export(&job, &mut |_| true)
        .map_err(|e| anyhow!("cannot draw the camera and logo in: {e:#}"))?;
    Ok(output)
}

fn unique_sibling(path: &Path, stem: &str) -> PathBuf {
    unique_sibling_ext(path, stem, "mp4")
}

fn unique_sibling_ext(path: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = path.with_file_name(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| path.with_file_name(format!("{stem} {n}.{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

// ---------- Albums ----------

fn albums_changed(app: &AppHandle) {
    let _ = app.emit("albums-changed", ());
    // Clip cards show which albums they are in.
    let _ = app.emit("clips-changed", ());
}

pub fn list_albums(app: &AppHandle) -> Vec<Album> {
    app.state::<AppState>().albums.list()
}

pub fn album_change(
    app: &AppHandle,
    change: impl FnOnce(&umbra_core::albums::AlbumStore) -> Result<Album>,
) -> Result<Album> {
    let album = change(&app.state::<AppState>().albums)?;
    albums_changed(app);
    Ok(album)
}

pub fn delete_album(app: &AppHandle, id: &str) -> Result<()> {
    app.state::<AppState>().albums.delete(id)?;
    albums_changed(app);
    Ok(())
}

/// Clip paths from the UI, checked to be inside the clip folder, in the form the UI uses.
pub fn checked_paths(app: &AppHandle, paths: &[String]) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|p| validate(app, p).map(|real| plain(&real)))
        .collect()
}

// ---------- Storage limits ----------

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageDto {
    pub bytes: u64,
    pub files: usize,
    /// Free on the clip folder's drive.
    pub free_bytes: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanupDto {
    pub files: usize,
    pub bytes: u64,
}

/// How much the clip folder holds.
pub fn storage_usage(app: &AppHandle) -> StorageUsageDto {
    let files = umbra_core::cleanup::library_files(&clip_folder(app));
    StorageUsageDto {
        bytes: files.iter().map(|f| f.bytes).sum(),
        files: files.len(),
        free_bytes: umbra_core::disk::free_bytes(&clip_folder(app)).unwrap_or(0),
    }
}

/// One cleanup at a time; a second request while one runs is dropped.
static CLEANING: Mutex<()> = Mutex::new(());

/// Deletes the oldest clips the storage limits say must go, the way deleting is set up
/// (Recycle Bin or for good). Favourites, and clips in albums when that is set, are kept.
pub fn enforce_limits(app: &AppHandle) -> Result<CleanupDto> {
    let Ok(_guard) = CLEANING.try_lock() else {
        return Ok(CleanupDto::default());
    };
    let settings = app.state::<AppState>().controller.settings();
    let storage = &settings.storage;
    let limits = umbra_core::cleanup::Limits {
        max_bytes: (storage.limit_gb > 0).then(|| storage.limit_gb as u64 * (1u64 << 30)),
        max_age: (storage.max_age_days > 0)
            .then(|| std::time::Duration::from_secs(storage.max_age_days as u64 * 86_400)),
    };
    if limits.max_bytes.is_none() && limits.max_age.is_none() {
        return Ok(CleanupDto::default());
    }
    let files = umbra_core::cleanup::library_files(&clip_folder(app));
    let albums = &app.state::<AppState>().albums;
    let keep_albums = storage.keep_albums;
    let keep = |path: &Path| {
        let in_albums = albums.albums_of(&plain(path));
        in_albums
            .iter()
            .any(|id| id == umbra_core::albums::FAVORITES)
            || (keep_albums && !in_albums.is_empty())
    };
    let gone = umbra_core::cleanup::plan(&files, &keep, limits, std::time::SystemTime::now());
    if gone.is_empty() {
        return Ok(CleanupDto::default());
    }
    let removed = remove(app, &umbra_core::inputfile::with_sidecars(&gone));
    // Some can go even when others cannot (one open in a player): those still leave albums.
    let gone: Vec<PathBuf> = gone.into_iter().filter(|p| !p.exists()).collect();
    if let Err(err) = &removed {
        log::warn!("storage limits could not remove every file: {err:#}");
        if gone.is_empty() {
            return removed.map(|_| CleanupDto::default());
        }
    }
    let bytes: u64 = files
        .iter()
        .filter(|f| gone.contains(&f.path))
        .map(|f| f.bytes)
        .sum();
    for path in &gone {
        umbra_core::recordings::remove(path);
    }
    let names: Vec<String> = gone.iter().map(|p| plain(p)).collect();
    if albums.paths_deleted(&names)? {
        let _ = app.emit("albums-changed", ());
    }
    log::info!(
        "storage limits removed {} files ({bytes} bytes)",
        gone.len()
    );
    let report = CleanupDto {
        files: gone.len(),
        bytes,
    };
    let _ = app.emit("storage-cleaned", report.clone());
    let _ = app.emit("clips-changed", ());
    Ok(report)
}

/// Runs the storage limits in the background.
pub fn enforce_limits_later(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(err) = enforce_limits(&app) {
            log::warn!("storage cleanup failed: {err:#}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_limit_sets_the_bitrate_and_a_fitting_picture() {
        let discord = ExportTargetDto {
            max_bytes: Some(10_000_000),
            mix_only: true,
            ..Default::default()
        };
        // 30 s in 10 MB: about 2.3 Mbit/s of video, which suits 720p.
        let (bitrate, scale) = shape_export(&discord, (2560, 1440), 30.0, 1, 160).unwrap();
        let kbps = bitrate.unwrap() / 1000;
        assert!((2200..2400).contains(&kbps), "{kbps}");
        assert_eq!(scale, Some((1280, 720)));
        // Shorter clips keep more of their picture: 1080p at 15 s, all of it at 8 s.
        let (_, scale) = shape_export(&discord, (2560, 1440), 15.0, 1, 160).unwrap();
        assert_eq!(scale, Some((1920, 1080)));
        let (_, scale) = shape_export(&discord, (2560, 1440), 8.0, 1, 160).unwrap();
        assert_eq!(scale, None);
        // Five minutes cannot fit; the message says how long can.
        let err = shape_export(&discord, (2560, 1440), 300.0, 1, 160).unwrap_err();
        assert!(err.to_string().contains("trim it to about"), "{err}");
    }

    #[test]
    fn a_height_cap_keeps_the_shape_and_never_upscales() {
        let youtube = ExportTargetDto {
            max_height: Some(1440),
            video_kbps: Some(40_000),
            ..Default::default()
        };
        assert_eq!(
            shape_export(&youtube, (3840, 2160), 10.0, 1, 160).unwrap(),
            (Some(40_000_000), Some((2560, 1440)))
        );
        assert_eq!(
            shape_export(&youtube, (1920, 1080), 10.0, 1, 160)
                .unwrap()
                .1,
            None
        );
        // A vertical crop: the width follows.
        let phone = ExportTargetDto {
            max_height: Some(1920),
            ..Default::default()
        };
        assert_eq!(
            shape_export(&phone, (1216, 2160), 10.0, 1, 160).unwrap().1,
            Some((1080, 1920))
        );
    }
}
