//! The always-on recorder: captures the screen, encodes it and keeps the last N seconds
//! in a replay buffer. Saving a clip copies packets out of the buffer and never re-encodes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};

use crate::Packet;
use crate::aac::AacEncoder;
use crate::align::{CHUNK, SAMPLE_RATE};
use crate::audio::{AudioSource, TrackWorker};
use crate::buffer::ReplayBuffer;
use crate::capture::{Capture, CaptureTarget};
use crate::clock::Clock;
use crate::d3d::Gpu;
use crate::encode::{
    EncoderTuning, RateControl, StreamInfo, VideoCodec, VideoEncoder, VideoSettings,
};
use crate::mux::Mp4Writer;
use crate::scale::GpuScaler;
use crate::settings::Scaling;
use crate::spill::{Held, Spill};
use crate::timer::Ticker;

#[derive(Clone, Debug)]
pub struct AudioTrackConfig {
    /// Track name shown in players, for example `Game` or `Microphone`.
    pub title: String,
    pub source: AudioSource,
    /// Whether this track is also summed into the mixed track.
    pub in_mix: bool,
    /// When set, the track is silent while the flag is false (push-to-talk).
    pub gate: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Debug)]
pub struct AudioConfig {
    pub tracks: Vec<AudioTrackConfig>,
    /// Adds a first track with every `in_mix` track summed together. Most players only
    /// play the first track, so this is what plays when the clip is opened or shared.
    pub mix: bool,
    /// Bitrate of each audio track, in kbit/s.
    pub bitrate_kbps: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            mix: false,
            bitrate_kbps: 160,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecorderConfig {
    pub target: CaptureTarget,
    pub codec: VideoCodec,
    pub fps: u32,
    pub rate: RateControl,
    pub preset: u8,
    pub capture_cursor: bool,
    /// Size of the recorded video. `None` records at the size of the captured surface;
    /// anything else is scaled on the GPU. Both values must be even.
    pub output_size: Option<(u32, u32)>,
    /// How a picture of another shape than `output_size` is fitted into it.
    pub scaling: Scaling,
    pub keyframe_secs: u32,
    pub tuning: EncoderTuning,
    /// Leave out frames of a still screen instead of encoding the last picture again: the
    /// video's frame rate then varies, and the encoder rests while nothing moves.
    pub skip_still: bool,
    /// Length of the replay buffer, and the shortest clip a save produces.
    pub buffer_secs: u32,
    /// Keep the replay buffer in files in this folder instead of in memory.
    pub buffer_dir: Option<PathBuf>,
    /// The screen is in HDR: capture it as such and convert it like this.
    pub hdr: Option<(crate::hdr::HdrInfo, crate::hdr::HdrOutput)>,
    pub audio: AudioConfig,
    /// A webcam, recorded as a video stream of its own after the audio.
    pub camera: Option<crate::camera::CameraConfig>,
    /// Keyboard and mouse input, kept in the buffer with the rest while it is captured.
    pub input: Option<Arc<crate::input::InputHub>>,
}

#[derive(Clone, Debug)]
pub struct RecorderInfo {
    pub adapter: String,
    pub encoder: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// How an HDR screen is recorded, when it is one.
    pub hdr: Option<crate::hdr::HdrOutput>,
    /// Audio tracks in file order, after the video stream.
    pub audio_tracks: Vec<String>,
    /// Tracks whose source could not be opened and are recorded as silence.
    pub audio_problems: Vec<(String, String)>,
    /// Size and frame rate of the camera stream, the last one in the file.
    pub camera: Option<(u32, u32, u32)>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Seconds of footage in the buffer right now.
    pub buffer_secs: f64,
    pub buffer_bytes: u64,
    /// The buffer is on disk; `buffer_bytes` is disk space, not memory.
    pub buffer_on_disk: bool,
    /// Frame slots the recorder was too slow to fill.
    pub dropped_frames: u64,
    pub frames_encoded: u64,
}

#[derive(Clone, Debug)]
pub struct SavedClip {
    pub path: PathBuf,
    pub duration_secs: f64,
    pub bytes: u64,
    /// How long writing the file took.
    pub write_secs: f64,
}

struct Shared {
    buffer: Mutex<ReplayBuffer<Held>>,
    /// Where packets go when the buffer is on disk.
    spill: Option<Spill>,
    /// Codec parameters are only Send, so they sit behind a lock to make `Shared` Sync.
    streams: Mutex<Vec<StreamInfo>>,
    stop: AtomicBool,
    dropped: AtomicU64,
    encoded: AtomicU64,
    /// Times the capture was started again after it stopped delivering frames.
    restarts: AtomicU64,
    error: Mutex<Option<String>>,
}

pub struct Recorder {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// Each audio track's capture, with the track's name.
    workers: Vec<(String, TrackWorker)>,
    /// The camera's stream index, and its worker.
    camera: Option<(usize, crate::camera::CameraWorker)>,
    /// Why the camera is not recorded at all (its encoder could not be made).
    camera_error: Option<String>,
    info: RecorderInfo,
    /// The clock packets are stamped on.
    clock: Clock,
    /// Input from the hub, while subscribed.
    _input: Option<crate::input::Subscription>,
}

/// Sums the tracks that belong in the mix and encodes the result as one more track.
struct Mixer {
    expected: usize,
    pending: std::collections::BTreeMap<u64, (Vec<f32>, usize)>,
    encoder: AacEncoder,
    stream: usize,
    shared: Arc<Shared>,
}

impl Mixer {
    fn add(&mut self, index: u64, samples: &[f32]) {
        let entry = self
            .pending
            .entry(index)
            .or_insert_with(|| (vec![0.0; CHUNK * 2], 0));
        for (sum, sample) in entry.0.iter_mut().zip(samples) {
            *sum += sample;
        }
        entry.1 += 1;
        if entry.1 < self.expected {
            return;
        }
        let (mut mixed, _) = self.pending.remove(&index).unwrap();
        for sample in &mut mixed {
            *sample = sample.clamp(-1.0, 1.0);
        }
        // A track that stopped delivering must not make the map grow without end.
        while self.pending.len() > 200 {
            self.pending.pop_first();
        }
        push_audio(&self.shared, self.stream, &mut self.encoder, index, &mixed);
    }
}

impl Shared {
    /// The packet as the buffer keeps it: written to disk when the buffer is there. If the
    /// disk fails (full, removed), it stays in memory rather than being lost.
    fn hold(&self, packet: Packet) -> Held {
        match &self.spill {
            Some(spill) => match spill.store(&packet) {
                Ok(stored) => Held::Disk(stored),
                Err(err) => {
                    static WARNED: AtomicBool = AtomicBool::new(false);
                    if !WARNED.swap(true, Ordering::Relaxed) {
                        log::warn!("replay buffer kept in memory: {err:#}");
                    }
                    Held::Memory(packet)
                }
            },
            None => Held::Memory(packet),
        }
    }
}

fn push_audio(
    shared: &Shared,
    stream: usize,
    encoder: &mut AacEncoder,
    index: u64,
    samples: &[f32],
) {
    let mut sink = |packet: Packet| {
        let time = packet.pts().unwrap_or(0) as f64 / SAMPLE_RATE as f64;
        let size = packet.size();
        let held = shared.hold(packet);
        shared
            .buffer
            .lock()
            .unwrap()
            .push(stream, time, size, false, held);
    };
    if let Err(err) = encoder.encode(index, samples, &mut sink) {
        log::warn!("audio encode failed: {err:#}");
    }
}

impl Recorder {
    pub fn start(config: RecorderConfig) -> Result<Self> {
        if config.fps == 0 || config.fps > 1000 {
            bail!("frame rate must be between 1 and 1000");
        }
        let clock = Clock::new();
        let gpu = Gpu::new()?;
        let capture = Capture::start(
            &gpu,
            config.target,
            config.capture_cursor,
            config.fps,
            config.hdr.is_some(),
        )?;
        // Encoders need even sizes. A window can have an odd one, and the extra row or
        // column is cropped off rather than refusing to record.
        let (out_width, out_height) = config
            .output_size
            .unwrap_or((capture.width & !1, capture.height & !1));
        if out_width % 2 != 0 || out_height % 2 != 0 || out_width == 0 || out_height == 0 {
            bail!("output size {out_width}x{out_height} must be even and not zero");
        }
        // An HDR screen is converted (and scaled) by a shader instead of the scaler. HDR10
        // needs a card that renders to 10-bit video; without, it is recorded as SDR.
        let converter = match config.hdr {
            Some((info, output)) => {
                let fit = config.scaling == Scaling::Fit;
                let size = (out_width, out_height);
                Some(
                    match crate::hdr::HdrConverter::new(&gpu, output, info, size, fit) {
                        Ok(c) => c,
                        Err(err) if output == crate::hdr::HdrOutput::Hdr10 => {
                            log::warn!("recording HDR as SDR: {err:#}");
                            crate::hdr::HdrConverter::new(
                                &gpu,
                                crate::hdr::HdrOutput::Sdr,
                                info,
                                size,
                                fit,
                            )?
                        }
                        Err(err) => return Err(err),
                    },
                )
            }
            None => None,
        };
        let scaler = if converter.is_some() {
            None
        } else {
            make_scaler(
                &gpu,
                (capture.width, capture.height),
                (out_width, out_height),
                config.scaling,
            )?
        };
        let hdr10 = converter
            .as_ref()
            .is_some_and(|c| c.output() == crate::hdr::HdrOutput::Hdr10);
        let mut encoder = VideoEncoder::new(
            &gpu,
            VideoSettings {
                codec: config.codec,
                width: out_width,
                height: out_height,
                fps: config.fps,
                rate: config.rate,
                preset: config.preset,
                keyframe_interval_secs: config.keyframe_secs.max(1),
                tuning: config.tuning,
                hdr10,
            },
        )?;
        if config.skip_still {
            encoder.time_keyframes();
        }
        let mut info = RecorderInfo {
            adapter: gpu.adapter_name.clone(),
            encoder: encoder.encoder_name.clone(),
            width: out_width,
            height: out_height,
            fps: config.fps,
            hdr: converter.as_ref().map(|c| c.output()),
            audio_tracks: Vec::new(),
            audio_problems: Vec::new(),
            camera: None,
        };

        // File layout: video, then the mix (if any), then the individual tracks.
        let audio = &config.audio;
        let mut streams = vec![encoder.stream_info()];
        let mut mix_encoder = None;
        if audio.mix {
            let (encoder, stream_info) = AacEncoder::new(audio.bitrate_kbps, "Mix")?;
            streams.push(stream_info);
            info.audio_tracks.push("Mix".to_string());
            mix_encoder = Some(encoder);
        }
        let mut track_encoders = Vec::new();
        for track in &audio.tracks {
            let (encoder, stream_info) = AacEncoder::new(audio.bitrate_kbps, &track.title)?;
            streams.push(stream_info);
            info.audio_tracks.push(track.title.clone());
            track_encoders.push(encoder);
        }

        let spill = match &config.buffer_dir {
            Some(dir) => match Spill::new(dir) {
                Ok(spill) => Some(spill),
                Err(err) => {
                    log::warn!("replay buffer kept in memory: {err:#}");
                    None
                }
            },
            None => None,
        };
        let shared = Arc::new(Shared {
            buffer: Mutex::new(ReplayBuffer::new(config.buffer_secs as f64)),
            spill,
            streams: Mutex::new(streams),
            stop: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            encoded: AtomicU64::new(0),
            restarts: AtomicU64::new(0),
            error: Mutex::new(None),
        });

        let mixed_tracks = audio.tracks.iter().filter(|t| t.in_mix).count();
        let mixer = mix_encoder.filter(|_| mixed_tracks > 0).map(|encoder| {
            Arc::new(Mutex::new(Mixer {
                expected: mixed_tracks,
                pending: Default::default(),
                encoder,
                stream: 1,
                shared: shared.clone(),
            }))
        });
        let first_track_stream = if audio.mix { 2 } else { 1 };
        let mut workers = Vec::new();
        for (i, (track, mut encoder)) in audio.tracks.iter().zip(track_encoders).enumerate() {
            let stream = first_track_stream + i;
            let track_shared = shared.clone();
            let track_mixer = if track.in_mix { mixer.clone() } else { None };
            let gate = track.gate.clone();
            let mut gain = if gate.is_some() { 0.0f32 } else { 1.0 };
            let mut gated = vec![0.0f32; CHUNK * 2];
            let worker = TrackWorker::spawn(
                track.source.clone(),
                clock,
                Box::new(move |index, samples| {
                    let samples = match &gate {
                        Some(gate) => {
                            // Fade over 5 ms instead of switching, which would click.
                            let target = if gate.load(Ordering::Relaxed) {
                                1.0
                            } else {
                                0.0
                            };
                            let step = 1.0 / (SAMPLE_RATE as f32 * 0.005);
                            for (out, pair) in
                                gated.chunks_exact_mut(2).zip(samples.chunks_exact(2))
                            {
                                gain = if gain < target {
                                    (gain + step).min(target)
                                } else {
                                    (gain - step).max(target)
                                };
                                out[0] = pair[0] * gain;
                                out[1] = pair[1] * gain;
                            }
                            &gated[..]
                        }
                        None => samples,
                    };
                    push_audio(&track_shared, stream, &mut encoder, index, samples);
                    if let Some(mixer) = &track_mixer {
                        mixer.lock().unwrap().add(index, samples);
                    }
                }),
            )?;
            if let Some(err) = &worker.error {
                info.audio_problems.push((track.title.clone(), err.clone()));
            }
            workers.push((track.title.clone(), worker));
        }

        // Last in the file, so a clip with the camera off all along can leave it out.
        let mut camera = None;
        let mut camera_error = None;
        if let Some(camera_config) = config.camera.clone() {
            let stream = shared.streams.lock().unwrap().len();
            let camera_shared = shared.clone();
            let sink = Box::new(move |packet: Packet, time: f64| {
                let (key, size) = (packet.is_key(), packet.size());
                let held = camera_shared.hold(packet);
                camera_shared
                    .buffer
                    .lock()
                    .unwrap()
                    .push(stream, time, size, key, held);
            });
            let size = (camera_config.width, camera_config.height);
            match crate::camera::CameraWorker::spawn(camera_config, clock, sink) {
                Ok((worker, camera_stream)) => {
                    shared.streams.lock().unwrap().push(camera_stream);
                    info.camera = Some((size.0, size.1, worker.fps));
                    camera = Some((stream, worker));
                }
                Err(err) => {
                    log::warn!("recording without the camera: {err:#}");
                    camera_error = Some(format!("{err:#}"));
                }
            }
        }

        // Input comes in blocks stamped on the QPC clock, a moment after it happened; it is
        // kept like any packet, so the buffer trims it and spills it to disk with the rest.
        let input = config.input.as_ref().map(|hub| {
            let shared = shared.clone();
            let offset = clock.qpc_offset();
            hub.subscribe(move |block| {
                let block = block.clone().shifted(offset);
                let bytes = block.encode(0.0);
                let held = shared.hold(Packet::copy(&bytes));
                shared.buffer.lock().unwrap().push(
                    crate::inputfile::STREAM,
                    block.start,
                    bytes.len(),
                    false,
                    held,
                );
            })
        });

        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("umbra-record".into())
            .spawn(move || {
                let window = match config.target {
                    CaptureTarget::Window(handle) => Some(handle),
                    CaptureTarget::Monitor(_) => None,
                };
                let source = Source {
                    target: config.target,
                    cursor: config.capture_cursor,
                    hdr: config.hdr.is_some(),
                };
                if let Err(err) = run(
                    &thread_shared,
                    gpu,
                    capture,
                    encoder,
                    scaler,
                    converter,
                    config.scaling,
                    config.fps,
                    config.skip_still,
                    clock,
                    window,
                    source,
                ) {
                    log::error!("recorder stopped: {err:#}");
                    *thread_shared.error.lock().unwrap() = Some(format!("{err:#}"));
                }
            })
            .context("cannot start recorder thread")?;

        Ok(Self {
            shared,
            thread: Some(thread),
            workers,
            camera,
            camera_error,
            info,
            clock,
            _input: input,
        })
    }

