//! Turning a recording written in fragments into an ordinary MP4 where it lies.
//!
//! A recording is written as a fragmented MP4 (an empty index, then `moof` + `mdat` pairs) so
//! a crash loses at most the last second. Players open such a file slowly, so it used to be
//! copied packet by packet into an ordinary MP4 once recording stopped: minutes and twice the
//! disk space for a long session. Here only the small `moof` boxes are read (the picture and
//! sound in the `mdat` boxes stay where they are), a complete index (`moov`) pointing at that
//! data is appended, and everything between the file type and the new index is wrapped in one
//! `mdat` box (16 bytes written over the old index's header), so a player finds the index in
//! three steps, as in a copied file. That takes about a second for any length.
//!
//! An earlier version renamed every `moof` to `free` instead. Players then walked thousands of
//! boxes to reach the index, each a separate request through the webview, which froze the app
//! for minutes on a long recording; [`compact`] rewraps such files.
//!
//! The file is changed in place, so the original bytes of everything changed are written to a
//! journal (`<file>.defrag`) first; [`recover`] puts them back after a crash part-way, leaving
//! the fragmented file as it was. Anything unexpected in the file makes [`finish_in_place`]
//! fail without changing it, and the caller copies it the old way.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};

/// One box: where it starts, its full size and the size of its header.
#[derive(Clone, Copy, Debug)]
struct Header {
    pos: u64,
    size: u64,
    kind: [u8; 4],
    len: u64,
}

impl Header {
    fn end(&self) -> u64 {
        self.pos + self.size
    }
    fn body(&self) -> u64 {
        self.pos + self.len
    }
}

/// Reads the box header at `pos`; `None` at the end of the file or when it is cut short.
fn header_at(file: &mut File, pos: u64, file_len: u64) -> Result<Option<Header>> {
    if pos + 8 > file_len {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(pos))?;
    let mut b = [0u8; 8];
    file.read_exact(&mut b)?;
    let size32 = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64;
    let kind = [b[4], b[5], b[6], b[7]];
    let (size, len) = match size32 {
        1 => {
            if pos + 16 > file_len {
                return Ok(None);
            }
            let mut l = [0u8; 8];
            file.read_exact(&mut l)?;
            (u64::from_be_bytes(l), 16)
        }
        0 => (file_len - pos, 8),
        n => (n, 8),
    };
    ensure!(size >= len, "a box at {pos} is smaller than its header");
    Ok(Some(Header {
        pos,
        size,
        kind,
        len,
    }))
}

/// The boxes inside `data` (a box's body), as (kind, whole box bytes, body bytes).
fn children(data: &[u8]) -> Result<Vec<([u8; 4], &[u8], &[u8])>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size32 = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap();
        let (size, len) = match size32 {
            1 => {
                ensure!(at + 16 <= data.len(), "a box is cut short");
                (
                    u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap()) as usize,
                    16,
                )
            }
            0 => (data.len() - at, 8),
            n => (n, 8),
        };
        ensure!(
            size >= len && at + size <= data.len(),
            "a box runs past its parent"
        );
        out.push((kind, &data[at..at + size], &data[at + len..at + size]));
        at += size;
    }
    Ok(out)
}

/// Big-endian reads from a box body.
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(
            self.at + n <= self.data.len(),
            "a box is shorter than its fields"
        );
        let s = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
}

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 16);
    if payload.len() + 8 <= u32::MAX as usize {
        out.extend(((payload.len() + 8) as u32).to_be_bytes());
        out.extend(kind);
    } else {
        out.extend(1u32.to_be_bytes());
        out.extend(kind);
        out.extend(((payload.len() + 16) as u64).to_be_bytes());
    }
    out.extend(payload);
    out
}

fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(payload.len() + 4);
    body.extend(((version as u32) << 24 | (flags & 0xff_ffff)).to_be_bytes());
    body.extend(payload);
    boxed(kind, &body)
}

#[derive(Clone, Copy, Default)]
struct Defaults {
    duration: u32,
    size: u32,
    flags: u32,
}

#[derive(Clone, Copy)]
struct Sample {
    size: u32,
    duration: u32,
    sync: bool,
    cto: i32,
}

struct Track {
    id: u32,
    /// The original `trak` box, whose sample tables are empty.
    trak: Vec<u8>,
    timescale: u32,
    defaults: Defaults,
    samples: Vec<Sample>,
    /// Each chunk: its file offset and how many samples it holds.
    chunks: Vec<(u64, u32)>,
    first_dts: Option<u64>,
    next_dts: u64,
    /// How long the track waits before it starts, in the movie's time scale: the empty edits
    /// the muxer put first in its edit list (`delay_moov` does that for a stream that starts
    /// late, such as the camera, rather than move its fragments' times).
    delay: u64,
}

/// What reading the file found, before anything is written.
struct Plan {
    tracks: Vec<Track>,
    movie_timescale: u32,
    /// The original `mvhd` and `moov` children other than `trak` and `mvex` (tags).
    mvhd: Vec<u8>,
    others: Vec<Vec<u8>>,
    ftyp: Header,
    moov: Header,
    moofs: Vec<u64>,
    /// Fragment random-access tables, which only make sense for a fragmented file.
    #[cfg_attr(not(test), allow(dead_code))]
    mfras: Vec<u64>,
    /// Where the complete fragments end; anything after (a fragment cut short by a crash)
    /// is dropped.
    keep: u64,
}

