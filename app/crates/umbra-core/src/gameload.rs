//! A stand-in for a game, to test recording under load: it keeps the graphics card busy
//! rendering frames as fast as it can (like a game without a frame cap), keeps some processor
//! cores busy, and holds memory the way a game does. It measures its own frame times, so the
//! cost of recording shows up as frames per second lost, as it would in a game.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BUFFER_DESC,
    D3D11_QUERY_DATA_TIMESTAMP_DISJOINT, D3D11_QUERY_DESC, D3D11_QUERY_TIMESTAMP,
    D3D11_QUERY_TIMESTAMP_DISJOINT, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
    ID3D11Buffer, ID3D11DeviceContext, ID3D11PixelShader, ID3D11Query, ID3D11RenderTargetView,
    ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::{PCSTR, s};

use crate::d3d::Gpu;

/// How hard the stand-in game works.
#[derive(Clone, Copy, Debug)]
pub struct GameLoadSpec {
    /// Size it renders at, like the game's resolution.
    pub width: u32,
    pub height: u32,
    /// Frame time it aims for on its own, which sets how heavy each frame is.
    pub target_fps: u32,
    /// Processor threads kept busy.
    pub cpu_threads: usize,
    /// Memory held, in bytes.
    pub memory_bytes: u64,
}

pub struct GameLoad {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// Frame times in milliseconds since the last `take_frames`.
    frames: Arc<Mutex<Vec<f64>>>,
    error: Arc<Mutex<Option<String>>>,
}

const SHADER: &str = r#"
cbuffer Work : register(b0) { float time; uint steps; float2 unused; };
float4 vs(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}
float4 ps(float4 pos : SV_Position) : SV_Target {
    float3 c = float3(pos.xy * 0.0013, time);
    [loop] for (uint i = 0; i < steps; i++) {
        c = frac(sin(c * 12.9898 + c.yzx * 78.233) * 43758.5453);
    }
    return float4(c, 1);
}
"#;

impl GameLoad {
    /// Starts the load. The graphics part first finds how heavy a frame must be to reach
    /// `target_fps` on its own, which takes about a second.
    pub fn start(spec: GameLoadSpec) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(Mutex::new(Vec::new()));
        let error = Arc::new(Mutex::new(None));
        let mut threads = Vec::new();

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        {
            let (stop, frames, error) = (stop.clone(), frames.clone(), error.clone());
            threads.push(
                std::thread::Builder::new()
                    .name("umbra-game-gpu".into())
                    .spawn(move || {
                        if let Err(err) = gpu_load(spec, &stop, &frames, &ready_tx) {
                            let message = format!("{err:#}");
                            let _ = ready_tx.send(Err(anyhow!(message.clone())));
                            *error.lock().unwrap() = Some(message);
                        }
                    })?,
            );
        }
        for n in 0..spec.cpu_threads {
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name(format!("umbra-game-cpu-{n}"))
                    .spawn(move || cpu_load(&stop))?,
            );
        }
        if spec.memory_bytes > 0 {
            let stop = stop.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("umbra-game-memory".into())
                    .spawn(move || memory_load(spec.memory_bytes, &stop))?,
            );
        }
        let this = Self {
            stop,
            threads,
            frames,
            error,
        };
        match ready_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => Ok(this),
            Ok(Err(err)) => Err(err),
            Err(_) => bail!("the simulated game did not start"),
        }
    }

    /// The frame times since the last call, in milliseconds.
    pub fn take_frames(&self) -> Vec<f64> {
        std::mem::take(&mut *self.frames.lock().unwrap())
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}

impl Drop for GameLoad {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            None,
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(err) = result {
        let detail = errors
            .map(|e| unsafe {
                let bytes = std::slice::from_raw_parts(
                    e.GetBufferPointer() as *const u8,
                    e.GetBufferSize(),
                );
                String::from_utf8_lossy(bytes).into_owned()
            })
            .unwrap_or_default();
        bail!("cannot compile the test shader: {err} {detail}");
    }
    code.context("the shader compiler returned nothing")
}

fn bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

#[repr(C)]
struct Work {
    time: f32,
    steps: u32,
    unused: [f32; 2],
}