    /// Why the camera is not recorded right now while it should be (not connected, taken by
    /// another program), if it is not.
    pub fn camera_problem(&self) -> Option<String> {
        match &self.camera {
            Some((_, worker)) => worker.problem(),
            None => self.camera_error.clone(),
        }
    }

    fn camera_stream(&self) -> Option<usize> {
        self.camera.as_ref().map(|(stream, _)| *stream)
    }

    pub fn info(&self) -> &RecorderInfo {
        &self.info
    }

    /// False once the recorder has stopped on its own because of an error.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// The error that stopped the recorder, if any.
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Tracks recorded as silence right now because their source cannot be opened (a
    /// microphone that is not connected), as (track, why).
    pub fn audio_problems(&self) -> Vec<(String, String)> {
        self.workers
            .iter()
            .filter_map(|(title, worker)| Some((title.clone(), worker.problem()?)))
            .collect()
    }

    pub fn stats(&self) -> Stats {
        let buffer = self.shared.buffer.lock().unwrap();
        Stats {
            buffer_secs: buffer.span_secs(),
            buffer_bytes: buffer.bytes() as u64,
            buffer_on_disk: self.shared.spill.is_some(),
            dropped_frames: self.shared.dropped.load(Ordering::Relaxed),
            frames_encoded: self.shared.encoded.load(Ordering::Relaxed),
        }
    }

    /// Changes the buffer length. Shrinking frees memory right away.
    pub fn set_buffer_secs(&self, secs: u32) {
        self.shared.buffer.lock().unwrap().set_window(secs as f64);
    }

    /// Writes the buffer to `path` as an MP4 file, with `tags` (see
    /// `Mp4Writer::create_with_tags`): all of it, or with `span` only what was recorded
    /// between those moments, from the keyframe before the first. Recording carries on while
    /// the file is written.
    pub fn save_clip(
        &self,
        path: &Path,
        tags: &[(String, String)],
        span: Option<(Instant, Instant)>,
    ) -> Result<SavedClip> {
        let began = Instant::now();
        let snapshot = {
            let buffer = self.shared.buffer.lock().unwrap();
            match span {
                Some((from, to)) => {
                    buffer.snapshot_between(self.clock.secs_at(from), self.clock.secs_at(to))
                }
                None => buffer.snapshot(),
            }
        }
        .ok_or_else(|| anyhow!("nothing recorded yet"))?;

        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        // Write under a temporary name so a half-written clip never looks finished.
        let partial = path.with_extension("part");
        let streams = self.shared.streams.lock().unwrap().clone();
        let result = write_clip(
            &partial,
            &streams,
            &snapshot.entries,
            snapshot.start,
            &self.tags_with_fps(tags),
            self.camera_stream(),
        );
        if let Err(err) = result {
            let _ = std::fs::remove_file(&partial);
            return Err(err);
        }
        std::fs::rename(&partial, path)
            .with_context(|| format!("cannot move clip to {}", path.display()))?;

        let frame = 1.0 / self.info.fps as f64;
        // The input of the clip, from the keyframe it really starts on (not the moment asked
        // for), so it lines up with the video exactly.
        let blocks = input_blocks(&snapshot.entries);
        if !blocks.is_empty() {
            let end = match span {
                Some((_, to)) => self.clock.secs_at(to).min(snapshot.end),
                None => snapshot.end,
            } + frame;
            let clip = crate::inputfile::remap(&blocks, &[(snapshot.start, end, 0.0)]);
            let sidecar = crate::inputfile::sidecar(path);
            if let Err(err) = crate::inputfile::write(&sidecar, &clip) {
                log::warn!("clip saved without its input: {err:#}");
            }
        }
        Ok(SavedClip {
            path: path.to_path_buf(),
            duration_secs: snapshot.end - snapshot.start + frame,
            bytes: std::fs::metadata(path).map_or(0, |m| m.len()),
            write_secs: began.elapsed().as_secs_f64(),
        })
    }

    /// Starts writing everything recorded from the next keyframe on to `path`, as it comes,
    /// next to the replay buffer: clips can still be saved meanwhile. The file is written
    /// under a `.recording` name and gets its real name from [`FileRecording::finish`].
    pub fn start_file(&self, path: &Path, tags: &[(String, String)]) -> Result<FileRecording> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let partial = path.with_extension(RECORDING_EXTENSION);
        let input_path = crate::inputfile::sidecar(path);
        let streams = self.shared.streams.lock().unwrap().clone();
        let writer = Mp4Writer::create_fragmented(&partial, &streams, &self.tags_with_fps(tags))?;
        let (tx, rx) = std::sync::mpsc::channel();
        if !self.shared.buffer.lock().unwrap().set_tap(tx) {
            drop(writer);
            let _ = std::fs::remove_file(&partial);
            bail!("a recording is already running");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(FileProgress::default());
        let time_bases: Vec<_> = streams.iter().map(|s| s.time_base).collect();
        let camera = self.camera_stream();
        let thread = {
            let (stop, paused, progress) = (stop.clone(), paused.clone(), progress.clone());
            std::thread::Builder::new()
                .name("umbra-record-file".into())
                .spawn(move || {
                    write_file(
                        rx,
                        writer,
                        &time_bases,
                        camera,
                        &input_path,
                        &stop,
                        &paused,
                        &progress,
                    )
                })
                .context("cannot start the recording thread")?
        };
        Ok(FileRecording {
            stop,
            paused,
            progress,
            thread: Some(thread),
            partial,
            path: path.to_path_buf(),
        })
    }

    /// Stops feeding the running recording, if any. Its file is finished by
    /// [`FileRecording::finish`].
    /// `tags` and the frame rate, which a file that leaves out still frames does not show.
    fn tags_with_fps(&self, tags: &[(String, String)]) -> Vec<(String, String)> {
        let mut tags = tags.to_vec();
        tags.push(("fps".into(), self.info.fps.to_string()));
        tags
    }

    pub fn end_file(&self) {
        self.shared.buffer.lock().unwrap().clear_tap();
    }

    /// Stops recording and frees the buffer.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // Dropping the workers stops and joins their threads.
        self.workers.clear();
        self.camera = None;
        let mut buffer = self.shared.buffer.lock().unwrap();
        buffer.clear_tap();
        buffer.clear();
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Extension of a recording that is still being written. One left behind by a crash is a
/// playable fragmented MP4; [`recover_recordings`] gives it back its `.mp4` name.
pub const RECORDING_EXTENSION: &str = "recording";

#[derive(Default)]
pub struct FileProgress {
    /// Bytes of packets written.
    pub bytes: AtomicU64,
    /// Footage written, in milliseconds.
    pub millis: AtomicU64,
}

/// A recording to a file, running next to the replay buffer.
pub struct FileRecording {
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    progress: Arc<FileProgress>,
    thread: Option<JoinHandle<Result<()>>>,
    partial: PathBuf,
    /// Where the file ends up.
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct SavedRecording {
    pub path: PathBuf,
    pub duration_secs: f64,
    pub bytes: u64,
}

impl FileRecording {
    /// Seconds of footage written so far.
    pub fn secs(&self) -> f64 {
        self.progress.millis.load(Ordering::Relaxed) as f64 / 1000.0
    }

    pub fn bytes(&self) -> u64 {
        self.progress.bytes.load(Ordering::Relaxed)
    }

    /// Leaves out what is recorded from now on until [`FileRecording::resume`]. The file has
    /// no gap: it carries on from the next keyframe after resuming, and the time paused is
    /// not counted in [`FileRecording::secs`].
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// True once writing stopped by itself: the recorder went away or the disk is full.
    pub fn has_ended(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Writes what is still queued, closes the file and gives it its real name, rewritten as
    /// an ordinary MP4 when `finalize` is set (see [`finalize_recording`]); `progress` gets
    /// the bytes read of the recording and its size while it is rewritten. An error while
    /// writing is returned too, but whatever was written is kept.
    pub fn finish(
        mut self,
        finalize: bool,
        progress: &mut dyn FnMut(u64, u64),
    ) -> (Option<SavedRecording>, Option<String>) {
        self.stop.store(true, Ordering::Relaxed);
        let result = match self.thread.take().map(|t| t.join()) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow!("the recording thread crashed")),
            None => Ok(()),
        };
        let error = result.err().map(|e| format!("{e:#}"));
        let duration_secs = self.secs();
        if self.bytes() == 0 {
            // Not a single keyframe arrived; there is nothing to keep.
            let _ = std::fs::remove_file(&self.partial);
            return (None, error.or_else(|| Some("nothing was recorded".into())));
        }
        if finalize {
            let size = std::fs::metadata(&self.partial).map_or(0, |m| m.len());
            progress(0, size);
            // In place first: about a second for any length, and no second copy on the disk.
            match crate::defrag::finish_in_place(&self.partial, &mut |read| progress(read, size)) {
                Ok(()) => {
                    if std::fs::rename(&self.partial, &self.path).is_ok() {
                        return (
                            Some(SavedRecording {
                                path: self.path.clone(),
                                duration_secs,
                                bytes: std::fs::metadata(&self.path).map_or(0, |m| m.len()),
                            }),
                            error,
                        );
                    }
                }
                Err(err) => log::warn!("cannot finish the recording in place, copying it: {err:#}"),
            }
            let ordinary = self.path.with_extension("mp4.part");
            match finalize_recording(&self.partial, &ordinary, &mut |read| progress(read, size)) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&self.partial);
                    if std::fs::rename(&ordinary, &self.path).is_ok() {
                        return (
                            Some(SavedRecording {
                                path: self.path.clone(),
                                duration_secs,
                                bytes: std::fs::metadata(&self.path).map_or(0, |m| m.len()),
                            }),
                            error,
                        );
                    }
                }
                Err(err) => {
                    // Not enough room for the copy, for example: keep the fragmented file.
                    log::warn!("cannot finish the recording as an ordinary MP4: {err:#}");
                    let _ = std::fs::remove_file(&ordinary);
                }
            }
        }
        match std::fs::rename(&self.partial, &self.path) {
            Ok(()) => (
                Some(SavedRecording {
                    path: self.path.clone(),
                    duration_secs,
                    bytes: std::fs::metadata(&self.path).map_or(0, |m| m.len()),
                }),
                error,
            ),
            Err(err) => (
                None,
                Some(format!(
                    "cannot move the recording to {}: {err}",
                    self.path.display()
                )),
            ),
        }
    }
}