/// The length of the empty edits (media time -1) at the start of an `elst` box body.
fn leading_empty_edits(elst: &[u8]) -> Result<u64> {
    let mut c = Cursor::new(elst);
    let version = c.u32()? >> 24;
    let mut delay = 0;
    for _ in 0..c.u32()? {
        let (duration, media_time) = if version == 1 {
            (c.u64()?, c.u64()? as i64)
        } else {
            (u64::from(c.u32()?), i64::from(c.u32()? as i32))
        };
        c.u32()?;
        if media_time != -1 {
            break;
        }
        delay += duration;
    }
    Ok(delay)
}

fn parse_moov(body: &[u8]) -> Result<(u32, Vec<u8>, Vec<Track>, Vec<Vec<u8>>)> {
    let mut timescale = 0;
    let mut mvhd = Vec::new();
    let mut tracks = Vec::new();
    let mut others = Vec::new();
    let mut trex = Vec::new();
    for (kind, whole, inner) in children(body)? {
        match &kind {
            b"mvhd" => {
                let mut c = Cursor::new(inner);
                let version = c.u32()? >> 24;
                if version == 1 {
                    c.take(16)?;
                } else {
                    c.take(8)?;
                }
                timescale = c.u32()?;
                mvhd = whole.to_vec();
            }
            b"trak" => {
                let mut id = 0;
                let mut media_timescale = 0;
                let mut delay = 0;
                for (k, _, b) in children(inner)? {
                    if &k == b"edts" {
                        for (k2, _, b2) in children(b)? {
                            if &k2 == b"elst" {
                                delay = leading_empty_edits(b2)?;
                            }
                        }
                    } else if &k == b"tkhd" {
                        let mut c = Cursor::new(b);
                        let version = c.u32()? >> 24;
                        c.take(if version == 1 { 16 } else { 8 })?;
                        id = c.u32()?;
                    } else if &k == b"mdia" {
                        for (k2, _, b2) in children(b)? {
                            if &k2 == b"mdhd" {
                                let mut c = Cursor::new(b2);
                                let version = c.u32()? >> 24;
                                c.take(if version == 1 { 16 } else { 8 })?;
                                media_timescale = c.u32()?;
                            }
                        }
                    }
                }
                ensure!(
                    id > 0 && media_timescale > 0,
                    "a track has no id or time scale"
                );
                tracks.push(Track {
                    id,
                    trak: whole.to_vec(),
                    timescale: media_timescale,
                    defaults: Defaults::default(),
                    samples: Vec::new(),
                    chunks: Vec::new(),
                    first_dts: None,
                    next_dts: 0,
                    delay,
                });
            }
            b"mvex" => {
                for (k, _, b) in children(inner)? {
                    if &k == b"trex" {
                        let mut c = Cursor::new(b);
                        c.u32()?;
                        let id = c.u32()?;
                        let description = c.u32()?;
                        ensure!(description <= 1, "more than one sample description");
                        trex.push((
                            id,
                            Defaults {
                                duration: c.u32()?,
                                size: c.u32()?,
                                flags: c.u32()?,
                            },
                        ));
                    }
                }
            }
            _ => others.push(whole.to_vec()),
        }
    }
    ensure!(
        timescale > 0 && !tracks.is_empty(),
        "the index has no tracks"
    );
    for (id, defaults) in trex {
        if let Some(track) = tracks.iter_mut().find(|t| t.id == id) {
            track.defaults = defaults;
        }
    }
    Ok((timescale, mvhd, tracks, others))
}

/// The samples of one `moof`, per track index, before they are known to be complete.
type Fragment = Vec<(usize, Option<u64>, Vec<Sample>, Vec<(u64, u32)>)>;

