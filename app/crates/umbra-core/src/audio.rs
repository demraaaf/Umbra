//! Audio capture through WASAPI: per-app loopback (a single program's sound, including its
//! helper processes) and microphones. Everything is converted to 48 kHz stereo f32.

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX, eCapture, eConsole,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{IUnknown, Interface, PCWSTR, Ref, implement};

use crate::align::{Aligner, SAMPLE_RATE};
use crate::clock::Clock;

#[derive(Clone, Debug)]
pub enum AudioSource {
    /// Everything a program and its child processes play.
    ProcessTree { pid: u32 },
    /// Everything except one program and its child processes.
    AllExceptTree { pid: u32 },
    /// A program found by executable name. The capture attaches when the program starts
    /// and follows it if it restarts.
    ProcessByName { exe: String },
    /// Whatever program `pid` currently holds (0 for none), set by the game detector.
    GameProcess { pid: Arc<AtomicU32> },
    /// Everything except the program `pid` holds, or except Umbra itself while it is 0.
    AllExceptGame { pid: Arc<AtomicU32> },
    /// A microphone or other capture device. `None` is the default one.
    Microphone { device_id: Option<String> },
    /// Every program except the game (`game`, 0 for none), the programs in `except` and
    /// Umbra, each captured on its own and summed (see `others`).
    OtherApps {
        game: Arc<AtomicU32>,
        except: Vec<String>,
    },
}

