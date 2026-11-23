//! The chime played when a clip is saved. Synthesised, so no audio file ships with the app.

use std::f32::consts::TAU;
use std::sync::Arc;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::core::PCWSTR;

const SAMPLE_RATE: u32 = 48_000;

pub struct SaveSound {
    /// Kept alive because asynchronous playback reads straight from this memory.
    wav: Arc<Vec<u8>>,
}

impl SaveSound {
    /// `volume` is 0 to 1.
    pub fn new(volume: f32) -> Self {
        Self {
            wav: Arc::new(chime(volume.clamp(0.0, 1.0))),
        }
    }

    /// Starts playing and returns immediately. A new call cuts off the previous one.
    pub fn play(&self) {
        unsafe {
            let _ = PlaySoundW(
                PCWSTR(self.wav.as_ptr() as *const u16),
                None,
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            );
        }
    }
}

/// Two short rising notes.
fn chime(volume: f32) -> Vec<u8> {
    let notes = [(660.0_f32, 0.085_f32), (990.0, 0.14)];
    let mut samples: Vec<i16> = Vec::new();
    for (frequency, length) in notes {
        let count = (SAMPLE_RATE as f32 * length) as usize;
        for i in 0..count {
            let t = i as f32 / SAMPLE_RATE as f32;
            let attack = (t / 0.004).min(1.0);
            let decay = (-t * 16.0).exp();
            let tone = (TAU * frequency * t).sin() + 0.25 * (TAU * frequency * 2.0 * t).sin();
            let value = tone * attack * decay * volume * 0.3;
            samples.push((value * i16::MAX as f32) as i16);
        }
    }

    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_consistent() {
        let wav = chime(1.0);
        assert_eq!(&wav[0..4], b"RIFF");
        let riff_len = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff_len, wav.len() - 8);
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(data_len, wav.len() - 44);
    }

    #[test]
    fn silent_at_zero_volume() {
        let wav = chime(0.0);
        assert!(wav[44..].iter().all(|&b| b == 0));
    }
}
