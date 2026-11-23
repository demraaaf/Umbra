//! Keyboard and mouse input kept with a clip, in a file of its own next to it
//! (`<name>.input`): what was held and pressed, and when, after the per-app rules
//! (`inputrules`) took out what must not be kept. A file of its own because MP4 has no place
//! for it (FFmpeg's MP4 muxer takes no data streams), because a recording's is written as it
//! goes, and because a clip shared by dragging it out or copying it then carries no input.
//!
//! The file is a header and then blocks, each a short stretch of time: what was held at its
//! start, then the events in it. Each block is checked by a CRC, so a file cut short by a
//! crash reads up to its last whole block. The same block encoding carries input through the
//! replay buffer.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const MAGIC: &[u8; 8] = b"UMBRAINP";
const VERSION: u16 = 1;
/// Times are kept in microseconds.
const TICKS: f64 = 1_000_000.0;
/// A block bigger than this is not one Umbra wrote.
const MAX_BLOCK: usize = 1 << 20;

/// The input file of `video`, if it has one. Only videos do: a screenshot can share a
/// clip's name.
pub fn existing_sidecar(video: &Path) -> Option<PathBuf> {
    let is_video = video
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
    let path = sidecar(video);
    (is_video && path.is_file()).then_some(path)
}

/// `videos` and the input files they have, to delete together.
pub fn with_sidecars(videos: &[PathBuf]) -> Vec<PathBuf> {
    let mut all = videos.to_vec();
    all.extend(videos.iter().filter_map(|v| existing_sidecar(v)));
    all
}

/// Where input sits in the replay buffer: a stream number no video file has.
pub const STREAM: usize = usize::MAX;

/// The input file of a clip or recording: next to it, named after it.
pub fn sidecar(video: &Path) -> PathBuf {
    video.with_extension("input")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    /// Windows virtual-key code.
    pub vk: u16,
    /// Keyboard scan code: where the key is, whatever the layout.
    pub scan: u16,
    /// One of the keys that share a scan code with another (right Ctrl, the arrows...).
    pub ext: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// The side buttons, back and forward.
    X1,
    X2,
}

impl Button {
    pub const ALL: [Button; 5] = [Self::Left, Self::Right, Self::Middle, Self::X1, Self::X2];

    fn bit(self) -> u8 {
        1 << self as u8
    }

    fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    KeyDown(Key),
    KeyUp(Key),
    /// A key that types text, kept without saying which.
    TextDown,
    TextUp,
    ButtonDown(Button),
    ButtonUp(Button),
    /// Wheel turns, 120 a notch: up and right are positive.
    Wheel {
        vertical: i32,
        horizontal: i32,
    },
    /// Mouse movement since the last, in the mouse's own counts.
    Move {
        dx: i32,
        dy: i32,
    },
}

/// What is held down at a moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pressed {
    /// Sorted.
    pub keys: Vec<Key>,
    /// Text keys held, without saying which.
    pub text: u32,
    /// One bit per [`Button`].
    pub buttons: u8,
}

impl Pressed {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.text == 0 && self.buttons == 0
    }

    pub fn holds(&self, button: Button) -> bool {
        self.buttons & button.bit() != 0
    }

    pub fn apply(&mut self, event: &Event) {
        match *event {
            Event::KeyDown(key) => {
                if let Err(at) = self.keys.binary_search(&key) {
                    self.keys.insert(at, key);
                }
            }
            Event::KeyUp(key) => {
                if let Ok(at) = self.keys.binary_search(&key) {
                    self.keys.remove(at);
                }
            }
            Event::TextDown => self.text += 1,
            Event::TextUp => self.text = self.text.saturating_sub(1),
            Event::ButtonDown(b) => self.buttons |= b.bit(),
            Event::ButtonUp(b) => self.buttons &= !b.bit(),
            Event::Wheel { .. } | Event::Move { .. } => {}
        }
    }
}

/// Input of a stretch of time: what was held at its start, then what happened, oldest first.
/// Times are seconds on whatever clock the block is read against.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub start: f64,
    pub pressed: Pressed,
    pub events: Vec<(f64, Event)>,
}