impl AudioSource {
    /// The concrete source to capture right now, or `None` if it isn't available yet.
    fn resolve(&self) -> Option<Resolved> {
        Some(match self {
            AudioSource::ProcessTree { pid } => Resolved::Include(*pid),
            AudioSource::AllExceptTree { pid } => Resolved::Exclude(*pid),
            AudioSource::ProcessByName { exe } => {
                Resolved::Include(crate::process::find_roots(exe).first().copied()?)
            }
            AudioSource::GameProcess { pid } => match pid.load(Ordering::Relaxed) {
                0 => return None,
                pid => Resolved::Include(pid),
            },
            AudioSource::AllExceptGame { pid } => match pid.load(Ordering::Relaxed) {
                0 => Resolved::Exclude(std::process::id()),
                pid => Resolved::Exclude(pid),
            },
            AudioSource::Microphone { device_id } => Resolved::Microphone(device_id.clone()),
            // Captured as a group by `others::run`, never as one stream.
            AudioSource::OtherApps { .. } => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Resolved {
    Include(u32),
    Exclude(u32),
    Microphone(Option<String>),
}

/// A running capture on its own thread. Dropping it stops the thread.
pub struct TrackWorker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Set when the source could not be opened and the track is silent instead.
    pub error: Option<String>,
    /// Why the source cannot be opened right now; cleared once it opens.
    problem: Arc<Mutex<Option<String>>>,
}

impl TrackWorker {
    /// Starts capturing `source` and calls `on_chunk(index, samples)` for every 1024-sample
    /// chunk. A source that can't be opened produces silence, so the track still exists
    /// and the clip keeps its track layout; `error` says why.
    pub fn spawn(
        source: AudioSource,
        clock: Clock,
        mut on_chunk: Box<dyn FnMut(u64, &[f32]) + Send>,
    ) -> Result<Self> {
        if let AudioSource::OtherApps { game, except } = &source {
            let (game, except) = (game.clone(), except.clone());
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread = std::thread::Builder::new()
                .name("umbra-other-apps".into())
                .spawn(move || {
                    crate::others::run(game, except, clock, thread_stop, &mut *on_chunk);
                })
                .context("cannot start audio thread")?;
            return Ok(Self {
                stop,
                thread: Some(thread),
                error: None,
                problem: Default::default(),
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let problem: Arc<Mutex<Option<String>>> = Default::default();
        let thread_problem = problem.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Option<String>>();

        let thread = std::thread::Builder::new()
            .name("umbra-audio".into())
            .spawn(move || unsafe {
                let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
                let mut aligner = Aligner::new();

                // The first attempt is reported to the caller; later ones (a program that
                // starts or restarts) happen quietly in the background.
                let mut opened: Option<(Resolved, Opened)> = None;
                let mut first_error = None;
                let mut next_attempt = Instant::now();
                let mut reported = false;
                let mut recheck = Instant::now();

                loop {
                    if thread_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let now = Instant::now();

                    // Follow the source: attach when it appears, switch when it changes.
                    if now >= recheck {
                        recheck = now + Duration::from_millis(500);
                        let wanted = source.resolve();
                        let current = opened.as_ref().map(|(r, _)| r);
                        if wanted.as_ref() != current {
                            if let Some((_, o)) = opened.take() {
                                let _ = o.client.Stop();
                                let _ = CloseHandle(o.event);
                            }
                            if let Some(target) = wanted {
                                if now >= next_attempt || !reported {
                                    match open_resolved(&target) {
                                        Ok(o) => {
                                            opened = Some((target, o));
                                            *thread_problem.lock().unwrap() = None;
                                        }
                                        Err(err) => {
                                            let message = format!("{err:#}");
                                            // Said once until the source opens: a missing
                                            // microphone is retried every 3 s, and logging each
                                            // try filled the log file within a day.
                                            let was = thread_problem
                                                .lock()
                                                .unwrap()
                                                .replace(err.to_string());
                                            if was.is_none() {
                                                log::warn!(
                                                    "audio source {source:?} unavailable: {message}"
                                                );
                                            }
                                            first_error.get_or_insert(message);
                                            next_attempt = now + Duration::from_secs(3);
                                        }
                                    }
                                }
                            } else {
                                // A program that is not running is no problem: the track
                                // waits for it.
                                *thread_problem.lock().unwrap() = None;
                                first_error.get_or_insert("the program is not running".to_string());
                            }
                        }
                        if !reported {
                            reported = true;
                            let _ = ready_tx.send(if opened.is_some() {
                                None
                            } else {
                                first_error.clone()
                            });
                        }
                    }

                    match opened.as_mut() {
                        Some((_, o)) => {
                            let wait = WaitForSingleObject(o.event, 20);
                            if wait == WAIT_OBJECT_0 || wait.0 == 0x102 {
                                if let Err(err) = o.drain(&clock, &mut aligner, &mut on_chunk) {
                                    log::warn!("audio capture stopped: {err:#}");
                                    if let Some((_, o)) = opened.take() {
                                        let _ = CloseHandle(o.event);
                                    }
                                    next_attempt = Instant::now() + Duration::from_secs(1);
                                }
                            }
                        }
                        None => std::thread::sleep(Duration::from_millis(20)),
                    }
                    aligner.tick(clock.now_secs(), |k, d| on_chunk(k, d));
                }
                if let Some((_, o)) = opened {
                    let _ = o.client.Stop();
                    let _ = CloseHandle(o.event);
                }
                if com {
                    CoUninitialize();
                }
            })
            .context("cannot start audio thread")?;

        let error = ready_rx.recv().unwrap_or(None);
        Ok(Self {
            stop,
            thread: Some(thread),
            error,
            problem,
        })
    }

    /// Why the track is silent right now because its source cannot be opened, if it is.
    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }
}

impl Drop for TrackWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Opened {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: HANDLE,
}

impl Opened {
    unsafe fn drain(
        &mut self,
        clock: &Clock,
        aligner: &mut Aligner,
        on_chunk: &mut Box<dyn FnMut(u64, &[f32]) + Send>,
    ) -> Result<()> {
        unsafe {
            loop {
                let available = self.capture.GetNextPacketSize()?;
                if available == 0 {
                    return Ok(());
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc = 0u64;
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))?;

                let count = frames as usize * 2;
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                // Without a trustworthy timestamp, place the packet at "now".
                let start = if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 || qpc == 0
                {
                    clock.now_secs() - frames as f64 / SAMPLE_RATE as f64
                } else {
                    clock.secs_from_qpc(qpc)
                };
                if silent || data.is_null() {
                    let zeros = vec![0.0f32; count];
                    aligner.push(start, &zeros, |k, d| on_chunk(k, d));
                } else {
                    let samples = std::slice::from_raw_parts(data as *const f32, count);
                    aligner.push(start, samples, |k, d| on_chunk(k, d));
                }
                self.capture.ReleaseBuffer(frames)?;
            }
        }
    }
}

fn wave_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: 2,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

unsafe fn open_resolved(source: &Resolved) -> Result<Opened> {
    unsafe {
        let (client, loopback) = match source {
            Resolved::Include(pid) => (activate_process_loopback(*pid, true)?, true),
            Resolved::Exclude(pid) => (activate_process_loopback(*pid, false)?, true),
            Resolved::Microphone(device_id) => (activate_microphone(device_id.as_deref())?, false),
        };

        let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        if loopback {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        // The client converts whatever the device or process plays into our format.
        flags |= AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        let format = wave_format();
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &format, None)
            .context("cannot initialise audio capture")?;
        let event = CreateEventW(None, false, false, PCWSTR::null())?;
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok(Opened {
            client,
            capture,
            event,
        })
    }
}

unsafe fn activate_microphone(device_id: Option<&str>) -> Result<IAudioClient> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = match device_id {
            Some(id) => {
                let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
                enumerator
                    .GetDevice(PCWSTR(wide.as_ptr()))
                    .with_context(|| format!("microphone `{id}`"))
                    .context("the chosen microphone is not connected; pick it again")?
            }
            None => enumerator
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .context("no default microphone")?,
        };
        Ok(device.Activate::<IAudioClient>(CLSCTX_ALL, None)?)
    }
}

#[implement(
    IActivateAudioInterfaceCompletionHandler,
    windows::Win32::System::Com::IAgileObject
)]
struct CompletionHandler {
    done: Mutex<Option<mpsc::Sender<()>>>,
}

