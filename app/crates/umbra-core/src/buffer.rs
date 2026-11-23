//! The replay buffer: a rolling window of encoded packets held in RAM.
//!
//! Packets are grouped into GOPs, each starting at a video keyframe. The oldest GOP is
//! dropped once the rest still cover the requested window, so the buffer always holds
//! at least `window` seconds and a clip can start on a keyframe without re-encoding.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::Sender;

pub struct Entry<P> {
    /// Index of the stream the packet belongs to.
    pub stream: usize,
    /// Presentation time in seconds on the shared recording clock.
    pub time: f64,
    pub size: usize,
    /// A video keyframe, where a file can start.
    pub key: bool,
    pub packet: P,
}

struct Gop<P> {
    start: f64,
    entries: Vec<Arc<Entry<P>>>,
    bytes: usize,
}

pub struct ReplayBuffer<P> {
    gops: VecDeque<Gop<P>>,
    window: f64,
    bytes: usize,
    newest: f64,
    /// Receives every packet the buffer keeps, as it arrives: a recording to a file.
    tap: Option<Sender<Arc<Entry<P>>>>,
}

pub struct Snapshot<P> {
    /// Time of the first video keyframe; the clip's zero point.
    pub start: f64,
    /// Time of the newest packet.
    pub end: f64,
    pub entries: Vec<Arc<Entry<P>>>,
}

impl<P> ReplayBuffer<P> {
    pub fn new(window_secs: f64) -> Self {
        Self {
            gops: VecDeque::new(),
            window: window_secs,
            bytes: 0,
            newest: 0.0,
            tap: None,
        }
    }

    /// Starts sending every kept packet to `tap`. False if a tap is already set.
    pub fn set_tap(&mut self, tap: Sender<Arc<Entry<P>>>) -> bool {
        if self.tap.is_some() {
            return false;
        }
        self.tap = Some(tap);
        true
    }

    /// Stops sending packets; the receiver sees the channel close.
    pub fn clear_tap(&mut self) {
        self.tap = None;
    }

    /// Adds a packet. A keyframe of the screen (stream 0) starts a new GOP; a keyframe of
    /// another video stream (the camera) is only marked. Other packets join the current GOP
    /// and are dropped if no keyframe has arrived yet, since a clip can't start without one.
    pub fn push(&mut self, stream: usize, time: f64, size: usize, video_keyframe: bool, packet: P) {
        if video_keyframe && stream == 0 {
            self.gops.push_back(Gop {
                start: time,
                entries: Vec::new(),
                bytes: 0,
            });
        }
        let Some(gop) = self.gops.back_mut() else {
            return;
        };
        let entry = Arc::new(Entry {
            stream,
            time,
            size,
            key: video_keyframe,
            packet,
        });
        if let Some(tap) = &self.tap {
            if tap.send(entry.clone()).is_err() {
                self.tap = None;
            }
        }
        gop.entries.push(entry);
        gop.bytes += size;
        self.bytes += size;
        self.newest = self.newest.max(time);
        self.trim();
    }

    fn trim(&mut self) {
        // Keep the newest GOP that starts at or before the window edge, and everything after.
        while self.gops.len() >= 2 && self.gops[1].start <= self.newest - self.window {
            if let Some(old) = self.gops.pop_front() {
                self.bytes -= old.bytes;
            }
        }
    }

    pub fn set_window(&mut self, window_secs: f64) {
        self.window = window_secs;
        self.trim();
    }

    /// Seconds of footage currently held.
    pub fn span_secs(&self) -> f64 {
        self.gops
            .front()
            .map_or(0.0, |gop| (self.newest - gop.start).max(0.0))
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.gops.is_empty()
    }

    pub fn clear(&mut self) {
        self.gops.clear();
        self.bytes = 0;
    }

    /// Cheap copy of everything in the buffer: only reference counts are cloned.
    pub fn snapshot(&self) -> Option<Snapshot<P>> {
        let first = self.gops.front()?;
        let entries = self
            .gops
            .iter()
            .flat_map(|gop| gop.entries.iter().cloned())
            .collect();
        Some(Snapshot {
            start: first.start,
            end: self.newest,
            entries,
        })
    }