impl Block {
    /// Every time moved by `by` seconds.
    pub fn shifted(mut self, by: f64) -> Self {
        self.start += by;
        for (t, _) in &mut self.events {
            *t += by;
        }
        self
    }

    /// The block with times counted from `origin`.
    pub fn encode(&self, origin: f64) -> Vec<u8> {
        let ticks = |t: f64| ((t - origin) * TICKS).round() as i64;
        let mut out = Vec::with_capacity(16 + self.events.len() * 6);
        let start = ticks(self.start);
        put_signed(&mut out, start);
        put(&mut out, self.pressed.keys.len() as u64);
        for key in &self.pressed.keys {
            put_key(&mut out, key);
        }
        put(&mut out, u64::from(self.pressed.text));
        out.push(self.pressed.buttons);
        put(&mut out, self.events.len() as u64);
        let mut last = start;
        for (t, event) in &self.events {
            let at = ticks(*t).max(last);
            put(&mut out, (at - last) as u64);
            last = at;
            // Kind and the length of what follows, so a reader can skip kinds it does not know.
            let mut body = Vec::new();
            let kind = match *event {
                Event::KeyDown(key) => {
                    put_key(&mut body, &key);
                    1
                }
                Event::KeyUp(key) => {
                    put_key(&mut body, &key);
                    2
                }
                Event::TextDown => 3,
                Event::TextUp => 4,
                Event::ButtonDown(b) => {
                    body.push(b as u8);
                    5
                }
                Event::ButtonUp(b) => {
                    body.push(b as u8);
                    6
                }
                Event::Wheel {
                    vertical,
                    horizontal,
                } => {
                    put_signed(&mut body, i64::from(vertical));
                    put_signed(&mut body, i64::from(horizontal));
                    7
                }
                Event::Move { dx, dy } => {
                    put_signed(&mut body, i64::from(dx));
                    put_signed(&mut body, i64::from(dy));
                    8
                }
            };
            out.push(kind);
            out.push(body.len() as u8);
            out.extend_from_slice(&body);
        }
        out
    }

    /// Reads what [`encode`](Self::encode) wrote with the same `origin`.
    pub fn decode(bytes: &[u8], origin: f64) -> Result<Self> {
        let mut r = Reader { bytes, at: 0 };
        let secs = |ticks: i64| origin + ticks as f64 / TICKS;
        let start = r.signed()?;
        let mut pressed = Pressed::default();
        for _ in 0..r.count()? {
            pressed.keys.push(r.key()?);
        }
        pressed.keys.sort();
        pressed.text = r.varint()? as u32;
        pressed.buttons = r.byte()?;
        let count = r.count()?;
        let mut events = Vec::with_capacity(count);
        let mut last = start;
        for _ in 0..count {
            last += r.varint()? as i64;
            let kind = r.byte()?;
            let length = r.byte()? as usize;
            let body = r.take(length)?;
            let mut b = Reader { bytes: body, at: 0 };
            let button = |b: &mut Reader| -> Result<Button> {
                let index = b.byte()?;
                Button::from_index(index).context("unknown mouse button")
            };
            let event = match kind {
                1 => Event::KeyDown(b.key()?),
                2 => Event::KeyUp(b.key()?),
                3 => Event::TextDown,
                4 => Event::TextUp,
                5 => Event::ButtonDown(button(&mut b)?),
                6 => Event::ButtonUp(button(&mut b)?),
                7 => Event::Wheel {
                    vertical: b.signed()? as i32,
                    horizontal: b.signed()? as i32,
                },
                8 => Event::Move {
                    dx: b.signed()? as i32,
                    dy: b.signed()? as i32,
                },
                // From a newer Umbra: its length lets it be skipped.
                _ => continue,
            };
            events.push((secs(last), event));
        }
        Ok(Self {
            start: secs(start),
            pressed,
            events,
        })
    }
}

/// What is held at `t`, from the blocks (sorted by start): the last block that began by
/// then, and its events up to `t`. Before the first block, nothing.
pub fn pressed_at(blocks: &[Block], t: f64) -> Pressed {
    held(blocks, t, true)
}

