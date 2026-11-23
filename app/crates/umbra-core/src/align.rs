//! Places captured audio on the recording timeline and cuts it into fixed-size chunks.

/// Samples per encoder frame. AAC uses 1024.
pub const CHUNK: usize = 1024;
pub const SAMPLE_RATE: u32 = 48_000;

/// Timestamps closer than this to where the last packet ended are trusted to be contiguous.
const JITTER: i64 = 960; // 20 ms
/// While a source is silent, timeline is filled with zeros up to this far behind the clock.
const FILL_LAG: f64 = 0.1;

/// Turns timestamped packets of interleaved stereo f32 into a gap-free stream of chunks
/// whose index counts from the clock's epoch: chunk `k` covers samples `k*1024..(k+1)*1024`.
pub struct Aligner {
    /// Samples appended so far, counted per channel.
    total: i64,
    pending: Vec<f32>,
    next_chunk: u64,
}

impl Aligner {
    pub fn new() -> Self {
        Self {
            total: 0,
            pending: Vec::with_capacity(CHUNK * 4),
            next_chunk: 0,
        }
    }

    /// Adds a packet that starts at `start_secs` on the recording clock. `emit` is called
    /// with each finished chunk.
    pub fn push(&mut self, start_secs: f64, samples: &[f32], mut emit: impl FnMut(u64, &[f32])) {
        let frames = (samples.len() / 2) as i64;
        let start = (start_secs * SAMPLE_RATE as f64).round() as i64;

        let mut data = samples;
        if self.total == 0 && self.next_chunk == 0 && self.pending.is_empty() && start < 0 {
            // Captured before the recording began: keep only the part after the epoch.
            let skip = (-start).min(frames);
            data = &samples[(skip * 2) as usize..];
        } else {
            let gap = start - self.total;
            if gap > JITTER {
                self.append_silence(gap, &mut emit);
            } else if gap < -JITTER {
                // Older than what is already written; skip the overlap.
                let skip = (-gap).min(frames);
                data = &samples[(skip * 2) as usize..];
            }
        }
        self.append(data, &mut emit);
    }

    /// Call regularly. When a source has been quiet for a while, fills the timeline with
    /// silence so the track keeps up with the clock.
    pub fn tick(&mut self, now_secs: f64, mut emit: impl FnMut(u64, &[f32])) {
        let target = ((now_secs - FILL_LAG) * SAMPLE_RATE as f64) as i64;
        if target > self.total {
            let gap = target - self.total;
            self.append_silence(gap, &mut emit);
        }
    }

    fn append_silence(&mut self, frames: i64, emit: &mut impl FnMut(u64, &[f32])) {
        // Done in pieces so a long gap doesn't allocate one huge buffer.
        let mut left = frames;
        let zeros = [0.0f32; CHUNK * 2];
        while left > 0 {
            let n = left.min(CHUNK as i64);
            self.append(&zeros[..(n * 2) as usize], emit);
            left -= n;
        }
    }

    fn append(&mut self, samples: &[f32], emit: &mut impl FnMut(u64, &[f32])) {
        self.total += (samples.len() / 2) as i64;
        self.pending.extend_from_slice(samples);
        let mut done = 0;
        while self.pending.len() - done >= CHUNK * 2 {
            emit(self.next_chunk, &self.pending[done..done + CHUNK * 2]);
            self.next_chunk += 1;
            done += CHUNK * 2;
        }
        self.pending.drain(..done);
    }
}

impl Default for Aligner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(f: impl FnOnce(&mut dyn FnMut(u64, &[f32]))) -> Vec<(u64, Vec<f32>)> {
        let mut out = Vec::new();
        f(&mut |k, data: &[f32]| out.push((k, data.to_vec())));
        out
    }

    #[test]
    fn chunks_are_numbered_from_the_epoch() {
        let mut aligner = Aligner::new();
        let ones = vec![1.0f32; 2 * SAMPLE_RATE as usize / 10];
        let out = collect(|emit| aligner.push(0.0, &ones, |k, d| emit(k, d)));
        // 4800 samples make four whole chunks and 704 left over.
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].0, 0);
        assert_eq!(out[3].0, 3);
        assert!(out.iter().all(|(_, d)| d.len() == CHUNK * 2));
    }

    #[test]
    fn late_start_is_padded_with_silence() {
        let mut aligner = Aligner::new();
        let ones = vec![1.0f32; CHUNK * 2];
        // The first packet starts a quarter second after the epoch.
        let out = collect(|emit| aligner.push(0.25, &ones, |k, d| emit(k, d)));
        let silent = out
            .iter()
            .filter(|(_, d)| d.iter().all(|&s| s == 0.0))
            .count();
        assert_eq!(silent, (0.25 * SAMPLE_RATE as f64 / CHUNK as f64) as usize);
    }

    #[test]
    fn gaps_are_filled_and_small_jitter_is_ignored() {
        let mut aligner = Aligner::new();
        let ten_ms = vec![0.5f32; 2 * 480];
        let mut count = 0;
        // Packets every 10 ms with ±2 ms of timestamp noise: nothing should be inserted or dropped.
        for i in 0..100 {
            let jitter = if i % 2 == 0 { 0.002 } else { -0.002 };
            aligner.push(i as f64 * 0.01 + jitter, &ten_ms, |_, _| count += 1);
        }
        assert_eq!(count, 480 * 100 / CHUNK);
    }

    #[test]
    fn tick_keeps_a_silent_source_running() {
        let mut aligner = Aligner::new();
        let mut chunks = 0;
        aligner.tick(2.1, |_, _| chunks += 1);
        // Two seconds of silence, since the fill stays 0.1 s behind the clock.
        assert_eq!(chunks, 2 * SAMPLE_RATE as usize / CHUNK);
    }

    #[test]
    fn audio_from_before_the_epoch_is_trimmed() {
        let mut aligner = Aligner::new();
        let data = vec![1.0f32; 2 * 4800];
        let mut first = None;
        aligner.push(-0.05, &data, |k, d| {
            first.get_or_insert((k, d[0]));
        });
        assert_eq!(first.unwrap().0, 0);
    }
}
