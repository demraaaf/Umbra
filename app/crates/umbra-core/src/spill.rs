//! The replay buffer on disk instead of in memory. Packets are written to buffer files and the
//! buffer keeps only where each one is, so a long buffer at a high bitrate costs disk space
//! instead of RAM.
//!
//! The files are opened "delete on close": Windows removes them when the last reference goes,
//! even if Umbra crashes, so nothing is left behind. A file holds up to `SEGMENT_BYTES`; the
//! next packets start a new one, and an old one goes once the buffer has moved past it and no
//! clip being saved still reads from it. Files are marked temporary, which lets Windows keep
//! recent ones in its cache while memory is free and write them out only when it is needed.

use std::fs::{File, OpenOptions};
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use ffmpeg_next::Packet;
use ffmpeg_next::packet::Flags;

const SEGMENT_BYTES: u64 = 256 * 1024 * 1024;
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

/// One buffer file. Dropping the last reference closes it, and Windows deletes it.
struct Segment {
    file: File,
}

/// A packet kept on disk: where its bytes are, and what the muxer needs besides them.
pub struct DiskPacket {
    segment: Arc<Segment>,
    offset: u64,
    len: u32,
    pts: Option<i64>,
    dts: Option<i64>,
    duration: i64,
    flags: Flags,
}

impl DiskPacket {
    /// Reads the packet back.
    pub fn load(&self) -> Result<Packet> {
        let mut data = vec![0u8; self.len as usize];
        let mut done = 0;
        while done < data.len() {
            let read = self
                .segment
                .file
                .seek_read(&mut data[done..], self.offset + done as u64)
                .context("cannot read the replay buffer from disk")?;
            if read == 0 {
                bail!("the replay buffer file ended early");
            }
            done += read;
        }
        let mut packet = Packet::copy(&data);
        packet.set_pts(self.pts);
        packet.set_dts(self.dts);
        packet.set_duration(self.duration);
        packet.set_flags(self.flags);
        Ok(packet)
    }
}

/// Writes packets to buffer files in `dir`.
pub struct Spill {
    dir: PathBuf,
    current: Mutex<Option<(Arc<Segment>, u64)>>,
}

impl Spill {
    /// Checks that a buffer file can be made in `dir` (creating the folder).
    pub fn new(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot create the buffer folder {}", dir.display()))?;
        let spill = Self {
            dir: dir.to_path_buf(),
            current: Mutex::new(None),
        };
        let first = spill.open_segment()?;
        *spill.current.lock().unwrap() = Some((first, 0));
        Ok(spill)
    }

    fn open_segment(&self) -> Result<Arc<Segment>> {
        let name = format!(
            "umbra-buffer-{}-{}.tmp",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        );
        let path = self.dir.join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .attributes(FILE_ATTRIBUTE_TEMPORARY)
            .open(&path)
            .with_context(|| format!("cannot create the buffer file {}", path.display()))?;
        Ok(Arc::new(Segment { file }))
    }

    /// Writes the packet's bytes and returns where they are.
    pub fn store(&self, packet: &Packet) -> Result<DiskPacket> {
        let data = packet.data().unwrap_or(&[]);
        let mut current = self.current.lock().unwrap();
        let full = current
            .as_ref()
            .is_none_or(|(_, offset)| *offset + data.len() as u64 > SEGMENT_BYTES);
        if full {
            *current = Some((self.open_segment()?, 0));
        }
        let (segment, offset) = current.as_mut().unwrap();
        let mut done = 0;
        while done < data.len() {
            let wrote = segment
                .file
                .seek_write(&data[done..], *offset + done as u64)
                .context("cannot write the replay buffer to disk")?;
            if wrote == 0 {
                bail!("the disk is full");
            }
            done += wrote;
        }
        let stored = DiskPacket {
            segment: segment.clone(),
            offset: *offset,
            len: data.len() as u32,
            pts: packet.pts(),
            dts: packet.dts(),
            duration: packet.duration(),
            flags: packet.flags(),
        };
        *offset += data.len() as u64;
        Ok(stored)
    }
}

/// A packet in the replay buffer: in memory, or on disk.
pub enum Held {
    Memory(Packet),
    Disk(DiskPacket),
}

impl Held {
    /// Runs `use_it` with the packet, reading it from disk first if that is where it is.
    pub fn with<R>(&self, use_it: impl FnOnce(&Packet) -> Result<R>) -> Result<R> {
        match self {
            Held::Memory(packet) => use_it(packet),
            Held::Disk(stored) => use_it(&stored.load()?),
        }
    }

    pub fn duration(&self) -> i64 {
        match self {
            Held::Memory(packet) => packet.duration(),
            Held::Disk(stored) => stored.duration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_come_back_as_written_and_files_go_away() {
        let dir = std::env::temp_dir().join(format!("umbra-spill-test-{}", std::process::id()));
        let spill = Spill::new(&dir).unwrap();
        let mut stored = Vec::new();
        for i in 0..50u8 {
            let mut packet = Packet::copy(&vec![i; 1000 + i as usize]);
            packet.set_pts(Some(i as i64 * 10));
            packet.set_dts(Some(i as i64 * 10 - 5));
            packet.set_duration(10);
            if i % 10 == 0 {
                packet.set_flags(Flags::KEY);
            }
            stored.push(spill.store(&packet).unwrap());
        }
        for (i, s) in stored.iter().enumerate() {
            let back = s.load().unwrap();
            assert_eq!(back.data().unwrap(), &vec![i as u8; 1000 + i][..]);
            assert_eq!(back.pts(), Some(i as i64 * 10));
            assert_eq!(back.dts(), Some(i as i64 * 10 - 5));
            assert_eq!(back.duration(), 10);
            assert_eq!(back.is_key(), i % 10 == 0);
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        drop(stored);
        drop(spill);
        // Deleted on close.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir(&dir);
    }
}