/// Like [`pressed_at`], without the events at `t` itself.
fn pressed_before(blocks: &[Block], t: f64) -> Pressed {
    held(blocks, t, false)
}

fn held(blocks: &[Block], t: f64, including: bool) -> Pressed {
    let index = blocks.partition_point(|b| b.start <= t);
    let Some(block) = index.checked_sub(1).map(|i| &blocks[i]) else {
        return Pressed::default();
    };
    let mut pressed = block.pressed.clone();
    for (at, event) in &block.events {
        if *at > t || (!including && *at == t) {
            break;
        }
        pressed.apply(event);
    }
    pressed
}

/// The input of the stretches `from..to` of `blocks` (seconds on their clock), each moved
/// to start at `at` (seconds on the new file's clock): a clip of the buffer is one stretch,
/// an edit with parts cut out several. Each stretch starts with what was held just before
/// it, so a key held across a cut is not left down.
pub fn remap(blocks: &[Block], stretches: &[(f64, f64, f64)]) -> Vec<Block> {
    let mut out = Vec::new();
    for &(from, to, at) in stretches {
        if to <= from {
            continue;
        }
        let shift = at - from;
        let events: Vec<(f64, Event)> = blocks
            .iter()
            .filter(|b| b.start <= to)
            .flat_map(|b| b.events.iter())
            .filter(|(t, _)| *t >= from && *t < to)
            .map(|(t, e)| (t + shift, *e))
            .collect();
        let pressed = pressed_before(blocks, from);
        if events.is_empty() && pressed.is_empty() {
            continue;
        }
        out.push(Block {
            start: at,
            pressed,
            events,
        });
    }
    out
}

/// Writes an input file, block by block. Each block is flushed with the next, and on drop.
pub struct InputWriter {
    file: BufWriter<File>,
    path: PathBuf,
    blocks: u64,
}

impl InputWriter {
    pub fn create(path: &Path) -> Result<Self> {
        let file =
            File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
        let mut file = BufWriter::new(file);
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(&0u16.to_le_bytes())?;
        file.write_all(&(TICKS as u32).to_le_bytes())?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            blocks: 0,
        })
    }

    /// Adds `block`, its times counted from the start of the file's video.
    pub fn append(&mut self, block: &Block) -> Result<()> {
        let payload = block.encode(0.0);
        self.file.write_all(&(payload.len() as u32).to_le_bytes())?;
        self.file.write_all(&crc32(&payload).to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.blocks += 1;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        Ok(self.file.flush()?)
    }

    /// Finishes the file; one with no block is removed. Returns whether it is kept.
    pub fn finish(mut self) -> Result<bool> {
        self.file.flush()?;
        if self.blocks == 0 {
            let path = self.path.clone();
            drop(self);
            let _ = std::fs::remove_file(path);
            return Ok(false);
        }
        Ok(true)
    }
}

/// Writes `blocks` as the input file `path`. Nothing is written for none.
pub fn write(path: &Path, blocks: &[Block]) -> Result<()> {
    if blocks.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let mut writer = InputWriter::create(path)?;
    for block in blocks {
        writer.append(block)?;
    }
    writer.finish()?;
    Ok(())
}

