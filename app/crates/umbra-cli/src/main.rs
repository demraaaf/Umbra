//! Command-line harness for exercising the recording engine without the UI.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use umbra_core::audio::AudioSource;
use umbra_core::capture::{Capture, CaptureTarget, list_monitors};
use umbra_core::controller::{Controller, Event, State};
use umbra_core::d3d::Gpu;
use umbra_core::encode::{RateControl, VideoCodec, VideoEncoder, VideoSettings};
use umbra_core::hotkey::{Hotkey, HotkeyListener, inject};
use umbra_core::mux::Mp4Writer;
use umbra_core::process::find_roots;
use umbra_core::recorder::{AudioConfig, AudioTrackConfig, Recorder, RecorderConfig};
use umbra_core::sound::SaveSound;
use umbra_core::timer::Ticker;
use umbra_core::util::{
    ctrl_c_pressed, install_ctrl_c_handler, local_timestamp, process_cpu_time, set_dpi_aware,
};

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

const USAGE: &str = "\
umbra-cli monitors
umbra-cli cameras
    Lists the webcams with their ids and the sizes they film at.
umbra-cli spike [--monitor N] [--seconds 30] [--fps 60] [--codec h264|hevc|av1]
                [--bitrate KBPS | --qp N] [--preset 1-7] [--out FILE]
umbra-cli colortest [--codec h264|hevc|av1] [--out FILE]
umbra-cli input FILE
    Prints a clip's input file (`<clip>.input`): what was held and pressed, and when.
umbra-cli burn-in FILE [camera] [input] [logo]
    Draws a clip's layers (all it has when none are named) into the picture, as
    `<name> (burned).mp4` beside it, at the default settings' placements.
umbra-cli clips FOLDER
    Lists the clips under FOLDER (subfolders included), newest first.
umbra-cli thumb FILE [--cache DIR]
    Makes (or reuses) the 320 px JPEG thumbnail of a clip and prints where it is.
umbra-cli export IN OUT --start S --end E [--crop X,Y,W,H] [--tracks 0,1|none]
                 [--reencode] [--encoder NAME] [--codec h264|hevc|av1] [--bitrate KBPS]
                 [--max-fps N] [--quick]
    Trims and crops a clip. Copies the packets when the cut starts on a keyframe and
    there is no crop, otherwise re-encodes the video. --tracks lists the audio tracks to
    keep (default all). Ctrl+C cancels.
umbra-cli record [--monitor N] [--fps 60] [--codec h264|hevc|av1] [--bitrate KBPS | --qp N]
                 [--preset 1-7] [--buffer 30] [--hotkey Alt+F10] [--out-dir DIR]
                 [--run-for SECS] [--press-after SECS] [--mute]
                 [--game-exe NAME] [--app NAME]... [--others] [--mic] [--mix]
                 [--audio-bitrate KBPS] [--size WIDTHxHEIGHT]
                 [--camera ID|pattern] [--camera-size 1280x720] [--skip-still]
umbra-cli daemon [--settings FILE] [--run-for SECS] [--press-after SECS] [--start]
    Runs the full app logic without a window: recording modes, game detection, the
    hotkey and clip naming, all from the settings file (default: the Umbra folder in Documents).
    Keeps the last --buffer seconds in RAM and saves a clip on the hotkey.
    --press-after types the hotkey itself after that many seconds, for testing.
    Audio: --game-exe records that program's sound as the Game track, each --app adds
    a track for another program (for example Discord.exe), --others records everything
    except the game, --mic adds the default microphone, --mix puts a combined track first.
    --camera records a webcam (an id from `cameras`, or `pattern` for colour bars) as a
    video stream of its own after the audio.";