impl Drop for FileRecording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

type Tapped = Arc<crate::buffer::Entry<Held>>;

#[allow(clippy::too_many_arguments)]
fn write_file(
    rx: std::sync::mpsc::Receiver<Tapped>,
    mut writer: Mp4Writer,
    time_bases: &[ffmpeg_next::Rational],
    camera: Option<usize>,
    input_path: &Path,
    stop: &AtomicBool,
    paused: &AtomicBool,
    progress: &FileProgress,
) -> Result<()> {
    // The camera joins each stretch on a keyframe of its own.
    let mut camera_started = false;
    use std::sync::mpsc::RecvTimeoutError;
    let seconds = |tb: &ffmpeg_next::Rational| tb.numerator() as f64 / tb.denominator() as f64;
    // Recording-clock time the file starts from: the first keyframe, moved on by every pause,
    // and each stream's offset for it.
    let mut shift: Option<(f64, Vec<i64>)> = None;
    // Where the stretch being written started; `None` before the first keyframe and after a
    // pause, until the next keyframe.
    let mut stretch: Option<f64> = None;
    // End of the latest packet written, on any stream, and the spacing of video frames (their
    // packets may carry no duration).
    let mut written_end = 0.0f64;
    let mut last_video: Option<f64> = None;
    let mut frame_step = 0.0f64;
    // The recording's input file, made once there is input to put in it.
    let mut input: Option<crate::inputfile::InputWriter> = None;
    let mut input_ok = true;
    let mut write = |entry: &crate::buffer::Entry<Held>| -> Result<()> {
        if paused.load(Ordering::Relaxed) {
            stretch = None;
            return Ok(());
        }
        if stretch.is_none() {
            // Every stretch starts on a video keyframe, like a clip.
            if !(entry.stream == 0 && entry.key) {
                return Ok(());
            }
            // The first one starts the file at zero; after a pause, the next carries on
            // where the last one ended.
            let start = match &shift {
                None => entry.time,
                Some((start, _)) => start + (entry.time - written_end).max(0.0),
            };
            let offsets = time_bases
                .iter()
                .map(|tb| (start / seconds(tb)).round() as i64)
                .collect();
            shift = Some((start, offsets));
            stretch = Some(entry.time);
            last_video = None;
            camera_started = false;
        }
        if entry.stream == crate::inputfile::STREAM {
            // Written as it comes, so a crash keeps all but the last moment; a block that
            // straddles the stretch's start keeps what came after it.
            if !input_ok {
                return Ok(());
            }
            let (start, _) = shift.as_ref().unwrap();
            let block = entry
                .packet
                .with(|p| crate::inputfile::Block::decode(p.data().unwrap_or_default(), 0.0))?;
            let from = stretch.unwrap().max(block.start);
            let parts = crate::inputfile::remap(
                std::slice::from_ref(&block),
                &[(from, f64::INFINITY, from - start)],
            );
            let written = (|| -> Result<()> {
                for part in &parts {
                    if input.is_none() {
                        input = Some(crate::inputfile::InputWriter::create(input_path)?);
                    }
                    input.as_mut().unwrap().append(part)?;
                }
                if let Some(input) = input.as_mut() {
                    input.flush()?;
                }
                Ok(())
            })();
            if let Err(err) = written {
                log::warn!("the recording's input stopped: {err:#}");
                input_ok = false;
            }
            return Ok(());
        }
        if entry.time < stretch.unwrap() - 1e-6 {
            return Ok(());
        }
        if Some(entry.stream) == camera && !camera_started {
            if !entry.key {
                return Ok(());
            }
            camera_started = true;
        }
        let (start, offsets) = shift.as_ref().unwrap();
        entry
            .packet
            .with(|packet| writer.write(entry.stream, packet, offsets[entry.stream]))?;
        let length = entry.packet.duration().max(0) as f64 * seconds(&time_bases[entry.stream]);
        if entry.stream == 0 {
            if let Some(prev) = last_video {
                if entry.time > prev && entry.time - prev < 1.0 {
                    frame_step = entry.time - prev;
                }
            }
            last_video = Some(entry.time);
        }
        let length = if entry.stream == 0 {
            length.max(frame_step)
        } else {
            length
        };
        written_end = written_end.max(entry.time + length);
        progress
            .bytes
            .fetch_add(entry.size as u64, Ordering::Relaxed);
        let millis = ((entry.time - start) * 1000.0).max(0.0) as u64;
        progress.millis.fetch_max(millis, Ordering::Relaxed);
        Ok(())
    };
    let result = (|| -> Result<()> {
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(entry) => write(&entry)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            if stop.load(Ordering::Relaxed) {
                for entry in rx.try_iter() {
                    write(&entry)?;
                }
                return Ok(());
            }
        }
    })();
    drop(rx);
    // Close the file properly even after a failed write, so what was written stays readable.
    let finished = writer.finish();
    result.and(finished)
}