fn parse_moof(body: &[u8], moof_pos: u64, tracks: &[Track]) -> Result<Fragment> {
    let mut out: Fragment = Vec::new();
    // Without an explicit base, a traf's data follows the previous traf's.
    let mut previous_end = moof_pos;
    let mut first_traf = true;
    for (kind, _, inner) in children(body)? {
        if &kind != b"traf" {
            continue;
        }
        let mut index = None;
        let mut defaults = Defaults::default();
        let mut base = 0u64;
        let mut tfdt = None;
        let mut samples = Vec::new();
        let mut chunks = Vec::new();
        let mut cursor_pos: Option<u64> = None;
        for (k, _, b) in children(inner)? {
            match &k {
                b"tfhd" => {
                    let mut c = Cursor::new(b);
                    let flags = c.u32()? & 0xff_ffff;
                    let id = c.u32()?;
                    let i = tracks
                        .iter()
                        .position(|t| t.id == id)
                        .ok_or_else(|| anyhow!("a fragment names unknown track {id}"))?;
                    index = Some(i);
                    defaults = tracks[i].defaults;
                    base = if flags & 0x1 != 0 {
                        c.u64()?
                    } else if flags & 0x2_0000 != 0 || first_traf {
                        moof_pos
                    } else {
                        previous_end
                    };
                    if flags & 0x2 != 0 {
                        ensure!(c.u32()? <= 1, "more than one sample description");
                    }
                    if flags & 0x8 != 0 {
                        defaults.duration = c.u32()?;
                    }
                    if flags & 0x10 != 0 {
                        defaults.size = c.u32()?;
                    }
                    if flags & 0x20 != 0 {
                        defaults.flags = c.u32()?;
                    }
                    ensure!(flags & 0x1_0000 == 0, "an empty fragment duration");
                }
                b"tfdt" => {
                    let mut c = Cursor::new(b);
                    let version = c.u32()? >> 24;
                    tfdt = Some(if version == 1 {
                        c.u64()?
                    } else {
                        c.u32()? as u64
                    });
                }
                b"trun" => {
                    ensure!(index.is_some(), "a run before its fragment header");
                    let mut c = Cursor::new(b);
                    let vf = c.u32()?;
                    let (version, flags) = (vf >> 24, vf & 0xff_ffff);
                    let count = c.u32()?;
                    let mut pos = match (flags & 0x1 != 0, cursor_pos) {
                        (true, _) => base
                            .checked_add_signed(c.u32()? as i32 as i64)
                            .context("a bad data offset")?,
                        (false, Some(p)) => p,
                        (false, None) => base,
                    };
                    let first_flags = if flags & 0x4 != 0 {
                        Some(c.u32()?)
                    } else {
                        None
                    };
                    let start = pos;
                    for n in 0..count {
                        let duration = if flags & 0x100 != 0 {
                            c.u32()?
                        } else {
                            defaults.duration
                        };
                        let size = if flags & 0x200 != 0 {
                            c.u32()?
                        } else {
                            defaults.size
                        };
                        let sample_flags = if flags & 0x400 != 0 {
                            c.u32()?
                        } else if n == 0 && first_flags.is_some() {
                            first_flags.unwrap()
                        } else {
                            defaults.flags
                        };
                        let cto = if flags & 0x800 != 0 {
                            let v = c.u32()?;
                            if version == 0 {
                                v.min(i32::MAX as u32) as i32
                            } else {
                                v as i32
                            }
                        } else {
                            0
                        };
                        samples.push(Sample {
                            size,
                            duration,
                            sync: sample_flags & 0x1_0000 == 0,
                            cto,
                        });
                        pos += size as u64;
                    }
                    if count > 0 {
                        chunks.push((start, count));
                    }
                    cursor_pos = Some(pos);
                    previous_end = pos;
                }
                _ => {}
            }
        }
        if let Some(i) = index {
            out.push((i, tfdt, samples, chunks));
        }
        first_traf = false;
    }
    Ok(out)
}

/// Reads the file's layout. Fails on anything this does not handle.
fn plan(file: &mut File, progress: &mut dyn FnMut(u64)) -> Result<Plan> {
    let file_len = file.metadata()?.len();
    let ftyp = header_at(file, 0, file_len)?.context("the file is empty")?;
    ensure!(
        &ftyp.kind == b"ftyp" && ftyp.size >= 16 && ftyp.size <= 64,
        "no file type box"
    );
    let moov = header_at(file, ftyp.end(), file_len)?.context("no index")?;
    ensure!(
        &moov.kind == b"moov" && moov.end() <= file_len && moov.size < 16 << 20,
        "no index after the file type"
    );
    let mut body = vec![0u8; (moov.size - moov.len) as usize];
    file.seek(SeekFrom::Start(moov.body()))?;
    file.read_exact(&mut body)?;
    let (movie_timescale, mvhd, mut tracks, others) = parse_moov(&body)?;

    let mut moofs = Vec::new();
    let mut mfras = Vec::new();
    let mut pending: Option<(u64, Fragment)> = None;
    let mut pos = moov.end();
    let mut keep = pos;
    let mut steps = 0u32;
    while let Some(h) = header_at(file, pos, file_len)? {
        if h.end() > file_len {
            break;
        }
        match &h.kind {
            b"moof" => {
                ensure!(h.size < 64 << 20, "a fragment header is too big");
                let mut body = vec![0u8; (h.size - h.len) as usize];
                file.seek(SeekFrom::Start(h.body()))?;
                file.read_exact(&mut body)?;
                // A fragment whose data never arrived is dropped with what follows.
                if let Some((at, _)) = pending.take() {
                    keep = at;
                    break;
                }
                pending = Some((h.pos, parse_moof(&body, h.pos, &tracks)?));
            }
            b"mdat" => {
                if let Some((at, fragment)) = pending.take() {
                    for (i, tfdt, samples, chunks) in fragment {
                        let track = &mut tracks[i];
                        if let Some(dts) = tfdt {
                            match track.first_dts {
                                None => track.first_dts = Some(dts),
                                // A gap or an overlap: the previous sample lasts until this one.
                                Some(_) if dts != track.next_dts => {
                                    if let Some(last) = track.samples.last_mut() {
                                        let start = track.next_dts - last.duration as u64;
                                        ensure!(dts > start, "fragment times go backwards");
                                        last.duration =
                                            u32::try_from(dts - start).context("a gap too long")?;
                                    }
                                }
                                Some(_) => {}
                            }
                            track.next_dts = dts;
                        } else if track.first_dts.is_none() {
                            track.first_dts = Some(0);
                        }
                        for s in &samples {
                            track.next_dts += s.duration as u64;
                        }
                        track.samples.extend(samples);
                        track.chunks.extend(chunks);
                    }
                    moofs.push(at);
                }
                keep = h.end();
            }
            b"mfra" => {
                mfras.push(h.pos);
                keep = h.end();
            }
            b"free" | b"skip" => keep = h.end(),
            other => bail!("an unexpected {} box", String::from_utf8_lossy(other)),
        }
        pos = h.end();
        steps += 1;
        if steps % 64 == 0 {
            progress(pos);
        }
    }
    if let Some((at, _)) = pending {
        keep = at;
    }
    ensure!(!moofs.is_empty(), "no complete fragment");
    // Every sample's data must lie inside what is kept.
    for track in &tracks {
        let mut i = 0usize;
        for &(start, count) in &track.chunks {
            let bytes: u64 = track.samples[i..i + count as usize]
                .iter()
                .map(|s| s.size as u64)
                .sum();
            ensure!(start + bytes <= keep, "data past the end of the file");
            i += count as usize;
        }
    }
    Ok(Plan {
        tracks,
        movie_timescale,
        mvhd,
        others,
        ftyp,
        moov,
        moofs,
        mfras,
        keep,
    })
}