fn main() -> Result<()> {
    log::set_logger(&Logger).ok();
    log::set_max_level(log::LevelFilter::Info);
    set_dpi_aware();
    umbra_core::quiet_ffmpeg_logs();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("monitors") => {
            for (index, m) in list_monitors().iter().enumerate() {
                println!(
                    "{index}: {} {}x{} @ {} Hz at ({}, {}){}",
                    m.device,
                    m.width,
                    m.height,
                    m.refresh_hz,
                    m.x,
                    m.y,
                    if m.primary { " primary" } else { "" }
                );
            }
            Ok(())
        }
        Some("cameras") => {
            for camera in umbra_core::camera::list_cameras() {
                let sizes: Vec<String> = camera
                    .sizes
                    .iter()
                    .map(|(w, h)| format!("{w}x{h}"))
                    .collect();
                println!(
                    "{}\n  {}\n  {}",
                    format!("{} ({})", camera.name, camera.api),
                    camera.id,
                    sizes.join(" ")
                );
            }
            Ok(())
        }
        Some("spike") => spike(&args[1..]),
        Some("colortest") => colortest(&args[1..]),
        Some("record") => record(&args[1..]),
        Some("daemon") => daemon(&args[1..]),
        Some("clips") => clips(&args[1..]),
        Some("thumb") => thumb(&args[1..]),
        Some("export") => export(&args[1..]),
        Some("input") => {
            let path = args.get(1).context("input needs a file")?;
            for block in umbra_core::inputfile::read(std::path::Path::new(path))? {
                let p = &block.pressed;
                println!(
                    "{:9.3}  held: {} keys {:?}, {} text, buttons {:05b}",
                    block.start,
                    p.keys.len(),
                    p.keys.iter().map(|k| k.vk).collect::<Vec<_>>(),
                    p.text,
                    p.buttons
                );
                for (t, event) in &block.events {
                    println!("{t:9.3}    {event:?}");
                }
            }
            Ok(())
        }
        Some("burn-in") => {
            use umbra_core::burnin::{self, Layers};
            let path = std::path::Path::new(args.get(1).context("burn-in needs a file")?);
            let meta = umbra_core::library::probe(path)?;
            let named = |n: &str| args[2..].iter().any(|a| a == n);
            let which = if args.len() > 2 {
                Layers {
                    camera: named("camera"),
                    input: named("input"),
                    logo: named("logo"),
                }
            } else {
                burnin::layers(&meta)
            };
            let output = burnin::output_for(path);
            let started = std::time::Instant::now();
            let settings = umbra_core::settings::Settings::default();
            burnin::burn(&meta, which, &settings, &output, &mut |_| true)?;
            burnin::check(&meta, which, &output)?;
            println!(
                "{which:?} burned in: {} ({:.1} s); it has {:?}",
                output.display(),
                started.elapsed().as_secs_f64(),
                burnin::layers(&umbra_core::library::probe(&output)?)
            );
            Ok(())
        }
        _ => {
            println!("ffmpeg {}\n\n{USAGE}", umbra_core::ffmpeg_version());
            Ok(())
        }
    }
}

struct Options {
    monitor: usize,
    seconds: u64,
    fps: u32,
    codec: VideoCodec,
    rate: RateControl,
    preset: u8,
    out: PathBuf,
    buffer: u32,
    hotkey: String,
    out_dir: PathBuf,
    run_for: u64,
    press_after: Option<u64>,
    mute: bool,
    game_exe: Option<String>,
    apps: Vec<String>,
    others: bool,
    mic: bool,
    mix: bool,
    audio_bitrate: u32,
    size: Option<(u32, u32)>,
    camera: Option<String>,
    camera_size: (u32, u32),
    skip_still: bool,
}