/// Rewrites a recording made in fragments as an ordinary MP4 at `to`, copying every packet and
/// tag; nothing is re-encoded. A fragmented file has to be read end to end before a player can
/// show it (about 7.5 s for ten minutes, measured), an ordinary one opens at once.
/// `progress` gets the bytes of `from` read so far.
pub fn finalize_recording(from: &Path, to: &Path, progress: &mut dyn FnMut(u64)) -> Result<()> {
    remux_to_mp4(from, to, &[], progress).map(|_| ())
}

/// Copies the video and audio of any file FFmpeg reads (MP4, MKV, MOV, WebM) into an ordinary
/// MP4 at `to`, without re-encoding, keeping Umbra's tags and adding `extra` ones. Fails when
/// a codec cannot go into MP4. `progress` gets the bytes of `from` read so far. Returns the
/// video and audio packets written, to check the copy against.
pub fn remux_to_mp4(
    from: &Path,
    to: &Path,
    extra: &[(String, String)],
    progress: &mut dyn FnMut(u64),
) -> Result<(usize, usize)> {
    use ffmpeg_next::media::Type;
    let mut input = ffmpeg_next::format::input(&from)
        .with_context(|| format!("cannot open {}", from.display()))?;
    let mut streams = Vec::new();
    let mut map = vec![None; input.streams().count()];
    for stream in input.streams() {
        let medium = stream.parameters().medium();
        if medium != Type::Video && medium != Type::Audio {
            continue;
        }
        map[stream.index()] = Some(streams.len());
        let metadata = stream.metadata();
        let title = metadata
            .get("handler_name")
            .or_else(|| metadata.get("title"))
            .map(str::to_string)
            .filter(|t| !t.is_empty() && !t.ends_with("Handler"));
        streams.push(StreamInfo {
            params: stream.parameters(),
            time_base: stream.time_base(),
            title,
        });
    }
    let tags: Vec<(String, String)> = input
        .metadata()
        .iter()
        .filter(|(k, _)| k.starts_with(crate::mux::TAG_PREFIX))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .chain(extra.iter().cloned())
        .collect();
    let mut writer = Mp4Writer::create_plain(to, &streams, &tags)?;
    let kinds: Vec<Type> = streams.iter().map(|s| s.params.medium()).collect();
    let (mut video, mut audio) = (0usize, 0usize);
    for (n, (stream, packet)) in input.packets().enumerate() {
        if let Some(index) = map[stream.index()] {
            writer.write(index, &packet, 0)?;
            match kinds[index] {
                Type::Video => video += 1,
                _ => audio += 1,
            }
        }
        if n % 256 == 0 && packet.position() > 0 {
            progress(packet.position() as u64);
        }
    }
    writer.finish()?;
    Ok((video, audio))
}