fn rescale(value: u64, from: u32, to: u32) -> u64 {
    (value as u128 * to as u128 / from.max(1) as u128) as u64
}

/// The sample tables, edit list and durations of one track.
fn build_trak(track: &Track, movie_timescale: u32) -> Result<(Vec<u8>, u64)> {
    let samples = &track.samples;
    let media_duration: u64 = samples.iter().map(|s| s.duration as u64).sum();
    let start =
        track.first_dts.unwrap_or(0) + rescale(track.delay, movie_timescale, track.timescale);
    let movie_duration = rescale(start + media_duration, track.timescale, movie_timescale);

    // stts
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for s in samples {
        match runs.last_mut() {
            Some((n, d)) if *d == s.duration => *n += 1,
            _ => runs.push((1, s.duration)),
        }
    }
    let mut stts = Vec::new();
    stts.extend((runs.len() as u32).to_be_bytes());
    for (n, d) in &runs {
        stts.extend(n.to_be_bytes());
        stts.extend(d.to_be_bytes());
    }
    let mut tables = full_box(b"stts", 0, 0, &stts);

    // ctts, when frames are shown in another order than stored
    if samples.iter().any(|s| s.cto != 0) {
        let mut runs: Vec<(u32, i32)> = Vec::new();
        for s in samples {
            match runs.last_mut() {
                Some((n, o)) if *o == s.cto => *n += 1,
                _ => runs.push((1, s.cto)),
            }
        }
        let negative = runs.iter().any(|(_, o)| *o < 0);
        let mut ctts = Vec::new();
        ctts.extend((runs.len() as u32).to_be_bytes());
        for (n, o) in &runs {
            ctts.extend(n.to_be_bytes());
            ctts.extend(o.to_be_bytes());
        }
        tables.extend(full_box(b"ctts", u8::from(negative), 0, &ctts));
    }

    // stss, unless every sample is a keyframe
    if samples.iter().any(|s| !s.sync) {
        let sync: Vec<u32> = samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync)
            .map(|(i, _)| i as u32 + 1)
            .collect();
        let mut stss = Vec::new();
        stss.extend((sync.len() as u32).to_be_bytes());
        for n in sync {
            stss.extend(n.to_be_bytes());
        }
        tables.extend(full_box(b"stss", 0, 0, &stss));
    }

    // stsc
    let mut stsc_runs: Vec<(u32, u32)> = Vec::new();
    for (i, &(_, count)) in track.chunks.iter().enumerate() {
        if stsc_runs.last().is_none_or(|(_, n)| *n != count) {
            stsc_runs.push((i as u32 + 1, count));
        }
    }
    let mut stsc = Vec::new();
    stsc.extend((stsc_runs.len() as u32).to_be_bytes());
    for (first, n) in &stsc_runs {
        stsc.extend(first.to_be_bytes());
        stsc.extend(n.to_be_bytes());
        stsc.extend(1u32.to_be_bytes());
    }
    tables.extend(full_box(b"stsc", 0, 0, &stsc));

    // stsz
    let mut stsz = Vec::new();
    let same = samples
        .first()
        .map(|s| s.size)
        .filter(|&size| samples.iter().all(|s| s.size == size));
    stsz.extend(same.unwrap_or(0).to_be_bytes());
    stsz.extend((samples.len() as u32).to_be_bytes());
    if same.is_none() {
        for s in samples {
            stsz.extend(s.size.to_be_bytes());
        }
    }
    tables.extend(full_box(b"stsz", 0, 0, &stsz));

    // stco or co64
    let big = track.chunks.iter().any(|(o, _)| *o > u32::MAX as u64);
    let mut offsets = Vec::new();
    offsets.extend((track.chunks.len() as u32).to_be_bytes());
    for (o, _) in &track.chunks {
        if big {
            offsets.extend(o.to_be_bytes());
        } else {
            offsets.extend((*o as u32).to_be_bytes());
        }
    }
    tables.extend(full_box(
        if big { b"co64" } else { b"stco" },
        0,
        0,
        &offsets,
    ));

    // Edit list: an empty edit while the track has not started yet, then all of it.
    let mut elst = Vec::new();
    let media_in_movie = rescale(media_duration, track.timescale, movie_timescale);
    let mut entries: Vec<(u64, i64)> = Vec::new();
    if start > 0 {
        entries.push((rescale(start, track.timescale, movie_timescale), -1));
    }
    entries.push((media_in_movie, 0));
    let wide = entries.iter().any(|(d, _)| *d > u32::MAX as u64);
    elst.extend((entries.len() as u32).to_be_bytes());
    for (duration, media_time) in entries {
        if wide {
            elst.extend(duration.to_be_bytes());
            elst.extend(media_time.to_be_bytes());
        } else {
            elst.extend((duration as u32).to_be_bytes());
            elst.extend((media_time as i32).to_be_bytes());
        }
        elst.extend(0x0001_0000u32.to_be_bytes());
    }
    let edts = boxed(b"edts", &full_box(b"elst", u8::from(wide), 0, &elst));

    // Rebuild the trak around the new tables.
    let (_, _, trak_body) = children(&track.trak)?
        .into_iter()
        .next()
        .context("an empty track")?;
    let mut out = Vec::new();
    for (kind, whole, inner) in children(trak_body)? {
        match &kind {
            b"tkhd" => out.extend(with_duration(b"tkhd", inner, movie_duration, 16)?),
            b"edts" => {}
            b"mdia" => {
                out.extend(&edts);
                let mut mdia = Vec::new();
                for (k, w, b) in children(inner)? {
                    match &k {
                        b"mdhd" => mdia.extend(with_duration(b"mdhd", b, media_duration, 12)?),
                        b"minf" => {
                            let mut minf = Vec::new();
                            for (k2, w2, b2) in children(b)? {
                                if &k2 == b"stbl" {
                                    let mut stbl = Vec::new();
                                    for (k3, w3, _) in children(b2)? {
                                        if &k3 == b"stsd" {
                                            stbl.extend(w3);
                                        }
                                    }
                                    ensure!(!stbl.is_empty(), "a track has no sample description");
                                    stbl.extend(&tables);
                                    minf.extend(boxed(b"stbl", &stbl));
                                } else {
                                    minf.extend(w2);
                                }
                            }
                            mdia.extend(boxed(b"minf", &minf));
                        }
                        _ => mdia.extend(w),
                    }
                }
                out.extend(boxed(b"mdia", &mdia));
            }
            _ => out.extend(whole),
        }
    }
    Ok((boxed(b"trak", &out), movie_duration))
}