/// The stand-in game's frames. They are queued a few deep, as a game queues them for its
/// swap chain, and timed with the card's own clock: the time from one frame finishing to the
/// next is the frame time, including any time the card spends on other work in between (such
/// as recording). Waiting on each frame from the processor is not precise enough: once the
/// card is busy, Windows only reports a finished frame on its 15.6 ms timer tick.
struct Frames {
    context: ID3D11DeviceContext,
    constants: ID3D11Buffer,
    disjoint: Vec<ID3D11Query>,
    stamps: Vec<ID3D11Query>,
    issued: usize,
    last_end: Option<u64>,
    started: Instant,
}

const DEPTH: usize = 3;

impl Frames {
    /// Draws a frame of `steps` work and returns the frame time of the oldest one in flight,
    /// in milliseconds, once there is one.
    fn next(&mut self, steps: u32, stop: &AtomicBool) -> Option<f64> {
        let slot = self.issued % DEPTH;
        let mut measured = None;
        if self.issued >= DEPTH {
            let (frequency, disjoint, end) = self.collect(slot, stop)?;
            if let Some(prev) = self.last_end {
                if !disjoint && end > prev && frequency > 0 {
                    measured = Some((end - prev) as f64 * 1000.0 / frequency as f64);
                }
            }
            self.last_end = Some(end);
        }
        unsafe {
            let work = Work {
                time: self.started.elapsed().as_secs_f32(),
                steps,
                unused: [0.0; 2],
            };
            let c = &self.context;
            c.Begin(&self.disjoint[slot]);
            c.UpdateSubresource(
                &self.constants,
                0,
                None,
                &work as *const _ as *const c_void,
                0,
                0,
            );
            c.Draw(3, 0);
            c.End(&self.stamps[slot]);
            c.End(&self.disjoint[slot]);
            c.Flush();
        }
        self.issued += 1;
        measured
    }

    /// Waits for the frame in `slot`: the clock frequency, whether the clock jumped, and the
    /// time it finished.
    fn collect(&self, slot: usize, stop: &AtomicBool) -> Option<(u64, bool, u64)> {
        loop {
            let mut info = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
            let mut end = 0u64;
            let ready = unsafe {
                let a = self.context.GetData(
                    &self.disjoint[slot],
                    Some(&mut info as *mut _ as *mut c_void),
                    size_of::<D3D11_QUERY_DATA_TIMESTAMP_DISJOINT>() as u32,
                    0,
                );
                let b = self.context.GetData(
                    &self.stamps[slot],
                    Some(&mut end as *mut u64 as *mut c_void),
                    size_of::<u64>() as u32,
                    0,
                );
                // S_FALSE (not ready yet) is also a success code; the values say whether
                // the data arrived.
                a.is_ok() && b.is_ok() && info.Frequency > 0 && end > 0
            };
            if ready {
                return Some((info.Frequency, info.Disjoint.as_bool(), end));
            }
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            std::thread::yield_now();
        }
    }
}