/// True when `path` is an MP4 written in fragments, as recordings are while they record.
pub fn is_fragmented(path: &Path) -> bool {
    ffmpeg_next::format::input(&path)
        .map(|input| {
            input
                .metadata()
                .get("major_brand")
                .is_some_and(|b| b.starts_with("iso5") || b.starts_with("iso6"))
        })
        .unwrap_or(false)
}

/// Gives recordings left behind by a crash (`*.recording`, fragmented MP4) their `.mp4` name,
/// anywhere under `folder`. Returns the files recovered.
pub fn recover_recordings(folder: &Path) -> Vec<PathBuf> {
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
            } else if path.extension().is_some_and(|e| e == RECORDING_EXTENSION) {
                // A conversion cut short by the crash is undone first.
                if let Err(err) = crate::defrag::recover(&path) {
                    log::warn!("cannot put back {}: {err:#}", path.display());
                }
                let target = path.with_extension("mp4");
                if !target.exists() && std::fs::rename(&path, &target).is_ok() {
                    found.push(target);
                }
            }
        }
    }
    found
}

/// The input blocks among `entries`, oldest first, on the recording clock.
fn input_blocks(entries: &[Arc<crate::buffer::Entry<Held>>]) -> Vec<crate::inputfile::Block> {
    let mut blocks: Vec<_> = entries
        .iter()
        .filter(|e| e.stream == crate::inputfile::STREAM)
        .filter_map(|e| {
            e.packet
                .with(|p| crate::inputfile::Block::decode(p.data().unwrap_or_default(), 0.0))
                .ok()
        })
        .collect();
    blocks.sort_by(|a, b| a.start.total_cmp(&b.start));
    blocks
}

