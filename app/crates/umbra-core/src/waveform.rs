//! Loudness outlines of a clip's audio tracks, for drawing waveforms under the editor's
//! timeline. One pass decodes every track; each value is the loudest sample in a short slice.

use std::path::Path;

use anyhow::{Context, Result};
use ffmpeg_next::{codec, format, frame, media};

use crate::library::init_ffmpeg;

/// At most this many slices per track, however long the clip.
const MAX_SLICES: usize = 8000;
/// The shortest slice, in seconds.
const MIN_SLICE_SECS: f64 = 0.01;

#[derive(Clone, Debug)]
pub struct Waveforms {
    /// Length of one slice in seconds. Slice `i` covers `i * slice_secs` onward.
    pub slice_secs: f64,
    /// Per audio track (in file order), the peak of each slice on a decibel scale: 0 is
    /// -60 dBFS or quieter, 255 is full scale. Game audio is often far below full scale, and
    /// a linear scale would draw it as a flat line.
    pub tracks: Vec<Vec<u8>>,
}

pub fn waveforms(path: &Path) -> Result<Waveforms> {
    init_ffmpeg()?;
    let mut input =
        format::input(path).with_context(|| format!("cannot open {}", path.display()))?;
    let duration = if input.duration() > 0 {
        input.duration() as f64 / 1_000_000.0
    } else {
        0.0
    };
    let slice_secs = (duration / MAX_SLICES as f64).max(MIN_SLICE_SECS);

    struct Track {
        stream: usize,
        decoder: ffmpeg_next::decoder::Audio,
        /// Samples per slice at this track's rate.
        per_slice: f64,
        /// Samples decoded so far.
        position: u64,
        peaks: Vec<u8>,
    }
    let mut tracks = Vec::new();
    for stream in input
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Audio)
    {
        let decoder = codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .audio()
            .context("cannot open the audio decoder")?;
        let per_slice = (decoder.rate().max(1) as f64 * slice_secs).max(1.0);
        tracks.push(Track {
            stream: stream.index(),
            decoder,
            per_slice,
            position: 0,
            peaks: Vec::new(),
        });
    }

    let mut decoded = frame::Audio::empty();
    let take = |track: &mut Track, decoded: &mut frame::Audio| {
        while track.decoder.receive_frame(decoded).is_ok() {
            let count = decoded.samples();
            // AAC decodes to planar float; any other format counts as silence.
            let planar_float = decoded.format()
                == ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar);
            let planes = if planar_float {
                decoded.planes().min(2)
            } else {
                0
            };
            for i in 0..count {
                let mut peak = 0f32;
                for plane in 0..planes {
                    peak = peak.max(decoded.plane::<f32>(plane)[i].abs());
                }
                let slice = (track.position as f64 / track.per_slice) as usize;
                if slice >= track.peaks.len() {
                    track.peaks.resize(slice + 1, 0);
                }
                let value = level(peak);
                if value > track.peaks[slice] {
                    track.peaks[slice] = value;
                }
                track.position += 1;
            }
        }
    };

    for (stream, packet) in input.packets() {
        if let Some(track) = tracks.iter_mut().find(|t| t.stream == stream.index()) {
            if track.decoder.send_packet(&packet).is_ok() {
                take(track, &mut decoded);
            }
        }
    }
    for track in &mut tracks {
        let _ = track.decoder.send_eof();
        take(track, &mut decoded);
    }

    Ok(Waveforms {
        slice_secs,
        tracks: tracks.into_iter().map(|t| t.peaks).collect(),
    })
}

/// A sample peak on the 0 to 255 decibel scale of [`Waveforms::tracks`].
fn level(peak: f32) -> u8 {
    if peak <= 0.001 {
        return 0;
    }
    let db = 20.0 * peak.min(1.0).log10();
    ((1.0 + db / 60.0) * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testutil::{TempDir, make_clip};

    #[test]
    fn levels_follow_decibels() {
        assert_eq!(level(0.0), 0);
        assert_eq!(level(1.0), 255);
        // -20 dBFS is two thirds of the way up.
        assert_eq!(level(0.1), 170);
    }

    #[test]
    fn every_track_gets_an_outline_of_the_whole_clip() {
        let dir = TempDir::new("waveform");
        let clip = dir.0.join("clip.mp4");
        make_clip(&clip, 2);
        let w = waveforms(&clip).unwrap();
        assert_eq!(w.tracks.len(), 2);
        for peaks in &w.tracks {
            let secs = peaks.len() as f64 * w.slice_secs;
            assert!((secs - 2.0).abs() < 0.1, "{secs}");
            // The test tones are steady and loud.
            let loud = peaks.iter().filter(|&&p| p > 64).count();
            assert!(loud > peaks.len() * 9 / 10, "{loud} of {}", peaks.len());
        }
    }
}