fn gpu_load(
    spec: GameLoadSpec,
    stop: &AtomicBool,
    frames_out: &Mutex<Vec<f64>>,
    ready: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    // A device of its own, as a game has.
    let gpu = Gpu::new()?;
    let device = &gpu.device;
    let context = &gpu.context;
    let mut frames = unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: spec.width,
            Height: spec.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..Default::default()
        };
        let mut target: Option<ID3D11Texture2D> = None;
        device.CreateTexture2D(&desc, None, Some(&mut target))?;
        let target = target.context("no render target")?;
        let mut view: Option<ID3D11RenderTargetView> = None;
        device.CreateRenderTargetView(&target, None, Some(&mut view))?;
        let view = view.context("no render target view")?;

        let vs_code = compile(s!("vs"), s!("vs_5_0"))?;
        let ps_code = compile(s!("ps"), s!("ps_5_0"))?;
        let mut vs: Option<ID3D11VertexShader> = None;
        device.CreateVertexShader(bytes(&vs_code), None, Some(&mut vs))?;
        let mut ps: Option<ID3D11PixelShader> = None;
        device.CreatePixelShader(bytes(&ps_code), None, Some(&mut ps))?;

        let cb_desc = D3D11_BUFFER_DESC {
            ByteWidth: size_of::<Work>() as u32,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let mut constants: Option<ID3D11Buffer> = None;
        device.CreateBuffer(&cb_desc, None, Some(&mut constants))?;
        let constants = constants.context("no constant buffer")?;

        let query = |kind| -> Result<ID3D11Query> {
            let mut q: Option<ID3D11Query> = None;
            device.CreateQuery(
                &D3D11_QUERY_DESC {
                    Query: kind,
                    MiscFlags: 0,
                },
                Some(&mut q),
            )?;
            q.context("no query")
        };
        let mut disjoint = Vec::new();
        let mut stamps = Vec::new();
        for _ in 0..DEPTH {
            disjoint.push(query(D3D11_QUERY_TIMESTAMP_DISJOINT)?);
            stamps.push(query(D3D11_QUERY_TIMESTAMP)?);
        }

        context.OMSetRenderTargets(Some(&[Some(view.clone())]), None);
        context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: spec.width as f32,
            Height: spec.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context.VSSetShader(vs.as_ref(), None);
        context.PSSetShader(ps.as_ref(), None);
        context.PSSetConstantBuffers(0, Some(&[Some(constants.clone())]));

        Frames {
            context: context.clone(),
            constants,
            disjoint,
            stamps,
            issued: 0,
            last_end: None,
            started: Instant::now(),
        }
    };

    // Make each frame heavy enough that the card alone reaches about the target rate.
    let goal_ms = 1000.0 / spec.target_fps.max(1) as f64;
    let mut steps = 16u32;
    for _ in 0..14 {
        let mut times = Vec::new();
        while times.len() < 12 {
            if let Some(ms) = frames.next(steps, stop) {
                times.push(ms);
            }
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
        }
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2].max(0.005);
        let next = (steps as f64 * goal_ms / median)
            .round()
            .clamp(1.0, 50_000.0) as u32;
        if (next as f64 - steps as f64).abs() / steps as f64 <= 0.05 {
            break;
        }
        steps = next;
    }
    let _ = ready.send(Ok(()));

    while !stop.load(Ordering::Relaxed) {
        if let Some(ms) = frames.next(steps, stop) {
            frames_out.lock().unwrap().push(ms);
        }
    }
    Ok(())
}

fn cpu_load(stop: &AtomicBool) {
    let mut x = 1.000_1f64;
    while !stop.load(Ordering::Relaxed) {
        for _ in 0..200_000 {
            x = (x * 1.000_000_1).sin().abs() + 1.0;
        }
        std::hint::black_box(x);
    }
}

fn memory_load(bytes: u64, stop: &AtomicBool) {
    // Touched page by page, so Windows really gives it out.
    let mut block = vec![0u8; bytes as usize];
    for i in (0..block.len()).step_by(4096) {
        block[i] = 1;
    }
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    std::hint::black_box(&block);
}

/// Frames per second and 1 % low from frame times in milliseconds.
pub fn frame_stats(frames: &[f64]) -> (f64, f64) {
    if frames.is_empty() {
        return (0.0, 0.0);
    }
    let total: f64 = frames.iter().sum();
    let fps = frames.len() as f64 * 1000.0 / total.max(0.001);
    let mut sorted = frames.to_vec();
    sorted.sort_by(f64::total_cmp);
    // The 1 % low: the frame rate of the slowest one percent of frames.
    let worst = &sorted[((sorted.len() as f64 * 0.99) as usize).min(sorted.len() - 1)..];
    let worst_avg = worst.iter().sum::<f64>() / worst.len() as f64;
    (fps, 1000.0 / worst_avg.max(0.001))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rate_and_one_percent_low() {
        let mut frames = vec![5.0; 990];
        frames.extend([20.0; 10]);
        let (fps, low) = frame_stats(&frames);
        assert!((fps - 1000.0 / 5.15).abs() < 0.5, "{fps}");
        assert!((low - 50.0).abs() < 0.5, "{low}");
    }

    /// Runs the stand-in game on this PC for a few seconds:
    /// `cargo test -p umbra-core gameload -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reaches_about_the_target_rate() {
        let load = GameLoad::start(GameLoadSpec {
            width: 1920,
            height: 1080,
            target_fps: 200,
            cpu_threads: 2,
            memory_bytes: 256 << 20,
        })
        .unwrap();
        load.take_frames();
        std::thread::sleep(Duration::from_secs(3));
        let (fps, low) = frame_stats(&load.take_frames());
        println!("{fps:.0} fps, 1% low {low:.0}");
        assert!((120.0..320.0).contains(&fps), "{fps}");
    }
}