fn write_clip(
    path: &Path,
    streams: &[StreamInfo],
    entries: &[Arc<crate::buffer::Entry<Held>>],
    start: f64,
    tags: &[(String, String)],
    camera: Option<usize>,
) -> Result<()> {
    // The camera starts on its own first keyframe. With none (it was off all along) its
    // stream, the last one, is left out.
    let camera_start = camera.and_then(|camera| {
        entries
            .iter()
            .find(|e| e.stream == camera && e.key && e.time >= start - 1e-6)
            .map(|e| e.time)
    });
    let streams = match (camera, camera_start) {
        (Some(camera), None) => &streams[..camera],
        _ => streams,
    };
    let mut writer = Mp4Writer::create_with_tags(path, streams, tags)?;
    // Shift every stream so the clip starts at zero, in each stream's own time base.
    let offsets: Vec<i64> = streams
        .iter()
        .map(|s| {
            (start * s.time_base.denominator() as f64 / s.time_base.numerator() as f64).round()
                as i64
        })
        .collect();
    for entry in entries {
        // Input goes into a file of its own.
        if entry.stream == crate::inputfile::STREAM {
            continue;
        }
        // Audio that began before the first video keyframe can't be used.
        if entry.time < start - 1e-6 {
            continue;
        }
        if Some(entry.stream) == camera
            && camera_start.is_none_or(|camera_start| entry.time < camera_start - 1e-6)
        {
            continue;
        }
        entry
            .packet
            .with(|packet| writer.write(entry.stream, packet, offsets[entry.stream]))?;
    }
    writer.finish()
}