fn parse_options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        monitor: 0,
        seconds: 30,
        fps: 60,
        codec: VideoCodec::H264,
        rate: RateControl::Cbr { kbps: 50_000 },
        preset: 4,
        out: PathBuf::from("spike.mp4"),
        buffer: 30,
        hotkey: "Alt+F10".into(),
        out_dir: PathBuf::from("clips"),
        run_for: 0,
        press_after: None,
        mute: false,
        skip_still: false,
        game_exe: None,
        apps: Vec::new(),
        others: false,
        mic: false,
        mix: false,
        audio_bitrate: 160,
        size: None,
        camera: None,
        camera_size: (1280, 720),
    };
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = || iter.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--monitor" => options.monitor = value()?.parse()?,
            "--seconds" => options.seconds = value()?.parse()?,
            "--fps" => options.fps = value()?.parse()?,
            "--codec" => {
                options.codec = VideoCodec::parse(value()?).context("unknown codec")?;
            }
            "--bitrate" => {
                options.rate = RateControl::Cbr {
                    kbps: value()?.parse()?,
                }
            }
            "--qp" => {
                options.rate = RateControl::ConstQp {
                    qp: value()?.parse()?,
                }
            }
            "--preset" => options.preset = value()?.parse()?,
            "--out" => options.out = PathBuf::from(value()?),
            "--buffer" => options.buffer = value()?.parse()?,
            "--hotkey" => options.hotkey = value()?.clone(),
            "--out-dir" => options.out_dir = PathBuf::from(value()?),
            "--run-for" => options.run_for = value()?.parse()?,
            "--press-after" => options.press_after = Some(value()?.parse()?),
            "--mute" => options.mute = true,
            "--skip-still" => options.skip_still = true,
            "--game-exe" => options.game_exe = Some(value()?.clone()),
            "--app" => options.apps.push(value()?.clone()),
            "--others" => options.others = true,
            "--mic" => options.mic = true,
            "--mix" => options.mix = true,
            "--audio-bitrate" => options.audio_bitrate = value()?.parse()?,
            "--size" => {
                let text = value()?;
                let (w, h) = text.split_once('x').context("--size needs WIDTHxHEIGHT")?;
                options.size = Some((w.parse()?, h.parse()?));
            }
            "--camera" => options.camera = Some(value()?.clone()),
            "--camera-size" => {
                let text = value()?;
                let (w, h) = text
                    .split_once('x')
                    .context("--camera-size needs WIDTHxHEIGHT")?;
                options.camera_size = (w.parse()?, h.parse()?);
            }
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
    }
    Ok(options)
}

/// Milestone 1: capture one monitor, encode on the GPU and write a file.
fn spike(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let monitors = list_monitors();
    let monitor = monitors
        .get(options.monitor)
        .with_context(|| format!("no monitor {}", options.monitor))?;

    let gpu = Gpu::new()?;
    println!("GPU      {}", gpu.adapter_name);
    println!(
        "Monitor  {} {}x{} @ {} Hz",
        monitor.device, monitor.width, monitor.height, monitor.refresh_hz
    );

    let capture = Capture::start(
        &gpu,
        CaptureTarget::Monitor(monitor.handle),
        true,
        options.fps,
        false,
    )?;
    let settings = VideoSettings {
        hdr10: false,
        codec: options.codec,
        width: capture.width,
        height: capture.height,
        fps: options.fps,
        rate: options.rate,
        preset: options.preset,
        keyframe_interval_secs: 1,
        tuning: Default::default(),
    };
    let mut encoder = VideoEncoder::new(&gpu, settings.clone())?;
    println!(
        "Encoder  {} {}x{} @ {} fps, {:?}, preset p{}",
        encoder.encoder_name,
        settings.width,
        settings.height,
        settings.fps,
        settings.rate,
        settings.preset
    );

    if let Some(parent) = options.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = Mp4Writer::create(&options.out, &[encoder.stream_info()])?;

    let mut write_error = None;
    let mut packets = 0u64;
    let mut bytes = 0u64;
    let mut sink = |packet: umbra_core::Packet| {
        packets += 1;
        bytes += packet.size() as u64;
        if let Err(err) = writer.write(0, &packet, 0) {
            write_error.get_or_insert(err);
        }
    };

    let mut ticker = Ticker::new(options.fps)?;
    let total_frames = options.seconds * options.fps as u64;
    let (mut fresh, mut repeated, mut waiting) = (0u64, 0u64, 0u64);
    let mut work = Duration::ZERO;
    let mut work_max = Duration::ZERO;
    let cpu_start = process_cpu_time();
    let started = Instant::now();

    println!("Recording {} s ...", options.seconds);
    loop {
        let index = ticker.wait_next();
        if index >= total_frames {
            break;
        }
        let began = Instant::now();
        match capture.take_latest() {
            Some(frame) => {
                if frame.width != settings.width || frame.height != settings.height {
                    bail!("monitor resolution changed during the recording");
                }
                encoder.encode_texture(&frame.texture, index as i64, &mut sink)?;
                fresh += 1;
            }
            None => {
                if encoder.encode_repeat(index as i64, &mut sink)? {
                    repeated += 1;
                } else {
                    waiting += 1;
                }
            }
        }
        let took = began.elapsed();
        work += took;
        work_max = work_max.max(took);
    }
    encoder.finish(&mut sink)?;
    let elapsed = started.elapsed();
    let cpu = process_cpu_time() - cpu_start;
    let arrived = capture.frames_arrived();
    drop(capture);

    if let Some(err) = write_error {
        return Err(err);
    }
    writer.finish()?;

    let encoded = fresh + repeated;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    println!();
    println!("Wrote    {}", options.out.display());
    println!(
        "Frames   {encoded} encoded of {total_frames} ({fresh} new, {repeated} repeated, {} skipped, {waiting} before first frame)",
        ticker.skipped
    );
    println!(
        "Capture  {arrived} frames delivered by Windows ({:.1} per second)",
        arrived as f64 / elapsed.as_secs_f64()
    );
    println!(
        "Per tick {:.2} ms average, {:.2} ms worst (budget {:.2} ms)",
        work.as_secs_f64() * 1000.0 / encoded.max(1) as f64,
        work_max.as_secs_f64() * 1000.0,
        1000.0 / options.fps as f64
    );
    println!(
        "CPU      {:.1}% of one core, {:.2}% of the whole CPU",
        cpu.as_secs_f64() / elapsed.as_secs_f64() * 100.0,
        cpu.as_secs_f64() / elapsed.as_secs_f64() * 100.0 / cores
    );
    println!(
        "Output   {packets} packets, {:.1} MB, {:.1} Mbit/s",
        bytes as f64 / 1e6,
        bytes as f64 * 8.0 / 1e6 / elapsed.as_secs_f64()
    );
    Ok(())
}