/// A `tkhd`, `mdhd` or `mvhd` with its duration set, made version 1 when it does not fit
/// version 0. `field` is where the duration sits in a version 0 body, after the version.
fn with_duration(kind: &[u8; 4], body: &[u8], duration: u64, field: usize) -> Result<Vec<u8>> {
    let version = body.first().copied().context("an empty header box")?;
    let flags = u32::from_be_bytes(body[0..4].try_into().unwrap()) & 0xff_ffff;
    let rest = &body[4..];
    if version == 1 {
        // Creation and modification time are 8 bytes each, so the duration sits 8 further.
        let at = field + 8;
        let mut out = rest.to_vec();
        ensure!(out.len() >= at + 8, "a header box is too short");
        out[at..at + 8].copy_from_slice(&duration.to_be_bytes());
        return Ok(full_box(kind, 1, flags, &out));
    }
    ensure!(rest.len() >= field + 4, "a header box is too short");
    if let Ok(d) = u32::try_from(duration) {
        let mut out = rest.to_vec();
        out[field..field + 4].copy_from_slice(&d.to_be_bytes());
        return Ok(full_box(kind, 0, flags, &out));
    }
    // Widen: creation and modification time (and the field before the duration) to 8 bytes.
    let mut out = Vec::new();
    out.extend((rest[0..4].try_into().map(u32::from_be_bytes).unwrap() as u64).to_be_bytes());
    out.extend((rest[4..8].try_into().map(u32::from_be_bytes).unwrap() as u64).to_be_bytes());
    out.extend(&rest[8..field]);
    out.extend(duration.to_be_bytes());
    out.extend(&rest[field + 4..]);
    Ok(full_box(kind, 1, flags, &out))
}

fn build_moov(plan: &Plan) -> Result<Vec<u8>> {
    let mut traks = Vec::new();
    let mut duration = 0u64;
    for track in &plan.tracks {
        let (trak, d) = build_trak(track, plan.movie_timescale)?;
        traks.extend(trak);
        duration = duration.max(d);
    }
    let (_, _, mvhd_body) = children(&plan.mvhd)?
        .into_iter()
        .next()
        .context("no movie header")?;
    let mut body = with_duration(b"mvhd", mvhd_body, duration, 12)?;
    body.extend(traks);
    for other in &plan.others {
        body.extend(other);
    }
    Ok(boxed(b"moov", &body))
}

fn journal_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".defrag");
    PathBuf::from(name)
}

/// Puts back a file whose conversion was interrupted (a journal next to it), so it is the
/// fragmented recording it was. Returns true when there was something to put back.
pub fn recover(path: &Path) -> Result<bool> {
    let journal = journal_path(path);
    if !journal.exists() {
        return Ok(false);
    }
    let reader = BufReader::new(File::open(&journal)?);
    let mut lines = reader.lines();
    let len: u64 = lines.next().context("an empty journal")??.trim().parse()?;
    let mut file = OpenOptions::new().write(true).open(path)?;
    for line in lines {
        let line = line?;
        let Some((at, hex)) = line.split_once(' ') else {
            continue;
        };
        let bytes: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<std::result::Result<_, _>>()?;
        file.seek(SeekFrom::Start(at.parse()?))?;
        file.write_all(&bytes)?;
    }
    file.set_len(len)?;
    file.sync_all()?;
    drop(file);
    std::fs::remove_file(&journal)?;
    log::info!("put back the fragmented recording {}", path.display());
    Ok(true)
}

