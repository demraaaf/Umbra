//! Video settings as a short code that can be pasted to a friend, such as
//! `UMBRA-042G60-3R0000-…`. The code packs every video setting into a few bytes, adds a check
//! byte so a mistyped code is refused instead of read wrongly, and writes them in Crockford
//! base 32, which avoids letters that look alike and ignores case.

use anyhow::{Result, bail};

use crate::settings::{CodecChoice, Multipass, RateMode, Scaling, Tune, VideoPrefs};

const PREFIX: &str = "UMBRA";
const VERSION: u8 = 1;
/// Letters in a group between dashes.
const GROUP: usize = 6;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The code for `video`.
pub fn encode(video: &VideoPrefs) -> String {
    let mut bytes = Vec::with_capacity(24);
    bytes.push(VERSION);
    bytes.push(
        codec_bits(video.codec)
            | rate_bits(video.rate_mode) << 2
            | tune_bits(video.tune) << 4
            | multipass_bits(video.multipass) << 6,
    );
    let size = video.width.zip(video.height);
    bytes.push(
        video.capture_cursor as u8
            | (video.spatial_aq as u8) << 1
            | (video.temporal_aq as u8) << 2
            | (size.is_some() as u8) << 3
            | ((video.scaling == Scaling::Fit) as u8) << 4,
    );
    bytes.extend_from_slice(&(video.fps as u16).to_be_bytes());
    let (width, height) = size.unwrap_or((0, 0));
    bytes.extend_from_slice(&(width as u16).to_be_bytes());
    bytes.extend_from_slice(&(height as u16).to_be_bytes());
    bytes.extend_from_slice(&video.bitrate_kbps.to_be_bytes()[1..]);
    bytes.extend_from_slice(&video.max_bitrate_kbps.to_be_bytes()[1..]);
    bytes.extend_from_slice(&[
        video.qp,
        video.cq,
        video.preset,
        video.keyframe_secs.min(255) as u8,
        video.lookahead,
        video.aq_strength,
    ]);
    bytes.push(check(&bytes));

    let letters = to_base32(&bytes);
    let mut code = String::from(PREFIX);
    for group in letters.as_bytes().chunks(GROUP) {
        code.push('-');
        code.push_str(std::str::from_utf8(group).unwrap());
    }
    code
}

/// Reads a code back. Spaces, dashes, case and the prefix are optional; `O` reads as zero and
/// `I` or `L` as one. Values are left for [`crate::settings::Settings::normalized`] to clamp.
pub fn decode(code: &str) -> Result<VideoPrefs> {
    let mut text: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if let Some(rest) = text.strip_prefix(PREFIX) {
        text = rest.to_string();
    }
    if text.is_empty() {
        bail!("That isn't a settings code.");
    }
    let Some(bytes) = from_base32(&text) else {
        bail!("That isn't a settings code: it has letters a code never uses.");
    };
    if bytes.first() != Some(&VERSION) {
        bail!("This code is from a newer version of Umbra, or it isn't a settings code.");
    }
    if bytes.len() != 22 {
        bail!("This code is incomplete. Check that all of it was copied.");
    }
    let (body, sum) = bytes.split_at(bytes.len() - 1);
    if check(body) != sum[0] {
        bail!("This code has a typo somewhere. Check it against the original.");
    }

    let modes = body[1];
    let flags = body[2];
    let u16_at = |i: usize| u16::from_be_bytes([body[i], body[i + 1]]) as u32;
    let u24_at = |i: usize| u32::from_be_bytes([0, body[i], body[i + 1], body[i + 2]]);
    let has_size = flags & 1 << 3 != 0;
    Ok(VideoPrefs {
        // Not part of a code: how an HDR screen is recorded depends on the screen.
        hdr: Default::default(),
        // Nor whether still frames are left out: that is about the PC, not the picture.
        skip_still: false,
        codec: match modes & 3 {
            0 => CodecChoice::H264,
            2 => CodecChoice::Av1,
            _ => CodecChoice::Hevc,
        },
        rate_mode: match modes >> 2 & 3 {
            0 => RateMode::Cbr,
            1 => RateMode::Vbr,
            2 => RateMode::Cq,
            _ => RateMode::Cqp,
        },
        tune: match modes >> 4 & 3 {
            1 => Tune::Ll,
            2 => Tune::Ull,
            _ => Tune::Hq,
        },
        multipass: match modes >> 6 & 3 {
            1 => Multipass::Quarter,
            2 => Multipass::Full,
            _ => Multipass::Off,
        },
        capture_cursor: flags & 1 != 0,
        spatial_aq: flags & 1 << 1 != 0,
        temporal_aq: flags & 1 << 2 != 0,
        scaling: if flags & 1 << 4 != 0 {
            Scaling::Fit
        } else {
            Scaling::Stretch
        },
        fps: u16_at(3),
        width: has_size.then(|| u16_at(5)),
        height: has_size.then(|| u16_at(7)),
        bitrate_kbps: u24_at(9),
        max_bitrate_kbps: u24_at(12),
        qp: body[15],
        cq: body[16],
        preset: body[17],
        keyframe_secs: body[18] as u32,
        lookahead: body[19],
        aq_strength: body[20],
    })
}