/// The blocks of an input file, up to the last whole one (a file cut short by a crash).
pub fn read(path: &Path) -> Result<Vec<Block>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .read_to_end(&mut bytes)?;
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Result<Vec<Block>> {
    if bytes.len() < 16 || &bytes[..8] != MAGIC {
        bail!("not an input file");
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version > VERSION {
        bail!("the input file is from a newer Umbra (version {version})");
    }
    let ticks = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    if f64::from(ticks) != TICKS {
        bail!("the input file counts time in an unknown unit");
    }
    let mut blocks = Vec::new();
    let mut at = 16;
    while at + 8 <= bytes.len() {
        let length = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        let Some(payload) = bytes.get(at + 8..at + 8 + length) else {
            break;
        };
        if length > MAX_BLOCK || crc32(payload) != crc {
            break;
        }
        match Block::decode(payload, 0.0) {
            Ok(block) => blocks.push(block),
            Err(_) => break,
        }
        at += 8 + length;
    }
    blocks.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(blocks)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn put(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_signed(out: &mut Vec<u8>, value: i64) {
    put(out, ((value << 1) ^ (value >> 63)) as u64);
}

fn put_key(out: &mut Vec<u8>, key: &Key) {
    put(out, u64::from(key.vk));
    put(out, (u64::from(key.scan) << 1) | u64::from(key.ext));
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8> {
        let byte = *self.bytes.get(self.at).context("the block ends early")?;
        self.at += 1;
        Ok(byte)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let part = self
            .bytes
            .get(self.at..self.at + length)
            .context("the block ends early")?;
        self.at += length;
        Ok(part)
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        bail!("a number in the block is too long")
    }

    fn signed(&mut self) -> Result<i64> {
        let raw = self.varint()?;
        Ok((raw >> 1) as i64 ^ -((raw & 1) as i64))
    }

    /// A count of items, each at least a byte long.
    fn count(&mut self) -> Result<usize> {
        let count = self.varint()? as usize;
        if count > self.bytes.len() {
            bail!("the block counts more than it holds");
        }
        Ok(count)
    }

    fn key(&mut self) -> Result<Key> {
        let vk = self.varint()? as u16;
        let place = self.varint()?;
        Ok(Key {
            vk,
            scan: (place >> 1) as u16,
            ext: place & 1 == 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: Key = Key {
        vk: 0x57,
        scan: 0x11,
        ext: false,
    };
    const RIGHT_CTRL: Key = Key {
        vk: 0xA3,
        scan: 0x1D,
        ext: true,
    };

    fn sample(start: f64) -> Block {
        Block {
            start,
            pressed: Pressed {
                keys: vec![RIGHT_CTRL],
                text: 1,
                buttons: Button::Left.bit(),
            },
            events: vec![
                (start + 0.010, Event::KeyDown(W)),
                (start + 0.012, Event::TextUp),
                (start + 0.020, Event::Move { dx: -7, dy: 300 }),
                (start + 0.021, Event::ButtonUp(Button::Left)),
                (start + 0.030, Event::ButtonDown(Button::X2)),
                (
                    start + 0.040,
                    Event::Wheel {
                        vertical: -120,
                        horizontal: 240,
                    },
                ),
                (start + 0.050, Event::KeyUp(W)),
            ],
        }
    }

    fn close(a: &Block, b: &Block) {
        assert!(
            (a.start - b.start).abs() < 1e-6,
            "{} vs {}",
            a.start,
            b.start
        );
        assert_eq!(a.pressed, b.pressed);
        assert_eq!(a.events.len(), b.events.len());
        for ((ta, ea), (tb, eb)) in a.events.iter().zip(&b.events) {
            assert!((ta - tb).abs() < 1e-6);
            assert_eq!(ea, eb);
        }
    }

    #[test]
    fn a_block_reads_back_as_written() {
        let block = sample(1234.5);
        close(
            &Block::decode(&block.encode(1000.0), 1000.0).unwrap(),
            &block,
        );
        // Before the origin too (a block from just before a clip's start).
        let early = sample(0.25);
        close(&Block::decode(&early.encode(3.0), 3.0).unwrap(), &early);
    }

    #[test]
    fn unknown_kinds_of_event_are_skipped() {
        let mut bytes = Block {
            start: 0.0,
            pressed: Pressed::default(),
            events: vec![],
        }
        .encode(0.0);
        // Two events: one of a kind from a newer Umbra, then a text press.
        bytes.pop();
        bytes.extend_from_slice(&[2, 5, 99, 3, 1, 2, 3, 7, 3, 0]);
        let block = Block::decode(&bytes, 0.0).unwrap();
        assert_eq!(block.events.len(), 1);
        assert_eq!(block.events[0].1, Event::TextDown);
        assert!((block.events[0].0 - 12e-6).abs() < 1e-9);
    }

    #[test]
    fn a_file_reads_back_and_survives_a_torn_end() {
        let dir = std::env::temp_dir().join(format!("umbra-input-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.input");
        let blocks = [sample(0.0), sample(0.1), sample(0.2)];
        write(&path, &blocks).unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back.len(), 3);
        close(&back[2], &blocks[2]);
        // A crash in the middle of the last block: the others still read.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 3]).unwrap();
        assert_eq!(read(&path).unwrap().len(), 2);
        // A damaged block ends it there.
        let mut bad = bytes.clone();
        bad[30] ^= 0xff;
        std::fs::write(&path, &bad).unwrap();
        assert!(read(&path).unwrap().is_empty());
        // A newer version is refused rather than misread.
        let mut newer = bytes;
        newer[8] = 9;
        std::fs::write(&path, &newer).unwrap();
        assert!(read(&path).is_err());
        // Nothing to write leaves no file.
        write(&path, &[]).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_held_follows_the_events() {
        let blocks = vec![sample(10.0), sample(11.0)];
        assert!(pressed_at(&blocks, 9.0).is_empty());
        let at = pressed_at(&blocks, 10.011);
        assert_eq!(at.keys, vec![W, RIGHT_CTRL]);
        assert_eq!(at.text, 1);
        let at = pressed_at(&blocks, 10.035);
        assert!(!at.holds(Button::Left) && at.holds(Button::X2));
        assert_eq!(at.text, 0);
        assert_eq!(pressed_at(&blocks, 10.6).keys, vec![RIGHT_CTRL]);
        // The next block's start says what is held from then.
        assert!(pressed_at(&blocks, 11.0).holds(Button::Left));
    }

    #[test]
    fn a_clip_starts_with_what_was_held_and_counts_from_its_keyframe() {
        let blocks = vec![sample(10.0), sample(11.0)];
        // The clip asked for 10.015, but its video starts on the keyframe at 10.013.
        let clip = remap(&blocks, &[(10.013, 11.035, 0.0)]);
        assert_eq!(clip[0].start, 0.0);
        assert_eq!(clip[0].pressed.keys, vec![W, RIGHT_CTRL]);
        assert_eq!(clip[0].pressed.text, 0);
        let (t, event) = clip[0].events[0];
        assert!((t - 0.007).abs() < 1e-9, "{t}");
        assert_eq!(event, Event::Move { dx: -7, dy: 300 });
        // Nothing past the end.
        assert!(
            clip[0]
                .events
                .iter()
                .all(|(t, _)| *t <= 11.035 - 10.013 + 1e-9)
        );
        assert_eq!(
            clip[0].events.last().unwrap().1,
            Event::ButtonDown(Button::X2)
        );
    }

    #[test]
    fn an_event_right_at_a_cut_counts_once() {
        let text = |start: f64, held: u32| Block {
            start,
            pressed: Pressed {
                text: held,
                ..Pressed::default()
            },
            events: vec![(start, Event::TextDown)],
        };
        let blocks = vec![text(1.0, 0), text(2.0, 1)];
        // From exactly the second block's start: its press is in the clip, not before it.
        let clip = remap(&blocks, &[(2.0, 3.0, 0.0)]);
        assert_eq!(clip[0].pressed.text, 1);
        assert_eq!(clip[0].events, vec![(0.0, Event::TextDown)]);
        assert_eq!(pressed_at(&clip, 0.0).text, 2);
    }

    #[test]
    fn a_key_held_across_a_cut_is_not_left_down() {
        let blocks = vec![sample(0.0)];
        // W goes down at 0.010 and up at 0.050; the part from 0.045 to 0.055 is cut out.
        let edited = remap(&blocks, &[(0.0, 0.045, 0.0), (0.055, 1.0, 0.045)]);
        assert_eq!(edited.len(), 2);
        assert_eq!(edited[1].start, 0.045);
        assert!(pressed_at(&edited, 0.044).keys.contains(&W));
        // Its release was in the cut, yet it is not held after it.
        assert_eq!(pressed_at(&edited, 0.046).keys, vec![RIGHT_CTRL]);
    }
}
