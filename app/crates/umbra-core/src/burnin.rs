//! Burning a clip's layers (its camera, keyboard and mouse, logo) into the picture for good,
//! from the library: a copy that plays as Umbra shows it everywhere. Only the video is encoded
//! again, with the clip's own codec at about its own bitrate (the export's "Original"); the
//! audio is copied. What is not burned in stays a layer of its own.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::export::{CameraBurn, ExportJob, InputBurn, LogoBurn};
use crate::library::ClipMeta;
use crate::settings::{LogoPrefs, Settings};

/// The layers a clip has, or those to burn in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Layers {
    pub camera: bool,
    pub input: bool,
    pub logo: bool,
}

impl Layers {
    pub fn any(self) -> bool {
        self.camera || self.input || self.logo
    }

    /// Those in both.
    pub fn and(self, other: Self) -> Self {
        Self {
            camera: self.camera && other.camera,
            input: self.input && other.input,
            logo: self.logo && other.logo,
        }
    }
}

fn detail<'a>(meta: &'a ClipMeta, key: &str) -> Option<&'a str> {
    meta.details
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// The clip's logo, when it has one whose picture is still there.
pub fn logo_of(meta: &ClipMeta) -> Option<LogoBurn> {
    let logo: LogoPrefs = serde_json::from_str(detail(meta, "logo")?).ok()?;
    Some(LogoBurn {
        path: PathBuf::from(logo.path?),
        placement: logo.placement,
    })
    .filter(|l| l.path.exists())
}

/// The layers a clip has, from the file itself (so clips from older versions and imported
/// ones count too): a camera stream, an input file beside it, a logo tag.
pub fn layers(meta: &ClipMeta) -> Layers {
    Layers {
        camera: meta.camera.is_some(),
        input: crate::inputfile::existing_sidecar(&meta.path).is_some(),
        logo: logo_of(meta).is_some(),
    }
}

/// Where a clip with layers burned in goes: `<name> (burned).mp4` beside it, numbered when
/// that is taken.
pub fn output_for(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or("clip".into(), |s| s.to_string_lossy());
    std::iter::once(format!("{stem} (burned).mp4"))
        .chain((2..).map(|n| format!("{stem} (burned) {n}.mp4")))
        .map(|name| path.with_file_name(name))
        .find(|p| !p.exists())
        .unwrap()
}

/// The export that draws `which` into the clip where the clip places them (where Settings
/// place them new, when it does not say), and keeps everything else: every audio track as it
/// is, the camera stream and the tags of layers not drawn.
pub fn job(meta: &ClipMeta, which: Layers, settings: &Settings, output: &Path) -> ExportJob {
    let has = which.and(layers(meta));
    let mut job = ExportJob::new(&meta.path, output, 0.0, meta.duration_secs + 1.0);
    job.audio_tracks = (0..meta.audio_tracks.len()).collect();
    job.threads = settings.export.threads;
    job.keep_camera = !has.camera;
    job.keep_logo = !has.logo;
    if has.camera {
        job.camera = Some(CameraBurn {
            placement: detail(meta, "camera")
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_else(|| settings.camera.placement.clone()),
            hidden: Vec::new(),
        });
    }
    if has.logo {
        job.logo = logo_of(meta);
    }
    if has.input {
        job.input_overlay = Some(InputBurn {
            overlay: detail(meta, "input")
                .and_then(|json| serde_json::from_str(json).ok())
                .unwrap_or_else(|| settings.input.overlay.clone()),
        });
    }
    job
}

/// Burns `which` into the clip, writing `output`. The input file goes with it while the input
/// stays a layer. `progress` gets the fraction done and returns false to stop.
pub fn burn(
    meta: &ClipMeta,
    which: Layers,
    settings: &Settings,
    output: &Path,
    progress: &mut dyn FnMut(f32) -> bool,
) -> Result<()> {
    let has = which.and(layers(meta));
    if !has.any() {
        bail!("it has none of those layers");
    }
    crate::export::export(&job(meta, has, settings, output), progress)?;
    if !has.input
        && let Some(input) = crate::inputfile::existing_sidecar(&meta.path)
        && let Err(err) = std::fs::copy(&input, crate::inputfile::sidecar(output))
    {
        let _ = std::fs::remove_file(output);
        return Err(err).context("cannot copy the keyboard and mouse input");
    }
    Ok(())
}