/// Encodes solid colour bars so the decoded colours can be compared with the originals.
fn colortest(args: &[String]) -> Result<()> {
    let mut options = parse_options(args)?;
    if options.out == PathBuf::from("spike.mp4") {
        options.out = PathBuf::from("colortest.mp4");
    }
    let gpu = Gpu::new()?;
    let (width, height) = (1280, 720);
    let texture = gpu.test_pattern(width, height)?;
    let mut encoder = VideoEncoder::new(
        &gpu,
        VideoSettings {
            hdr10: false,
            codec: options.codec,
            width,
            height,
            fps: 60,
            rate: RateControl::ConstQp { qp: 10 },
            preset: options.preset,
            keyframe_interval_secs: 1,
            tuning: Default::default(),
        },
    )?;
    let mut writer = Mp4Writer::create(&options.out, &[encoder.stream_info()])?;
    let mut write_error = None;
    let mut sink = |packet: umbra_core::Packet| {
        if let Err(err) = writer.write(0, &packet, 0) {
            write_error.get_or_insert(err);
        }
    };
    for index in 0..30 {
        encoder.encode_texture(&texture, index, &mut sink)?;
    }
    encoder.finish(&mut sink)?;
    if let Some(err) = write_error {
        return Err(err);
    }
    writer.finish()?;
    println!("Wrote {}", options.out.display());
    println!(
        "Bars, left to right (R, G, B): {:?}",
        umbra_core::d3d::TEST_PATTERN_BARS
    );
    Ok(())
}