/// Turns the fragmented recording at `path` into an ordinary MP4 in place. `progress` gets
/// how far into the file the reading is. On failure the file is as it was.
pub fn finish_in_place(path: &Path, progress: &mut dyn FnMut(u64)) -> Result<()> {
    recover(path)?;
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_len = file.metadata()?.len();
    let plan = plan(&mut file, progress)?;
    let moov = build_moov(&plan)?;

    // The file type: an ordinary MP4 now, in the same number of bytes.
    let mut ftyp_old = vec![0u8; plan.ftyp.size as usize];
    file.seek(SeekFrom::Start(plan.ftyp.pos))?;
    file.read_exact(&mut ftyp_old)?;
    let brands_len = plan.ftyp.size as usize - 16;
    let mut ftyp_new = ftyp_old.clone();
    ftyp_new[8..12].copy_from_slice(b"isom");
    ftyp_new[12..16].copy_from_slice(&0x200u32.to_be_bytes());
    let brands: [&[u8; 4]; 4] = [b"isom", b"iso2", b"avc1", b"mp41"];
    for i in 0..brands_len / 4 {
        ftyp_new[16 + i * 4..20 + i * 4].copy_from_slice(brands[i.min(3)]);
    }

    // One box over the old index and every fragment, up to the new index.
    ensure!(plan.moov.size >= 16, "the old index is too small to rewrap");
    let mut wrap_old = [0u8; 16];
    file.seek(SeekFrom::Start(plan.moov.pos))?;
    file.read_exact(&mut wrap_old)?;
    let wrap_new = wrap_header(plan.keep - plan.moov.pos);

    // The journal: how to put every changed byte back.
    let mut journal = String::new();
    journal.push_str(&format!("{}\n", plan.keep));
    journal.push_str(&format!("{} {}\n", plan.ftyp.pos, hex(&ftyp_old)));
    journal.push_str(&format!("{} {}\n", plan.moov.pos, hex(&wrap_old)));
    let journal_file = journal_path(path);
    {
        let mut j = File::create(&journal_file)?;
        j.write_all(journal.as_bytes())?;
        j.sync_all()?;
    }

    let result = (|| -> Result<()> {
        // The new index after the last complete fragment; an incomplete one is dropped.
        file.set_len(plan.keep)?;
        file.seek(SeekFrom::Start(plan.keep))?;
        file.write_all(&moov)?;
        file.sync_data()?;
        // The old index and the fragments become the inside of one data box.
        file.seek(SeekFrom::Start(plan.moov.pos))?;
        file.write_all(&wrap_new)?;
        file.seek(SeekFrom::Start(plan.ftyp.pos))?;
        file.write_all(&ftyp_new)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    let checked = result.and_then(|()| check(path, &plan));
    if let Err(err) = checked {
        recover(path).context("cannot put the recording back")?;
        return Err(err);
    }
    std::fs::remove_file(&journal_file)?;
    log::info!(
        "finished {} in place: {} fragments, {} bytes of {} kept",
        path.display(),
        plan.moofs.len(),
        plan.keep,
        file_len
    );
    Ok(())
}

/// The 16-byte header of an `mdat` box `size` bytes long (a 64-bit size).
fn wrap_header(size: u64) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[0..4].copy_from_slice(&1u32.to_be_bytes());
    h[4..8].copy_from_slice(b"mdat");
    h[8..16].copy_from_slice(&size.to_be_bytes());
    h
}

/// Top-level boxes of the file, up to `limit` of them; the file length too.
fn top_level(file: &mut File, limit: usize) -> Result<(Vec<Header>, u64)> {
    let len = file.metadata()?.len();
    let mut boxes = Vec::new();
    let mut pos = 0;
    while let Some(h) = header_at(file, pos, len)? {
        boxes.push(h);
        if h.end() > len || boxes.len() >= limit {
            break;
        }
        pos = h.end();
    }
    Ok((boxes, len))
}

/// True when the file is an ordinary MP4 with many boxes before its index at the end, as an
/// earlier version of [`finish_in_place`] left them.
pub fn needs_compact(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    match top_level(&mut file, 64) {
        Ok((boxes, _)) => {
            boxes.len() >= 64
                && &boxes[0].kind == b"ftyp"
                && boxes[1..]
                    .iter()
                    .all(|b| matches!(&b.kind, b"free" | b"mdat"))
        }
        Err(_) => false,
    }
}

/// Wraps everything between the file type and the index at the end in one `mdat` box, so
/// players reach the index in a few steps. Only for files [`needs_compact`] picks out; the
/// sample data does not move.
pub fn compact(path: &Path) -> Result<()> {
    recover(path)?;
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let (boxes, len) = top_level(&mut file, usize::MAX)?;
    ensure!(boxes.len() >= 4 && &boxes[0].kind == b"ftyp", "not an MP4");
    let moov = *boxes.last().unwrap();
    ensure!(
        &moov.kind == b"moov" && moov.end() == len,
        "the index is not at the end"
    );
    ensure!(
        boxes[1..boxes.len() - 1]
            .iter()
            .all(|b| matches!(&b.kind, b"free" | b"mdat" | b"skip" | b"mfra")),
        "something other than data before the index"
    );
    let first = boxes[1];
    ensure!(first.size >= 16, "the first box is too small to rewrap");
    let mut old = [0u8; 16];
    file.seek(SeekFrom::Start(first.pos))?;
    file.read_exact(&mut old)?;
    let journal_file = journal_path(path);
    {
        let mut j = File::create(&journal_file)?;
        j.write_all(format!("{len}\n{} {}\n", first.pos, hex(&old)).as_bytes())?;
        j.sync_all()?;
    }
    file.seek(SeekFrom::Start(first.pos))?;
    file.write_all(&wrap_header(moov.pos - first.pos))?;
    file.sync_all()?;
    drop(file);
    let opens = ffmpeg_next::format::input(&path)
        .map(|i| i.streams().count() > 0 && i.duration() > 0)
        .unwrap_or(false);
    if !opens {
        recover(path).context("cannot put the recording back")?;
        bail!("the rewrapped file does not open");
    }
    std::fs::remove_file(&journal_file)?;
    log::info!(
        "rewrapped {} ({} boxes before the index)",
        path.display(),
        boxes.len() - 2
    );
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Opens the converted file the way players do and compares what they see with the plan:
/// sample counts per track, and the last video packet readable at the offset it should be.
fn check(path: &Path, plan: &Plan) -> Result<()> {
    let mut input =
        ffmpeg_next::format::input(&path).context("the converted file does not open")?;
    let counts: Vec<i64> = input.streams().map(|s| s.frames()).collect();
    ensure!(
        counts.len() == plan.tracks.len(),
        "the converted file has {} tracks, not {}",
        counts.len(),
        plan.tracks.len()
    );
    for (track, count) in plan.tracks.iter().zip(&counts) {
        ensure!(
            *count == track.samples.len() as i64,
            "track {} has {count} samples after converting, {} before",
            track.id,
            track.samples.len()
        );
    }
    // The last packet of the file, read from where the index says it is.
    let (video, last) = plan
        .tracks
        .iter()
        .enumerate()
        .max_by_key(|(_, t)| t.chunks.last().map_or(0, |c| c.0))
        .map(|(i, t)| (i, t.samples.last().map_or(0, |s| s.size)))
        .context("no samples")?;
    let stream = input.stream(video).context("no stream")?;
    let end = stream.duration().max(0);
    unsafe {
        ffmpeg_next::ffi::av_seek_frame(
            input.as_mut_ptr(),
            video as i32,
            end,
            ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
        );
    }
    let mut seen = 0usize;
    let mut found = false;
    for (s, packet) in input.packets() {
        if s.index() == video && packet.size() == last as usize {
            found = true;
        }
        seen += 1;
        if seen > 20_000 {
            break;
        }
    }
    ensure!(
        found,
        "the last packet cannot be read from the converted file"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testutil::{TempDir, ffmpeg_exe};

    /// A recording-like fragmented MP4: video with a keyframe a second, three audio tracks.
    fn make_fragmented(path: &Path, secs: u32) {
        let status = std::process::Command::new(ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size=320x180:rate=30:duration={secs}"))
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=440:duration={secs}:sample_rate=48000"),
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=220:duration={secs}:sample_rate=48000"),
            ])
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("anullsrc=r=48000:cl=stereo:d={secs}"),
            ])
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:a"])
            .args(["-c:v", "libopenh264", "-b:v", "1M", "-g", "30", "-bf", "0"])
            .args(["-c:a", "aac", "-b:a", "96k"])
            .args(["-metadata", "umbra_recorded=2026-10-02T20:00:00Z"])
            .args([
                "-movflags",
                "+frag_keyframe+empty_moov+default_base_moof+use_metadata_tags",
            ])
            .arg(path)
            .status()
            .expect("cannot run ffmpeg.exe");
        assert!(status.success());
    }

    /// Every packet as (stream, pts, dts, size, keyframe).
    fn packets(path: &Path) -> Vec<(usize, i64, i64, usize, bool)> {
        let mut input = ffmpeg_next::format::input(&path).unwrap();
        input
            .packets()
            .map(|(s, p)| {
                (
                    s.index(),
                    p.pts().unwrap_or(-1),
                    p.dts().unwrap_or(-1),
                    p.size(),
                    p.is_key(),
                )
            })
            .collect()
    }

    fn sorted(mut v: Vec<(usize, i64, i64, usize, bool)>) -> Vec<(usize, i64, i64, usize, bool)> {
        v.sort();
        v
    }

    #[test]
    fn a_fragmented_file_becomes_the_same_ordinary_mp4() {
        let dir = TempDir::new("defrag");
        let path = dir.0.join("rec.mp4");
        make_fragmented(&path, 6);
        assert!(crate::recorder::is_fragmented(&path));
        let remuxed = dir.0.join("remuxed.mp4");
        crate::recorder::remux_to_mp4(&path, &remuxed, &[], &mut |_| {}).unwrap();
        let before = sorted(packets(&path));
        let size = std::fs::metadata(&path).unwrap().len();

        finish_in_place(&path, &mut |_| {}).unwrap();
        assert!(!crate::recorder::is_fragmented(&path));
        assert!(!journal_path(&path).exists());
        assert_eq!(sorted(packets(&path)), before);
        assert_eq!(sorted(packets(&path)), sorted(packets(&remuxed)));
        // Three boxes, as a player expects: file type, data, index.
        let mut file = File::open(&path).unwrap();
        let (boxes, _) = top_level(&mut file, 100).unwrap();
        let kinds: Vec<&[u8; 4]> = boxes.iter().map(|b| &b.kind).collect();
        assert_eq!(kinds, [b"ftyp", b"mdat", b"moov"]);
        drop(file);
        assert!(!needs_compact(&path));
        // Only an index was added.
        let after = std::fs::metadata(&path).unwrap().len();
        assert!(
            after > size && after < size + 64 * 1024,
            "{size} -> {after}"
        );
        let meta = crate::library::probe(&path).unwrap();
        assert!(
            (meta.duration_secs - 6.0).abs() < 0.1,
            "{} s",
            meta.duration_secs
        );
        assert_eq!(meta.audio_tracks.len(), 3);
        assert!(
            meta.created_at.starts_with("2026-10-02T20:00:00"),
            "{}",
            meta.created_at
        );
    }

    /// A recording whose camera opens after the screen started: written the way the recorder
    /// writes, then finished, the camera's first frame stays where it was filmed.
    #[test]
    fn a_stream_that_starts_late_keeps_its_start() {
        let dir = TempDir::new("defrag-late");
        let source = dir.0.join("source.mp4");
        let status = std::process::Command::new(ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=320x180:rate=30:duration=3")
            .args(["-itsoffset", "0.6", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=160x90:rate=30:duration=2")
            .args([
                "-map",
                "0",
                "-map",
                "1",
                "-c:v",
                "libopenh264",
                "-g",
                "30",
                "-bf",
                "0",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());

        let path = dir.0.join("rec.mp4");
        let mut input = ffmpeg_next::format::input(&source).unwrap();
        let streams: Vec<crate::encode::StreamInfo> = input
            .streams()
            .map(|s| crate::encode::StreamInfo {
                params: s.parameters(),
                time_base: s.time_base(),
                title: None,
            })
            .collect();
        let mut writer = crate::mux::Mp4Writer::create_fragmented(&path, &streams, &[]).unwrap();
        for (stream, packet) in input.packets() {
            writer.write(stream.index(), &packet, 0).unwrap();
        }
        writer.finish().unwrap();
        finish_in_place(&path, &mut |_| {}).unwrap();

        let input = ffmpeg_next::format::input(&path).unwrap();
        let late = input.stream(1).unwrap();
        let tb = late.time_base();
        let mut input = ffmpeg_next::format::input(&path).unwrap();
        let first = input
            .packets()
            .find(|(s, _)| s.index() == 1)
            .and_then(|(_, p)| p.pts())
            .unwrap();
        let secs = first as f64 * tb.numerator() as f64 / tb.denominator() as f64;
        assert!(
            (secs - 0.6).abs() < 0.02,
            "the late stream starts at {secs} s"
        );
    }

    #[test]
    fn a_recording_cut_short_keeps_its_complete_fragments() {
        let dir = TempDir::new("defrag-cut");
        let path = dir.0.join("rec.mp4");
        make_fragmented(&path, 6);
        let len = std::fs::metadata(&path).unwrap().len();
        // A crash part-way through the last fragment's data.
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len - 20_000)
            .unwrap();
        finish_in_place(&path, &mut |_| {}).unwrap();
        let meta = crate::library::probe(&path).unwrap();
        assert!(
            meta.duration_secs > 3.5 && meta.duration_secs < 6.0,
            "{} s",
            meta.duration_secs
        );
        let all = packets(&path);
        assert!(all.iter().any(|p| p.0 == 0 && p.4));
    }

    #[test]
    fn files_left_with_many_boxes_are_rewrapped() {
        let dir = TempDir::new("defrag-compact");
        let path = dir.0.join("rec.mp4");
        make_fragmented(&path, 70);
        let before = sorted(packets(&path));
        // What the earlier version did: a new index, every moof and the old index renamed.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let plan = plan(&mut file, &mut |_| {}).unwrap();
        let moov = build_moov(&plan).unwrap();
        file.set_len(plan.keep).unwrap();
        file.seek(SeekFrom::Start(plan.keep)).unwrap();
        file.write_all(&moov).unwrap();
        for &at in std::iter::once(&plan.moov.pos)
            .chain(&plan.moofs)
            .chain(&plan.mfras)
        {
            file.seek(SeekFrom::Start(at + 4)).unwrap();
            file.write_all(b"free").unwrap();
        }
        drop(file);
        assert!(needs_compact(&path));

        compact(&path).unwrap();
        assert!(!needs_compact(&path));
        let mut file = File::open(&path).unwrap();
        let (boxes, _) = top_level(&mut file, 100).unwrap();
        assert_eq!(boxes.len(), 3);
        drop(file);
        assert_eq!(sorted(packets(&path)), before);
    }

    #[test]
    fn an_interrupted_conversion_is_put_back() {
        let dir = TempDir::new("defrag-journal");
        let path = dir.0.join("rec.mp4");
        make_fragmented(&path, 3);
        let original = std::fs::read(&path).unwrap();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let plan = plan(&mut file, &mut |_| {}).unwrap();
        // What a conversion does before a crash: the journal, the new index, some renames.
        let mut journal = format!("{}\n{} {}\n", plan.keep, plan.moov.pos + 4, hex(b"moov"));
        journal.push_str(&format!("{} {}\n", plan.moofs[0] + 4, hex(b"moof")));
        std::fs::write(journal_path(&path), journal).unwrap();
        let moov = build_moov(&plan).unwrap();
        file.seek(SeekFrom::Start(plan.keep)).unwrap();
        file.write_all(&moov).unwrap();
        file.seek(SeekFrom::Start(plan.moov.pos + 4)).unwrap();
        file.write_all(b"free").unwrap();
        file.seek(SeekFrom::Start(plan.moofs[0] + 4)).unwrap();
        file.write_all(b"free").unwrap();
        drop(file);

        assert!(recover(&path).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!recover(&path).unwrap());
    }
}