fn codec_bits(codec: CodecChoice) -> u8 {
    match codec {
        CodecChoice::H264 => 0,
        CodecChoice::Hevc => 1,
        CodecChoice::Av1 => 2,
    }
}

fn rate_bits(mode: RateMode) -> u8 {
    match mode {
        RateMode::Cbr => 0,
        RateMode::Vbr => 1,
        RateMode::Cq => 2,
        RateMode::Cqp => 3,
    }
}

fn tune_bits(tune: Tune) -> u8 {
    match tune {
        Tune::Hq => 0,
        Tune::Ll => 1,
        Tune::Ull => 2,
    }
}

fn multipass_bits(multipass: Multipass) -> u8 {
    match multipass {
        Multipass::Off => 0,
        Multipass::Quarter => 1,
        Multipass::Full => 2,
    }
}

/// CRC-8 (polynomial 0x07). Catches every single wrong letter and every swap of two.
fn check(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn to_base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = buffer << 8 | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[(buffer >> bits & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[(buffer << (5 - bits) & 31) as usize] as char);
    }
    out
}

fn from_base32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.chars() {
        let c = match c {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        };
        let value = ALPHABET.iter().position(|&a| a as char == c)? as u32;
        buffer = (buffer << 5 | value) & 0xFFFF;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn custom() -> VideoPrefs {
        VideoPrefs {
            hdr: Default::default(),
            skip_still: false,
            codec: CodecChoice::Av1,
            width: Some(1920),
            height: Some(1080),
            fps: 144,
            rate_mode: RateMode::Cq,
            bitrate_kbps: 45_000,
            max_bitrate_kbps: 1_500_000,
            qp: 18,
            cq: 21,
            preset: 6,
            capture_cursor: false,
            keyframe_secs: 2,
            tune: Tune::Ll,
            multipass: Multipass::Quarter,
            lookahead: 20,
            spatial_aq: true,
            temporal_aq: true,
            aq_strength: 11,
            scaling: Scaling::Fit,
        }
    }

    #[test]
    fn every_setting_survives_the_trip() {
        for video in [Settings::default().video, custom()] {
            let code = encode(&video);
            assert!(code.starts_with("UMBRA-"), "{code}");
            assert_eq!(decode(&code).unwrap(), video, "{code}");
        }
    }

    #[test]
    fn codes_are_short_and_forgiving_to_type() {
        let code = encode(&custom());
        assert!(code.len() <= 50, "{code} is {} long", code.len());
        let sloppy = code.to_lowercase().replace('-', " ").replace("umbra", "");
        assert_eq!(decode(&sloppy).unwrap(), custom());
    }

    #[test]
    fn a_typo_is_caught() {
        let code = encode(&custom());
        // Change one letter of the payload.
        let mut chars: Vec<char> = code.chars().collect();
        let at = 10;
        chars[at] = if chars[at] == 'A' { 'B' } else { 'A' };
        let typo: String = chars.into_iter().collect();
        let err = decode(&typo).unwrap_err().to_string();
        assert!(err.contains("typo"), "{err}");
        assert!(decode(&code[..code.len() - 6]).is_err());
        assert!(decode("hello there").is_err());
        assert!(decode("").is_err());
    }
}