/// Milestone 2: keep a replay buffer in RAM and save a clip when the hotkey is pressed.
fn record(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let hotkey: Hotkey = options.hotkey.parse()?;
    let monitors = list_monitors();
    let monitor = monitors
        .get(options.monitor)
        .with_context(|| format!("no monitor {}", options.monitor))?;
    install_ctrl_c_handler();

    let recorder = Recorder::start(RecorderConfig {
        target: CaptureTarget::Monitor(monitor.handle),
        codec: options.codec,
        fps: options.fps,
        rate: options.rate,
        preset: options.preset,
        capture_cursor: true,
        output_size: options.size,
        scaling: umbra_core::settings::Scaling::Stretch,
        keyframe_secs: 1,
        skip_still: options.skip_still,
        tuning: Default::default(),
        buffer_secs: options.buffer,
        buffer_dir: None,
        hdr: None,
        audio: audio_config(&options),
        camera: options
            .camera
            .as_ref()
            .map(|id| umbra_core::camera::CameraConfig {
                input: if id == "pattern" {
                    umbra_core::camera::CameraInput::Pattern
                } else {
                    umbra_core::camera::CameraInput::Device(id.clone())
                },
                width: options.camera_size.0,
                height: options.camera_size.1,
                max_fps: options.fps,
                codec: options.codec,
                preset: options.preset,
                on: Arc::new(AtomicBool::new(true)),
            }),
        input: None,
    })?;
    let info = recorder.info();
    println!(
        "Recording {} {}x{} @ {} fps with {} on {}",
        monitor.device, info.width, info.height, info.fps, info.encoder, info.adapter
    );
    if !info.audio_tracks.is_empty() {
        println!("Audio tracks: {}", info.audio_tracks.join(", "));
    }
    for (track, problem) in &info.audio_problems {
        println!("Audio track {track} is silent: {problem}");
    }
    if let Some((w, h, fps)) = info.camera {
        println!("Camera {w}x{h} @ {fps} fps");
    }
    if let Some(problem) = recorder.camera_problem() {
        println!("Camera not recorded: {problem}");
    }
    println!(
        "Buffer {} s, hotkey {}, clips go to {}",
        options.buffer,
        options.hotkey,
        options.out_dir.display()
    );

    let (press_tx, press_rx) = mpsc::channel::<Instant>();
    let _listener = HotkeyListener::start(hotkey.clone(), move || {
        // Runs inside the input hook, so only signal.
        let _ = press_tx.send(Instant::now());
    })?;
    let sound = SaveSound::new(0.8);
    let (saved_tx, saved_rx) = mpsc::channel();

    let started = Instant::now();
    let mut last_report = Instant::now();
    let mut pressed_by_us = false;
    let mut saves = 0;
    let recorder = std::sync::Arc::new(recorder);
    println!("Press {} to save a clip. Ctrl+C quits.", options.hotkey);

    loop {
        if ctrl_c_pressed() {
            break;
        }
        if options.run_for > 0 && started.elapsed().as_secs() >= options.run_for {
            break;
        }
        if !recorder.is_running() {
            bail!(
                "recorder stopped: {}",
                recorder.error().unwrap_or_else(|| "unknown error".into())
            );
        }
        if let Some(after) = options.press_after {
            if !pressed_by_us && started.elapsed().as_secs() >= after {
                pressed_by_us = true;
                println!("Typing {} ...", options.hotkey);
                inject(&hotkey)?;
            }
        }

        if let Ok(pressed_at) = press_rx.recv_timeout(Duration::from_millis(100)) {
            let latency = pressed_at.elapsed();
            saves += 1;
            let path = options
                .out_dir
                .join(format!("Clip {}.mp4", local_timestamp()))
                .with_file_name(format!("Clip {} ({saves}).mp4", local_timestamp()));
            println!(
                "Hotkey seen after {:.1} ms, saving {}",
                latency.as_secs_f64() * 1000.0,
                path.display()
            );
            let recorder = recorder.clone();
            let saved_tx = saved_tx.clone();
            std::thread::spawn(move || {
                let _ = saved_tx.send(recorder.save_clip(&path, &[], None));
            });
        }
        while let Ok(result) = saved_rx.try_recv() {
            match result {
                Ok(clip) => {
                    if !options.mute {
                        sound.play();
                    }
                    println!(
                        "Saved    {} ({:.1} s, {:.1} MB, written in {:.0} ms)",
                        clip.path.display(),
                        clip.duration_secs,
                        clip.bytes as f64 / 1e6,
                        clip.write_secs * 1000.0
                    );
                }
                Err(err) => eprintln!("Save failed: {err:#}"),
            }
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            last_report = Instant::now();
            let stats = recorder.stats();
            println!(
                "Buffer   {:.1} s, {:.0} MB, {} frames encoded, {} dropped",
                stats.buffer_secs,
                stats.buffer_bytes as f64 / 1e6,
                stats.frames_encoded,
                stats.dropped_frames
            );
        }
    }

    // Let a save that is still writing finish before the recorder goes away.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match saved_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(clip)) => println!(
                "Saved    {} ({:.1} s)",
                clip.path.display(),
                clip.duration_secs
            ),
            Ok(Err(err)) => eprintln!("Save failed: {err:#}"),
            Err(_) => break,
        }
    }
    Ok(())
}

