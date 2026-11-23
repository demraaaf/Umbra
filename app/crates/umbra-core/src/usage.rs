//! What Umbra itself is using right now: CPU, memory, GPU and the GPU's video encoder.
//! The window's WebView2 helper processes are counted too, since they are part of the app.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::{PCWSTR, w};

use crate::process::list_processes;

#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    /// Share of the whole CPU, 0 to 100.
    pub cpu_percent: f64,
    /// Private memory of Umbra and its helper processes, in bytes.
    pub memory_bytes: u64,
    /// Busiest 3D/compute engine share, 0 to 100, if Windows reports GPU engines.
    pub gpu_percent: Option<f64>,
    /// Video encoder share, 0 to 100, if Windows reports it.
    pub encoder_percent: Option<f64>,
}

pub struct UsageSampler {
    query: Option<(PDH_HQUERY, PDH_HCOUNTER)>,
    last: Option<(Instant, Duration)>,
    cores: f64,
}

// The PDH handles are only used by the thread that owns the sampler.
unsafe impl Send for UsageSampler {}

impl UsageSampler {
    pub fn new() -> Self {
        let query = unsafe {
            let mut query = PDH_HQUERY::default();
            let mut counter = PDH_HCOUNTER::default();
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) == 0
                && PdhAddEnglishCounterW(
                    query,
                    w!(r"\GPU Engine(*)\Utilization Percentage"),
                    0,
                    &mut counter,
                ) == 0
            {
                // Rates need two readings; take the first one now.
                PdhCollectQueryData(query);
                Some((query, counter))
            } else {
                if !query.is_invalid() {
                    PdhCloseQuery(query);
                }
                None
            }
        };
        Self {
            query,
            last: None,
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()) as f64,
        }
    }

    /// Reads current usage. Call every second or two; the CPU figure is the average since the
    /// previous call.
    pub fn sample(&mut self) -> Usage {
        let pids = own_process_tree();
        let mut cpu_total = Duration::ZERO;
        let mut memory = 0u64;
        for &pid in &pids {
            if let Some((cpu, private)) = process_figures(pid) {
                cpu_total += cpu;
                memory += private;
            }
        }
        let now = Instant::now();
        let cpu_percent = match self.last {
            Some((when, before)) if now > when => {
                let busy = cpu_total.saturating_sub(before).as_secs_f64();
                (busy / (now - when).as_secs_f64() / self.cores * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        self.last = Some((now, cpu_total));

        let (gpu_percent, encoder_percent) = match self.query {
            Some((query, counter)) => gpu_engines(query, counter, &pids),
            None => (None, None),
        };
        Usage {
            cpu_percent,
            memory_bytes: memory,
            gpu_percent,
            encoder_percent,
        }
    }
}

impl Default for UsageSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for UsageSampler {
    fn drop(&mut self) {
        if let Some((query, _)) = self.query.take() {
            unsafe {
                PdhCloseQuery(query);
            }
        }
    }
}

/// This process and every process it started, directly or not.
fn own_process_tree() -> HashSet<u32> {
    let all = list_processes();
    let mut tree: HashSet<u32> = HashSet::from([std::process::id()]);
    loop {
        let before = tree.len();
        for p in &all {
            if tree.contains(&p.parent_pid) && p.pid != p.parent_pid {
                tree.insert(p.pid);
            }
        }
        if tree.len() == before {
            return tree;
        }
    }
}

/// CPU time used so far and private memory of one process.
fn process_figures(pid: u32) -> Option<(Duration, u64)> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let times = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let memory = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        );
        let _ = CloseHandle(handle);
        let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        let cpu = if times.is_ok() {
            Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
        } else {
            Duration::ZERO
        };
        let private = if memory.is_ok() {
            counters.PrivateUsage as u64
        } else {
            0
        };
        Some((cpu, private))
    }
}

/// Sums GPU engine use of our processes. Instance names look like
/// `pid_1234_luid_0x0_0x1_phys_0_eng_3_engtype_VideoEncode`.
fn gpu_engines(
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    pids: &HashSet<u32>,
) -> (Option<f64>, Option<f64>) {
    unsafe {
        if PdhCollectQueryData(query) != 0 {
            return (None, None);
        }
        let (mut size, mut count) = (0u32, 0u32);
        let status =
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
        if status != PDH_MORE_DATA || size == 0 {
            return (None, None);
        }
        // The buffer holds the items followed by their name strings.
        let items = size as usize / size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() + 1;
        let mut buffer: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = vec![Default::default(); items];
        if PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut size,
            &mut count,
            Some(buffer.as_mut_ptr()),
        ) != 0
        {
            return (None, None);
        }
        let mut per_engine_type: std::collections::HashMap<String, f64> = Default::default();
        for item in &buffer[..count as usize] {
            if item.FmtValue.CStatus != 0 {
                continue;
            }
            let name = item.szName.to_string().unwrap_or_default();
            let Some(pid) = name
                .strip_prefix("pid_")
                .and_then(|rest| rest.split('_').next())
                .and_then(|pid| pid.parse::<u32>().ok())
            else {
                continue;
            };
            if !pids.contains(&pid) {
                continue;
            }
            let engine_type = name.rsplit("engtype_").next().unwrap_or("").to_string();
            *per_engine_type.entry(engine_type).or_default() += item.FmtValue.Anonymous.doubleValue;
        }
        let encoder = per_engine_type
            .get("VideoEncode")
            .copied()
            .unwrap_or(0.0)
            .min(100.0);
        // Task Manager's "GPU" figure is the busiest engine type other than video.
        let gpu = per_engine_type
            .iter()
            .filter(|(kind, _)| !kind.starts_with("Video"))
            .map(|(_, value)| *value)
            .fold(0.0, f64::max)
            .min(100.0);
        (Some(gpu), Some(encoder))
    }
}