// Marks the handler as callable from any thread, which ActivateAudioInterfaceAsync requires.
impl windows::Win32::System::Com::IAgileObject_Impl for CompletionHandler_Impl {}

impl IActivateAudioInterfaceCompletionHandler_Impl for CompletionHandler_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        if let Some(tx) = self.done.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

unsafe fn activate_process_loopback(pid: u32, include_tree: bool) -> Result<IAudioClient> {
    unsafe {
        if pid == 0 {
            bail!("no process to capture");
        }
        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: if include_tree {
                        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                    } else {
                        PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                    },
                },
            },
        };
        // Never dropped: PROPVARIANT's destructor would try to free the blob, which lives on
        // this stack frame.
        let variant = ManuallyDrop::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        blob: BLOB {
                            cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                            pBlobData: &mut params as *mut _ as *mut u8,
                        },
                    },
                }),
            },
        });

        let (tx, rx) = mpsc::channel();
        let handler: IActivateAudioInterfaceCompletionHandler = CompletionHandler {
            done: Mutex::new(Some(tx)),
        }
        .into();
        let operation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*variant),
            &handler,
        )
        .context("cannot start per-app audio capture (needs Windows 10 2004 or newer)")?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| anyhow!("timed out opening per-app audio capture"))?;
        let mut result = windows::core::HRESULT(0);
        let mut unknown: Option<IUnknown> = None;
        operation.GetActivateResult(&mut result, &mut unknown)?;
        result
            .ok()
            .context("Windows refused per-app audio capture for this process")?;
        let unknown = unknown.ok_or_else(|| anyhow!("no audio client returned"))?;
        let client = unknown.cast::<IAudioClient>()?;
        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_microphone_is_reported_and_silent() {
        let source = AudioSource::Microphone {
            device_id: Some("{0.0.1.00000000}.{00000000-0000-0000-0000-000000000000}".into()),
        };
        let chunks = Arc::new(AtomicU32::new(0));
        let counted = chunks.clone();
        let worker = TrackWorker::spawn(
            source,
            Clock::new(),
            Box::new(move |_, samples| {
                assert!(samples.iter().all(|&s| s == 0.0));
                counted.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
        let problem = worker.problem().expect("a problem is reported");
        assert!(problem.contains("not connected"), "{problem}");
        // The track still runs, as silence.
        std::thread::sleep(Duration::from_millis(400));
        assert!(chunks.load(Ordering::Relaxed) > 0);
    }
}
