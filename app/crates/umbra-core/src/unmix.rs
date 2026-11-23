//! Finding a separately recorded app inside the "Other apps" track.
//!
//! "Other apps" is everything except the game, so an app that also has its own track
//! (Discord, Spotify) is in both. Windows timestamps the two captures independently, so the
//! copies sit a few milliseconds apart, by a different amount in every recording. To change
//! the app's volume everywhere, the export subtracts its own track from "Other apps" at the
//! offset found here. The tracks are AAC, so the subtraction is limited by coding noise: the
//! app ends up about 25 to 30 dB quieter than it was, not gone to the last bit.

use std::path::Path;

use anyhow::{Context, Result};
use ffmpeg_next::{Rational, codec, format, frame, media};

use crate::align::SAMPLE_RATE;
use crate::library::{MixRole, init_ffmpeg, mix_roles, stream_title};

/// Seconds of audio read from the start of the file to find the offset.
const READ_SECS: usize = 90;
/// Length of the stretch the offset is measured on.
const WINDOW_SECS: usize = 3;
/// The largest offset searched, in samples (200 ms).
const MAX_LAG: i64 = 9600;
/// Decimation for the coarse search.
const COARSE: usize = 8;

/// An app track and where its sound sits inside "Other apps".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppInOthers {
    /// Audio track index (file order) of the app's own track.
    pub app: usize,
    /// Audio track index of "Other apps".
    pub others: usize,
    /// Samples to add to a position in the app's track to find the same sound in "Other
    /// apps": `others[t] ≈ app[t - lag]`.
    pub lag: i64,
}

/// For every app recorded next to "Other apps", the offset of its copy inside "Other apps".
/// Apps that were silent, or whose copy could not be found with confidence, are left out:
/// there is nothing to take out of "Other apps" for them, or no safe way to.
pub fn find_apps_in_others(path: &Path) -> Result<Vec<AppInOthers>> {
    init_ffmpeg()?;
    let mut input =
        format::input(path).with_context(|| format!("cannot open {}", path.display()))?;
    let audio: Vec<(usize, String)> = input
        .streams()
        .filter(|s| s.parameters().medium() == media::Type::Audio)
        .enumerate()
        .map(|(i, s)| {
            (
                s.index(),
                stream_title(&s.metadata()).unwrap_or_else(|| format!("Track {}", i + 1)),
            )
        })
        .collect();
    let titles: Vec<&str> = audio.iter().map(|(_, t)| t.as_str()).collect();
    let mix_tag = input
        .metadata()
        .get(&format!("{}mix", crate::mux::TAG_PREFIX))
        .map(str::to_string);
    let roles = mix_roles(&titles, mix_tag.as_deref());
    let Some(others) = titles
        .iter()
        .position(|t| t.eq_ignore_ascii_case("Other apps"))
    else {
        return Ok(Vec::new());
    };
    let apps: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter(|(_, r)| **r == MixRole::InOthers)
        .map(|(i, _)| i)
        .collect();
    if apps.is_empty() {
        return Ok(Vec::new());
    }

    let mut wanted = apps.clone();
    wanted.push(others);
    let signals = read_mono(&mut input, &audio, &wanted)?;
    let other = &signals[wanted.len() - 1];
    let mut found = Vec::new();
    for (k, &app) in apps.iter().enumerate() {
        if let Some(lag) = find_lag(&signals[k], other) {
            found.push(AppInOthers { app, others, lag });
        }
    }
    Ok(found)
}

/// The first `READ_SECS` of the given audio tracks as mono, each placed on the file's
/// timeline by its first packet's timestamp.
fn read_mono(
    input: &mut format::context::Input,
    audio: &[(usize, String)],
    wanted: &[usize],
) -> Result<Vec<Vec<f32>>> {
    struct Reader {
        stream: usize,
        decoder: ffmpeg_next::decoder::Audio,
        time_base: Rational,
        samples: Vec<f32>,
        start: Option<i64>,
    }
    let limit = READ_SECS * SAMPLE_RATE as usize;
    let mut readers = Vec::new();
    for &track in wanted {
        let stream_index = audio[track].0;
        let stream = input.stream(stream_index).context("missing audio stream")?;
        let decoder = codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .audio()?;
        readers.push(Reader {
            stream: stream_index,
            decoder,
            time_base: stream.time_base(),
            samples: Vec::new(),
            start: None,
        });
    }
    let mut decoded = frame::Audio::empty();
    for (stream, packet) in input.packets() {
        let Some(r) = readers.iter_mut().find(|r| r.stream == stream.index()) else {
            continue;
        };
        if r.samples.len() >= limit {
            if readers.iter().all(|r| r.samples.len() >= limit) {
                break;
            }
            continue;
        }
        if r.start.is_none() {
            let pts = packet.pts().unwrap_or(0);
            r.start = Some(
                pts * r.time_base.numerator() as i64 * SAMPLE_RATE as i64
                    / r.time_base.denominator().max(1) as i64,
            );
        }
        if r.decoder.send_packet(&packet).is_err() {
            continue;
        }
        while r.decoder.receive_frame(&mut decoded).is_ok() {
            if decoded.format()
                != ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar)
                || decoded.planes() < 2
            {
                continue;
            }
            let n = decoded.samples();
            let (left, right) = (decoded.plane::<f32>(0), decoded.plane::<f32>(1));
            r.samples.extend((0..n).map(|i| (left[i] + right[i]) * 0.5));
        }
    }
    // Put every track on the same timeline: pad the ones that started later.
    let first = readers.iter().filter_map(|r| r.start).min().unwrap_or(0);
    Ok(readers
        .into_iter()
        .map(|r| {
            let pad = (r.start.unwrap_or(first) - first).max(0) as usize;
            let mut out = vec![0.0; pad];
            out.extend(r.samples);
            out.truncate(limit);
            out
        })
        .collect())
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| v as f64 * v as f64).sum()
}

