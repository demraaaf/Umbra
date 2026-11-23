//! Fixed-rate frame pacing on a high-resolution waitable timer.

use std::time::{Duration, Instant};

use anyhow::Result;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
    TIMER_ALL_ACCESS, WaitForSingleObject,
};

pub struct Ticker {
    timer: HANDLE,
    start: Instant,
    interval: Duration,
    index: u64,
    started: bool,
    /// Frame 0 is due at this instant. Without one, it is due when `wait_next` is first called.
    epoch: Option<Instant>,
    /// Ticks skipped because the loop fell more than one interval behind.
    pub skipped: u64,
}

// The timer handle is only used by the thread that owns the Ticker.
unsafe impl Send for Ticker {}

impl Ticker {
    pub fn new(fps: u32) -> Result<Self> {
        Self::create(fps, None)
    }

    /// Frame `i` is due at `epoch + i / fps`, so the timeline lines up with other streams
    /// that count from the same epoch (audio).
    pub fn starting_at(fps: u32, epoch: Instant) -> Result<Self> {
        Self::create(fps, Some(epoch))
    }

    fn create(fps: u32, epoch: Option<Instant>) -> Result<Self> {
        let timer = unsafe {
            CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            )?
        };
        Ok(Self {
            timer,
            start: Instant::now(),
            interval: Duration::from_secs_f64(1.0 / fps as f64),
            index: 0,
            started: false,
            epoch,
            skipped: 0,
        })
    }

    /// Blocks until the next frame slot and returns its index. Indices are tied to wall
    /// time, so a late loop skips indices instead of drifting.
    pub fn wait_next(&mut self) -> u64 {
        if !self.started {
            self.started = true;
            match self.epoch {
                None => {
                    self.start = Instant::now();
                    return 0;
                }
                Some(epoch) => {
                    // First frame that is still in the future.
                    self.start = epoch;
                    let elapsed = epoch.elapsed();
                    self.index =
                        (elapsed.as_secs_f64() / self.interval.as_secs_f64()).ceil() as u64;
                    let target = self.start + self.interval.mul_f64(self.index as f64);
                    let now = Instant::now();
                    if target > now {
                        self.sleep(target - now);
                    }
                    return self.index;
                }
            }
        }
        self.index += 1;
        let now = Instant::now();
        let target = self.start + self.interval.mul_f64(self.index as f64);
        if target > now {
            self.sleep(target - now);
        } else {
            let behind = ((now - target).as_secs_f64() / self.interval.as_secs_f64()) as u64;
            self.index += behind;
            self.skipped += behind;
        }
        self.index
    }

    fn sleep(&self, duration: Duration) {
        // Negative due time means relative, in 100 ns units.
        let due = -((duration.as_nanos() / 100) as i64);
        if due == 0 {
            return;
        }
        unsafe {
            if SetWaitableTimer(self.timer, &due, 0, None, None, false).is_ok() {
                WaitForSingleObject(self.timer, INFINITE);
            } else {
                std::thread::sleep(duration);
            }
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.timer);
        }
    }
}