/// Checks that `output` holds the whole clip with the layers that should stay, before the
/// original may go.
pub fn check(meta: &ClipMeta, which: Layers, output: &Path) -> Result<()> {
    let new = crate::library::probe(output).context("cannot read the new file")?;
    let tolerance = 1.0f64.max(meta.duration_secs * 0.01);
    if (new.duration_secs - meta.duration_secs).abs() > tolerance {
        bail!(
            "the new file is {:.1} s long, the original {:.1} s",
            new.duration_secs,
            meta.duration_secs
        );
    }
    if new.audio_tracks.len() != meta.audio_tracks.len()
        || (new.width, new.height) != (meta.width, meta.height)
    {
        bail!("the new file's tracks or picture size differ");
    }
    if new.camera.is_some() != (meta.camera.is_some() && !which.camera) {
        bail!("the new file's camera is not as it should be");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputfile::{Block, Event, Key, Pressed};
    use crate::library::probe;
    use crate::library::testutil::{TempDir, ffmpeg_exe};
    use ffmpeg_next::format;

    /// A 3 s clip of 320×180 with a camera stream, two audio tracks, Umbra's camera and logo
    /// tags, and an input file; and its logo picture.
    fn layered_clip(dir: &Path) -> PathBuf {
        let clip = dir.join("clip.mp4");
        let logo = dir.join("logo.png");
        let ok = std::process::Command::new(ffmpeg_exe())
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=red:size=64x64",
            ])
            .args(["-frames:v", "1"])
            .arg(&logo)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let logo_tag = serde_json::to_string(&LogoPrefs {
            path: Some(logo.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .unwrap();
        let ok = std::process::Command::new(ffmpeg_exe())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("color=black:size=320x180:rate=30:duration=3")
            .args([
                "-f",
                "lavfi",
                "-i",
                "color=white:size=160x90:rate=30:duration=3",
            ])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3"])
            .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=3"])
            .args(["-map", "0", "-map", "1", "-map", "2", "-map", "3"])
            .args(["-c:v", "libopenh264", "-bf", "0", "-g", "30"])
            .args(["-c:a", "aac", "-b:a", "96k"])
            .args(["-metadata:s:v:1", "handler_name=Camera"])
            .args(["-movflags", "+use_metadata_tags"])
            .arg("-metadata")
            .arg(format!("umbra_logo={logo_tag}"))
            .args(["-metadata", "umbra_game=Test"])
            .arg(&clip)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let w = Key {
            vk: 0x57,
            scan: 0x11,
            ext: false,
        };
        crate::inputfile::write(
            &crate::inputfile::sidecar(&clip),
            &[Block {
                start: 0.0,
                pressed: Pressed::default(),
                events: vec![(1.0, Event::KeyDown(w)), (2.0, Event::KeyUp(w))],
            }],
        )
        .unwrap();
        clip
    }

    /// The sizes of each audio stream's packets.
    fn audio_packets(path: &Path) -> Vec<Vec<usize>> {
        let mut input = format::input(path).unwrap();
        let audio: Vec<usize> = input
            .streams()
            .filter(|s| s.parameters().medium() == ffmpeg_next::media::Type::Audio)
            .map(|s| s.index())
            .collect();
        let mut sizes = vec![Vec::new(); audio.len()];
        for (stream, packet) in input.packets() {
            if let Some(i) = audio.iter().position(|&a| a == stream.index()) {
                sizes[i].push(packet.size());
            }
        }
        sizes
    }

    /// Every audio track copied: its packets a run of the original's, unchanged (the export
    /// trims a packet or two outside the video at the ends).
    fn audio_copied(original: &[Vec<usize>], output: &Path) {
        let out = audio_packets(output);
        assert_eq!(out.len(), original.len(), "every audio track");
        for (o, n) in original.iter().zip(&out) {
            assert!(n.len() + 3 >= o.len(), "{} of {} packets", n.len(), o.len());
            assert!(
                o.windows(n.len()).any(|w| w == n.as_slice()),
                "copied as they are"
            );
        }
    }

    #[test]
    fn layers_come_from_the_file() {
        crate::library::init_ffmpeg().unwrap();
        let dir = TempDir::new("burnin-layers");
        let clip = layered_clip(&dir.0);
        let all = Layers {
            camera: true,
            input: true,
            logo: true,
        };
        assert_eq!(layers(&probe(&clip).unwrap()), all);

        // The logo's picture gone: nothing to draw, so no logo.
        std::fs::remove_file(dir.0.join("logo.png")).unwrap();
        let meta = probe(&clip).unwrap();
        assert_eq!(layers(&meta), Layers { logo: false, ..all });

        let plain = dir.0.join("plain.mp4");
        let ok = std::process::Command::new(ffmpeg_exe())
            .args(["-y", "-v", "error", "-i"])
            .arg(&clip)
            .args(["-map", "0:0", "-c", "copy"])
            .arg(&plain)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        assert_eq!(layers(&probe(&plain).unwrap()), Layers::default());
    }

    #[test]
    fn burning_in_keeps_the_audio_and_the_layers_not_burned() {
        crate::library::init_ffmpeg().unwrap();
        let dir = TempDir::new("burnin-layout");
        let clip = layered_clip(&dir.0);
        let meta = probe(&clip).unwrap();
        let settings = Settings::default();
        let audio = audio_packets(&clip);
        assert_eq!(audio.len(), 2);

        // The input drawn in: the camera and the logo stay layers.
        let input_only = Layers {
            input: true,
            ..Default::default()
        };
        let out = output_for(&clip);
        assert_eq!(out.file_name().unwrap(), "clip (burned).mp4");
        burn(&meta, input_only, &settings, &out, &mut |_| true).unwrap();
        check(&meta, input_only, &out).unwrap();
        let burned = probe(&out).unwrap();
        assert_eq!(
            layers(&burned),
            Layers {
                camera: true,
                input: false,
                logo: true
            }
        );
        assert!(
            burned
                .details
                .iter()
                .any(|(k, v)| k == "game" && v == "Test")
        );
        audio_copied(&audio, &out);

        // The camera and logo drawn in: the input stays, its file beside the new one.
        let pictures = Layers {
            camera: true,
            logo: true,
            ..Default::default()
        };
        let out2 = output_for(&clip);
        assert_eq!(out2.file_name().unwrap(), "clip (burned) 2.mp4");
        burn(&meta, pictures, &settings, &out2, &mut |_| true).unwrap();
        check(&meta, pictures, &out2).unwrap();
        let burned = probe(&out2).unwrap();
        assert_eq!(
            layers(&burned),
            Layers {
                camera: false,
                input: true,
                logo: false
            }
        );
        audio_copied(&audio, &out2);
        // The check notices a camera that should have stayed.
        assert!(check(&meta, input_only, &out2).is_err());

        // Nothing of what was asked for: nothing is written.
        let out3 = output_for(&clip);
        let plain = Layers {
            camera: true,
            ..Default::default()
        };
        assert!(
            burn(&probe(&out2).unwrap(), plain, &settings, &out3, &mut |_| {
                true
            })
            .is_err()
        );
        assert!(!out3.exists());
    }
}