    /// Like [`snapshot`](Self::snapshot), of `from` to `to` only: from the last keyframe at or
    /// before `from` (the oldest held, if `from` is older), up to `to`.
    pub fn snapshot_between(&self, from: f64, to: f64) -> Option<Snapshot<P>> {
        let first = self.gops.iter().rposition(|g| g.start <= from).unwrap_or(0);
        let start = self.gops.get(first)?.start;
        let entries: Vec<_> = self
            .gops
            .iter()
            .skip(first)
            .flat_map(|gop| gop.entries.iter())
            .filter(|e| e.time <= to)
            .cloned()
            .collect();
        let end = entries.iter().map(|e| e.time).fold(start, f64::max);
        (!entries.is_empty()).then_some(Snapshot {
            start,
            end,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of video at 10 fps per GOP: keyframe first, then nine delta frames.
    fn push_gop(buffer: &mut ReplayBuffer<()>, start_secs: u32) {
        for i in 0..10 {
            let time = start_secs as f64 + i as f64 / 10.0;
            buffer.push(0, time, 1000, i == 0, ());
        }
    }

    #[test]
    fn holds_at_least_the_window() {
        let mut buffer = ReplayBuffer::new(5.0);
        for second in 0..30 {
            push_gop(&mut buffer, second);
            let held = buffer.span_secs();
            if second >= 5 {
                assert!(held >= 5.0, "held only {held} s at second {second}");
                assert!(held < 6.5, "held {held} s at second {second}");
            }
        }
    }

    #[test]
    fn snapshot_starts_on_a_keyframe() {
        let mut buffer = ReplayBuffer::new(5.0);
        for second in 0..12 {
            push_gop(&mut buffer, second);
        }
        let snapshot = buffer.snapshot().unwrap();
        assert_eq!(snapshot.start, snapshot.entries[0].time);
        assert_eq!(snapshot.start.fract(), 0.0);
    }

    #[test]
    fn a_stretch_starts_on_the_keyframe_before_it() {
        let mut buffer = ReplayBuffer::new(20.0);
        for second in 0..20 {
            push_gop(&mut buffer, second);
            buffer.push(1, second as f64 + 0.5, 50, false, ());
        }
        let part = buffer.snapshot_between(7.4, 12.0).unwrap();
        assert_eq!(part.start, 7.0);
        assert_eq!(part.end, 12.0);
        assert!(part.entries.iter().all(|e| (7.0..=12.0).contains(&e.time)));
        assert_eq!(part.entries.iter().filter(|e| e.stream == 1).count(), 5);
        // Asking for more than is held gives what is held.
        let all = buffer.snapshot_between(-50.0, 100.0).unwrap();
        assert_eq!(all.entries.len(), buffer.snapshot().unwrap().entries.len());
        assert!(buffer.snapshot_between(-50.0, -40.0).is_none());
    }

    #[test]
    fn ignores_packets_before_the_first_keyframe() {
        let mut buffer: ReplayBuffer<()> = ReplayBuffer::new(5.0);
        buffer.push(1, 0.0, 100, false, ());
        assert!(buffer.is_empty());
        assert_eq!(buffer.bytes(), 0);
    }

    #[test]
    fn byte_count_matches_contents() {
        let mut buffer = ReplayBuffer::new(3.0);
        for second in 0..20 {
            push_gop(&mut buffer, second);
        }
        let snapshot = buffer.snapshot().unwrap();
        let total: usize = snapshot.entries.iter().map(|e| e.size).sum();
        assert_eq!(buffer.bytes(), total);
    }

    #[test]
    fn shrinking_the_window_drops_old_footage() {
        let mut buffer = ReplayBuffer::new(20.0);
        for second in 0..20 {
            push_gop(&mut buffer, second);
        }
        let before = buffer.span_secs();
        buffer.set_window(4.0);
        assert!(buffer.span_secs() < before);
        assert!(buffer.span_secs() >= 4.0);
    }

    #[test]
    fn a_tap_gets_every_kept_packet_once() {
        let mut buffer = ReplayBuffer::new(2.0);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.push(1, 0.0, 10, false, ());
        assert!(buffer.set_tap(tx.clone()));
        assert!(!buffer.set_tap(tx));
        for second in 0..5 {
            push_gop(&mut buffer, second);
        }
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got.len(), 50);
        assert!(got[0].key);
        buffer.clear_tap();
        push_gop(&mut buffer, 5);
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn audio_is_kept_with_its_gop() {
        let mut buffer = ReplayBuffer::new(2.0);
        for second in 0..10 {
            push_gop(&mut buffer, second);
            buffer.push(1, second as f64 + 0.5, 50, false, ());
        }
        let snapshot = buffer.snapshot().unwrap();
        let audio = snapshot.entries.iter().filter(|e| e.stream == 1).count();
        assert!(audio >= 2);
        assert!(snapshot.entries.iter().all(|e| e.time >= snapshot.start));
    }
}