/// Correlation of `others[t]` with `app[t - lag]` over `t` in `range`.
fn correlation(app: &[f32], others: &[f32], range: std::ops::Range<usize>, lag: i64) -> f64 {
    let mut sum = 0.0f64;
    for t in range {
        let s = t as i64 - lag;
        if s >= 0 && (s as usize) < app.len() {
            sum += others[t] as f64 * app[s as usize] as f64;
        }
    }
    sum
}

/// The offset of `app` inside `others`, or `None` when the app is silent or not there.
fn find_lag(app: &[f32], others: &[f32]) -> Option<i64> {
    let window = WINDOW_SECS * SAMPLE_RATE as usize;
    let n = app.len().min(others.len());
    if n < window + 2 * MAX_LAG as usize {
        return None;
    }
    // The loudest stretch of the app, away from the edges so every lag has data.
    let margin = MAX_LAG as usize;
    let step = SAMPLE_RATE as usize / 2;
    let (start, loudness) = (margin..n - margin - window)
        .step_by(step)
        .map(|s| (s, energy(&app[s..s + window])))
        .fold((margin, 0.0), |best, x| if x.1 > best.1 { x } else { best });
    // Below about -70 dBFS on average there is nothing worth taking out.
    if loudness / (window as f64) < 1e-7 {
        return None;
    }

    // Coarse search on averaged samples, then exact around the best coarse lag.
    let decimate = |x: &[f32]| -> Vec<f32> {
        x.chunks(COARSE)
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect()
    };
    let (app_c, others_c) = (decimate(app), decimate(others));
    let range_c = start / COARSE..(start + window) / COARSE;
    let max_c = MAX_LAG / COARSE as i64;
    let coarse = (-max_c..=max_c)
        .map(|l| (l, correlation(&app_c, &others_c, range_c.clone(), l)))
        .fold((0, f64::MIN), |best, x| if x.1 > best.1 { x } else { best })
        .0
        * COARSE as i64;
    let range = start..start + window;
    let lag = (coarse - 2 * COARSE as i64..=coarse + 2 * COARSE as i64)
        .map(|l| (l, correlation(app, others, range.clone(), l)))
        .fold((0, f64::MIN), |best, x| if x.1 > best.1 { x } else { best })
        .0;

    // Taking the app out at this offset has to remove most of its energy. At a wrong offset
    // the subtraction adds energy instead.
    let before = energy(&others[range.clone()]);
    let after: f64 = range
        .clone()
        .map(|t| {
            let s = t as i64 - lag;
            let v = others[t] as f64 - app[s as usize] as f64;
            v * v
        })
        .sum();
    let removed = before - after;
    (removed > 0.5 * loudness).then_some(lag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((s >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.2
            })
            .collect()
    }

    #[test]
    fn finds_the_offset_of_an_app_inside_others() {
        let n = 8 * SAMPLE_RATE as usize;
        let app = noise(n, 1);
        let rest = noise(n, 2);
        for lag in [-91i64, 0, 85, 3000] {
            let others: Vec<f32> = (0..n)
                .map(|t| {
                    let s = t as i64 - lag;
                    let a = if s >= 0 && (s as usize) < n {
                        app[s as usize]
                    } else {
                        0.0
                    };
                    a + rest[t] * 0.5
                })
                .collect();
            assert_eq!(find_lag(&app, &others), Some(lag));
        }
    }

    #[test]
    fn an_app_that_is_not_in_others_is_not_found() {
        let n = 8 * SAMPLE_RATE as usize;
        assert_eq!(find_lag(&noise(n, 3), &noise(n, 4)), None);
        // Silent app: nothing to take out.
        assert_eq!(find_lag(&vec![0.0; n], &noise(n, 4)), None);
    }
}

#[cfg(test)]
mod real_clip {
    /// `UMBRA_CLIP=<path> cargo test -p umbra-core real_clip -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_offsets_of_a_real_clip() {
        let path = std::env::var("UMBRA_CLIP").expect("set UMBRA_CLIP");
        let started = std::time::Instant::now();
        let found = super::find_apps_in_others(std::path::Path::new(&path)).unwrap();
        println!("{found:?} in {:?}", started.elapsed());
    }
}