fn root_pid(exe: &str) -> u32 {
    let roots = find_roots(exe);
    if roots.is_empty() {
        eprintln!("{exe} is not running; its track will be silent");
    }
    roots.first().copied().unwrap_or(0)
}

fn audio_config(options: &Options) -> AudioConfig {
    let mut tracks = Vec::new();
    let mut game_pid = None;
    if let Some(exe) = &options.game_exe {
        let pid = root_pid(exe);
        game_pid = Some(pid);
        tracks.push(AudioTrackConfig {
            title: "Game".into(),
            source: AudioSource::ProcessTree { pid },
            in_mix: true,
            gate: None,
        });
    }
    for exe in &options.apps {
        tracks.push(AudioTrackConfig {
            title: exe
                .trim_end_matches(".exe")
                .trim_end_matches(".EXE")
                .to_string(),
            source: AudioSource::ProcessTree { pid: root_pid(exe) },
            // "Other apps" already contains it, so adding it again would double it.
            in_mix: !options.others,
            gate: None,
        });
    }
    if options.others {
        // Excluding the game, or this program when there is no game, leaves everything else.
        let pid = game_pid.unwrap_or_else(std::process::id);
        tracks.push(AudioTrackConfig {
            title: "Other apps".into(),
            source: AudioSource::AllExceptTree { pid },
            in_mix: true,
            gate: None,
        });
    }
    if options.mic {
        tracks.push(AudioTrackConfig {
            title: "Microphone".into(),
            source: AudioSource::Microphone { device_id: None },
            in_mix: true,
            gate: None,
        });
    }
    AudioConfig {
        tracks,
        mix: options.mix,
        bitrate_kbps: options.audio_bitrate,
    }
}