/// A scaler is needed only when the captured size can't be used as it is: a different
/// output size was asked for. Sizes that differ by an odd pixel are cropped by the copy.
fn make_scaler(
    gpu: &Gpu,
    input: (u32, u32),
    output: (u32, u32),
    scaling: Scaling,
) -> Result<Option<GpuScaler>> {
    let cropped = (input.0 & !1, input.1 & !1);
    if output == input || output == cropped {
        Ok(None)
    } else {
        Ok(Some(GpuScaler::new(
            gpu,
            input.0,
            input.1,
            output.0,
            output.1,
            scaling == Scaling::Fit,
        )?))
    }
}

/// What the capture was started with, to start it again.
struct Source {
    target: CaptureTarget,
    cursor: bool,
    hdr: bool,
}

/// No frame for this long starts the capture again (doubling up to `MAX_STALL` while nothing
/// comes, as a screen that does not change sends nothing either).
const FIRST_STALL: Duration = Duration::from_secs(2);
const MAX_STALL: Duration = Duration::from_secs(30);
/// A restart that failed (the screen is still changing mode) is tried again this soon, then
/// twice as long each time it fails again.
const RETRY: Duration = Duration::from_millis(500);

/// The target to start the capture on again. A screen is looked up by its device name, as a
/// change to the screen setup can give it a new handle.
fn retarget(target: CaptureTarget, monitor: Option<&str>) -> CaptureTarget {
    match (target, monitor) {
        (CaptureTarget::Monitor(_), Some(device)) => crate::capture::list_monitors()
            .into_iter()
            .find(|m| m.device.eq_ignore_ascii_case(device))
            .map_or(target, |m| CaptureTarget::Monitor(m.handle)),
        _ => target,
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    shared: &Shared,
    gpu: Gpu,
    mut capture: Capture,
    mut encoder: VideoEncoder,
    mut scaler: Option<GpuScaler>,
    mut converter: Option<crate::hdr::HdrConverter>,
    scaling: Scaling,
    fps: u32,
    skip_still: bool,
    clock: Clock,
    window: Option<isize>,
    source: Source,
) -> Result<()> {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
    // The captured surface's size, which the scaler was built for. The video keeps its size
    // when the captured surface changes size (a resized window, or a game switching the
    // screen to a stretched resolution); the picture is stretched or fitted into it.
    let (mut width, mut height) = (capture.width, capture.height);
    let output = (encoder.settings.width, encoder.settings.height);
    let time_base = shared.streams.lock().unwrap()[0].time_base;
    let seconds_per_tick = time_base.numerator() as f64 / time_base.denominator() as f64;

    let mut sink = |packet: Packet| {
        let time = packet.pts().unwrap_or(0) as f64 * seconds_per_tick;
        let (key, size) = (packet.is_key(), packet.size());
        let held = shared.hold(packet);
        shared.buffer.lock().unwrap().push(0, time, size, key, held);
    };

    // A capture can stop delivering without an error: Windows ends it when the screen setup
    // changes (a game switching resolution or to exclusive fullscreen, HDR turned on or off, a
    // screen asleep or plugged in again, a driver reset). Left alone, the recording would
    // repeat the last picture until Umbra restarts; so it is started again.
    let monitor = match source.target {
        CaptureTarget::Monitor(handle) => crate::capture::list_monitors()
            .into_iter()
            .find(|m| m.handle == handle)
            .map(|m| m.device),
        CaptureTarget::Window(_) => None,
    };
    let mut seen = 0u64;
    let mut last_frame = Instant::now();
    let mut stall = FIRST_STALL;
    // Set while restarting keeps failing: the wait before the next try.
    let mut retry: Option<Duration> = None;

    // With `skip_still`, a still screen still gets a frame this often, so players and
    // editors never see a long gap and the video ends close to where the audio does.
    let keep_alive = u64::from(fps / 2).max(1);
    let mut last_sent = 0u64;
    let mut ticker = Ticker::starting_at(fps, clock.epoch)?;
    while !shared.stop.load(Ordering::Relaxed) {
        let index = ticker.wait_next();
        let arrived = capture.frames_arrived();
        if arrived != seen {
            seen = arrived;
            last_frame = Instant::now();
            // A fresh capture always sends one first frame; only more than that shows the
            // picture is moving, so a still screen keeps the longer wait.
            if arrived > 1 {
                stall = FIRST_STALL;
                retry = None;
            }
        } else if (capture.is_closed() && retry.is_none()) || last_frame.elapsed() >= stall {
            // A capture Windows ended is started again at once, but after a failed try it waits
            // like any other, or it would be tried again on every frame.
            // A window that is gone cannot be captured again: stop, so the controller starts
            // over on the program's new window (a session carries on in a new file).
            if window.is_some_and(|w| !crate::wnd::exists(w)) {
                bail!("the recorded window closed");
            }
            // A minimized window sends nothing on purpose.
            if !window.is_some_and(crate::wnd::is_minimized) {
                if let Err(reason) = unsafe { gpu.device.GetDeviceRemovedReason() } {
                    bail!("the graphics driver was reset ({reason})");
                }
                let target = retarget(source.target, monitor.as_deref());
                let why = if capture.is_closed() {
                    "Windows ended it"
                } else {
                    "no new frames"
                };
                match Capture::start(&gpu, target, source.cursor, fps, source.hdr) {
                    Ok(fresh) => {
                        log::warn!(
                            "capture started again ({why} for {:.1} s); {}x{} now",
                            last_frame.elapsed().as_secs_f64(),
                            fresh.width,
                            fresh.height
                        );
                        shared.restarts.fetch_add(1, Ordering::Relaxed);
                        capture = fresh;
                        seen = 0;
                        retry = None;
                        if (capture.width, capture.height) != (width, height) {
                            width = capture.width;
                            height = capture.height;
                            if converter.is_none() {
                                scaler = make_scaler(&gpu, (width, height), output, scaling)?;
                            }
                        }
                    }
                    Err(err) => {
                        log::warn!("cannot start the capture again ({why}): {err:#}");
                        retry = Some(retry.map_or(RETRY, |r| (r * 2).min(MAX_STALL)));
                    }
                }
            }
            last_frame = Instant::now();
            // Until a restart works the recording repeats the last picture, so a failed one is
            // tried again soon instead of after the doubled wait.
            stall = retry.unwrap_or((stall * 2).min(MAX_STALL));
        }
        // A minimized window is only a strip of title bar: keep the last picture until the
        // window is back.
        let latest = capture
            .take_latest()
            .filter(|_| !window.is_some_and(crate::wnd::is_minimized));
        match latest {
            Some(frame) => {
                if frame.width != width || frame.height != height {
                    if frame.width == 0 || frame.height == 0 {
                        continue;
                    }
                    log::info!(
                        "captured surface changed from {width}x{height} to {}x{}",
                        frame.width,
                        frame.height
                    );
                    width = frame.width;
                    height = frame.height;
                    capture.resize(width, height)?;
                    if converter.is_none() {
                        scaler = make_scaler(&gpu, (width, height), output, scaling)?;
                    }
                }
                // Still in the old, smaller texture: only part of the picture, and copying it
                // would record an older picture. The last one stays until a whole one comes.
                // ponytail: trusts the resized pool to send whole frames next, as Windows
                // documents; if a driver never does, count the skips and restart the capture.
                if !frame.is_whole() {
                    continue;
                }
                if let Some(converter) = converter.as_mut() {
                    let converted =
                        converter.convert(&frame.texture, (frame.width, frame.height))?;
                    encoder.encode_texture(converted, index as i64, &mut sink)?;
                    last_sent = index;
                    shared.encoded.fetch_add(1, Ordering::Relaxed);
                    shared.dropped.store(ticker.skipped, Ordering::Relaxed);
                    continue;
                }
                match &scaler {
                    Some(scaler) => {
                        let scaled = scaler.scale(&frame.texture, frame.width, frame.height)?;
                        encoder.encode_texture(scaled, index as i64, &mut sink)?;
                    }
                    None => encoder.encode_texture(&frame.texture, index as i64, &mut sink)?,
                }
            }
            None => {
                if skip_still && index < last_sent + keep_alive {
                    continue;
                }
                // Nothing new on screen; encode the previous frame again to keep the timeline steady.
                if !encoder.encode_repeat(index as i64, &mut sink)? {
                    continue;
                }
            }
        }
        last_sent = index;
        shared.encoded.fetch_add(1, Ordering::Relaxed);
        shared.dropped.store(ticker.skipped, Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::ReplayBuffer;
    use crate::inputfile::{Block, Event, Key, Pressed, STREAM};

    /// Input in the replay buffer comes out of a clip cut to the keyframe the clip starts on,
    /// counted from it, with what was held there: the way `save_clip` writes it.
    #[test]
    fn a_clip_s_input_starts_on_its_keyframe() {
        let mut buffer: ReplayBuffer<Held> = ReplayBuffer::new(4.0);
        let w = Key {
            vk: 0x57,
            scan: 0x11,
            ext: false,
        };
        // Ten seconds of video, a keyframe every second; W goes down at 5.5 and up at 7.2.
        for frame in 0..300 {
            let t = frame as f64 / 30.0;
            buffer.push(0, t, 100, frame % 30 == 0, Held::Memory(Packet::copy(&[0])));
            // Input arrives late, in blocks, the way the capture hands it out.
            if frame % 15 == 0 && t >= 0.35 {
                let from = t - 0.35;
                let events: Vec<(f64, Event)> = [(5.5, Event::KeyDown(w)), (7.2, Event::KeyUp(w))]
                    .into_iter()
                    .filter(|(at, _)| *at >= from && *at < from + 0.5)
                    .collect();
                let block = Block {
                    start: from,
                    pressed: Pressed {
                        keys: if (5.5..7.2).contains(&from) {
                            vec![w]
                        } else {
                            vec![]
                        },
                        ..Pressed::default()
                    },
                    events,
                };
                let bytes = block.encode(0.0);
                buffer.push(
                    STREAM,
                    from,
                    bytes.len(),
                    false,
                    Held::Memory(Packet::copy(&bytes)),
                );
            }
        }
        // A clip asked to start at 6.4 starts on the keyframe at 6.0.
        let snapshot = buffer.snapshot_between(6.4, 9.0).unwrap();
        assert_eq!(snapshot.start, 6.0);
        let blocks = input_blocks(&snapshot.entries);
        let clip = crate::inputfile::remap(&blocks, &[(snapshot.start, 9.0, 0.0)]);
        // W is held from the clip's first frame and goes up 1.2 s into it.
        assert_eq!(crate::inputfile::pressed_at(&clip, 0.0).keys, vec![w]);
        assert_eq!(crate::inputfile::pressed_at(&clip, 1.19).keys, vec![w]);
        assert!(crate::inputfile::pressed_at(&clip, 1.21).keys.is_empty());
        // The buffer trimmed what it no longer holds: no clip reaches before its first keyframe.
        let all = buffer.snapshot().unwrap();
        assert!(
            input_blocks(&all.entries)
                .iter()
                .all(|b| b.start >= all.start - 1.0)
        );
    }
}