/// The app without a window, for testing the controller.
fn daemon(args: &[String]) -> Result<()> {
    let mut settings_path = umbra_core::settings::settings_path();
    let (mut run_for, mut press_after, mut start) = (0u64, None::<u64>, false);
    let mut iter = args.iter();
    while let Some(flag) = iter.next() {
        let mut value = || iter.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--settings" => settings_path = PathBuf::from(value()?),
            "--run-for" => run_for = value()?.parse()?,
            "--press-after" => press_after = Some(value()?.parse()?),
            "--start" => start = true,
            other => bail!(
                "unknown option {other}

{USAGE}"
            ),
        }
    }
    install_ctrl_c_handler();

    let controller = Controller::start(settings_path.clone(), |event| match event {
        Event::Status(status) => {
            let label = match status.state {
                State::Idle => "idle".to_string(),
                State::Recording => format!(
                    "recording {} {}x{}@{} on {}, buffer {:.1} s / {:.0} MB, {} dropped",
                    status.target.as_deref().unwrap_or("?"),
                    status.width,
                    status.height,
                    status.fps,
                    status.encoder.as_deref().unwrap_or("?"),
                    status.buffer_secs,
                    status.buffer_bytes as f64 / 1e6,
                    status.dropped_frames
                ),
                State::Error => format!("error: {}", status.error.as_deref().unwrap_or("?")),
            };
            println!(
                "Status   {label}; game: {}",
                status.game.as_deref().unwrap_or("none")
            );
        }
        Event::ClipSaved(saved) => println!(
            "Saved    {} ({}, {:.1} s, {:.1} MB)",
            saved.clip.path.display(),
            saved.game,
            saved.clip.duration_secs,
            saved.clip.bytes as f64 / 1e6
        ),
        Event::ScreenshotSaved(shot) => {
            println!("Saved    {} ({})", shot.path.display(), shot.game)
        }
        Event::RecordingSaved(rec) => match rec.saved {
            Some(saved) => println!(
                "Saved    {} ({}, {:.0} s, {:.1} MB){}",
                saved.path.display(),
                rec.game,
                saved.duration_secs,
                saved.bytes as f64 / 1e6,
                rec.error.map(|e| format!("; {e}")).unwrap_or_default()
            ),
            None => println!(
                "Recording not saved: {}",
                rec.error.as_deref().unwrap_or("?")
            ),
        },
        Event::RecordingStarted { kind, game } => {
            println!("Recording {} ({kind:?})", game)
        }
        Event::RecordingFinishing { game, .. } => println!("Finishing the recording of {game}"),
        Event::AutoClipLog(entry) => println!("Auto-clip {}", entry.message),
        Event::ProcessingStarted => {}
        Event::RecordingPaused { paused, game, .. } => println!(
            "Recording of {game} {}",
            if paused { "paused" } else { "resumed" }
        ),
        Event::ClipPending { secs, .. } => println!("Saving a clip in {secs} s"),
        Event::ClipFailed { error } => println!("Clip not saved: {error}"),
        Event::LowDisk { free_bytes, .. } => {
            println!("Disk almost full: {} MB left", free_bytes / 1_000_000)
        }
        Event::LibraryChanged => {}
    })?;
    println!("Settings {}", settings_path.display());
    if start {
        controller.start_recording();
    }
    let began = Instant::now();
    let mut pressed = false;
    while !ctrl_c_pressed() && (run_for == 0 || began.elapsed().as_secs() < run_for) {
        if let Some(after) = press_after {
            if !pressed && began.elapsed().as_secs() >= after {
                pressed = true;
                let hotkey: Hotkey = controller.settings().clip.hotkey.parse()?;
                println!("Typing {} ...", controller.settings().clip.hotkey);
                inject(&hotkey)?;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    controller.shutdown();
    Ok(())
}

/// Lists the clips under a folder, newest first.
fn clips(args: &[String]) -> Result<()> {
    let folder = PathBuf::from(args.first().context("clips needs a FOLDER")?);
    let started = Instant::now();
    let found = umbra_core::library::scan(&folder);
    let took = started.elapsed();
    for clip in &found {
        let tracks = clip
            .audio_tracks
            .iter()
            .map(|t| format!("{}:{}", t.index, t.title))
            .collect::<Vec<_>>()
            .join(", ");
        let game = umbra_core::library::game_from_path(&folder, &clip.path);
        println!(
            "{}  {:>7.2} s  {}x{} @ {} fps  {:<4}  {:>8.1} MB  audio [{}]  game {}  {}",
            clip.created_at,
            clip.duration_secs,
            clip.width,
            clip.height,
            clip.fps,
            clip.codec,
            clip.size_bytes as f64 / 1e6,
            tracks,
            game.as_deref().unwrap_or("-"),
            clip.path.display()
        );
    }
    println!(
        "{} clips in {:.0} ms",
        found.len(),
        took.as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Makes the thumbnail of one clip.
fn thumb(args: &[String]) -> Result<()> {
    let mut file = None;
    let mut cache = std::env::temp_dir().join("umbra-thumbnails");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--cache" => cache = PathBuf::from(iter.next().context("--cache needs a value")?),
            other if other.starts_with("--") => bail!(
                "unknown option {other}

{USAGE}"
            ),
            _ => file = Some(PathBuf::from(arg)),
        }
    }
    let file = file.context("thumb needs a FILE")?;
    let was_cached = umbra_core::library::cached_thumbnail(&file, &cache).is_some();
    let started = Instant::now();
    let path = umbra_core::library::thumbnail(&file, &cache)?;
    println!(
        "{} ({} bytes, {:.0} ms{})",
        path.display(),
        std::fs::metadata(&path)?.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        if was_cached { ", from the cache" } else { "" }
    );
    Ok(())
}

/// Trims and crops a clip.
fn export(args: &[String]) -> Result<()> {
    use umbra_core::export::{Crop, ExportJob, ExportMode, export_with_report, is_cancelled, plan};

    let mut positional = Vec::new();
    let (mut start, mut end) = (0.0f64, None::<f64>);
    let (mut crop, mut tracks) = (None, None);
    let (mut force_reencode, mut encoder) = (false, None);
    let (mut codec, mut bitrate) = (None, None::<i64>);
    let (mut max_fps, mut quick) = (None, false);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--start" => start = value()?.parse().context("--start needs seconds")?,
            "--end" => end = Some(value()?.parse().context("--end needs seconds")?),
            "--crop" => {
                let numbers: Vec<u32> = value()?
                    .split(',')
                    .map(|n| n.trim().parse())
                    .collect::<Result<_, _>>()
                    .context("--crop needs X,Y,W,H")?;
                let [x, y, width, height] = numbers[..] else {
                    bail!("--crop needs X,Y,W,H")
                };
                crop = Some(Crop {
                    x,
                    y,
                    width,
                    height,
                });
            }
            "--tracks" => {
                let text = value()?;
                tracks = Some(if text == "none" || text.is_empty() {
                    Vec::new()
                } else {
                    text.split(',')
                        .map(|n| n.trim().parse())
                        .collect::<Result<Vec<usize>, _>>()
                        .context("--tracks needs numbers like 0,1, or none")?
                });
            }
            "--reencode" => force_reencode = true,
            "--encoder" => encoder = Some(value()?.clone()),
            "--codec" => {
                codec = Some(match value()?.as_str() {
                    "h264" => umbra_core::encode::VideoCodec::H264,
                    "hevc" => umbra_core::encode::VideoCodec::Hevc,
                    "av1" => umbra_core::encode::VideoCodec::Av1,
                    other => bail!("unknown codec {other}"),
                })
            }
            "--max-fps" => {
                max_fps = Some(
                    value()?
                        .parse::<u32>()
                        .context("--max-fps needs a number")?,
                )
            }
            "--quick" => quick = true,
            "--bitrate" => {
                bitrate = Some(value()?.parse::<i64>().context("--bitrate needs kbit/s")? * 1000)
            }
            other if other.starts_with("--") => bail!(
                "unknown option {other}

{USAGE}"
            ),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [input, output] = &positional[..] else {
        bail!(
            "export needs IN and OUT

{USAGE}"
        )
    };

    let meta = umbra_core::library::probe(input)?;
    let mut job = ExportJob::new(input, output, start, end.unwrap_or(meta.duration_secs));
    job.crop = crop;
    job.audio_tracks =
        tracks.unwrap_or_else(|| meta.audio_tracks.iter().map(|t| t.index).collect());
    job.force_reencode = force_reencode;
    job.encoder = encoder;
    job.codec = codec;
    job.video_bitrate = bitrate;
    job.max_fps = max_fps;
    job.quick = quick;

    let planned = plan(&job)?;
    println!(
        "Input    {} ({:.2} s, {}x{} {}, {} audio tracks)",
        input.display(),
        meta.duration_secs,
        meta.width,
        meta.height,
        meta.codec,
        meta.audio_tracks.len()
    );
    println!(
        "Plan     {} from {:.3} s to {:.3} s{}{}",
        match planned.mode {
            ExportMode::StreamCopy => "stream copy",
            ExportMode::Reencode => "re-encode",
        },
        planned.start_secs,
        planned.end_secs,
        planned
            .reason
            .as_ref()
            .map(|r| format!(" ({r})"))
            .unwrap_or_default(),
        planned
            .crop
            .map(|c| format!(", crop {},{} {}x{}", c.x, c.y, c.width, c.height))
            .unwrap_or_default()
    );

    install_ctrl_c_handler();
    let mut last_percent = -1;
    let result = export_with_report(&job, &mut |fraction| {
        let percent = (fraction * 100.0) as i32;
        if percent != last_percent {
            last_percent = percent;
            eprint!("\rProgress {percent:>3}%");
        }
        !ctrl_c_pressed()
    });
    eprintln!();
    let report = match result {
        Ok(report) => report,
        Err(err) if is_cancelled(&err) => {
            println!("Cancelled; no file was written");
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    println!(
        "Wrote    {} ({:.1} MB)",
        output.display(),
        report.output_bytes as f64 / 1e6
    );
    println!(
        "Result   {} video frames in {:.2} s{}",
        report.video_frames,
        report.elapsed_secs,
        match (&report.encoder, report.video_bitrate_kbps) {
            (Some(name), Some(kbps)) => format!(", {name} at {:.1} Mbit/s", kbps as f64 / 1000.0),
            _ => String::new(),
        }
    );
    Ok(())
}
