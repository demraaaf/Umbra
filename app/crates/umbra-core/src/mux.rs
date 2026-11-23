//! Writes encoded packets to an MP4 file (or another container). Nothing is re-encoded.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use ffmpeg_next::ffi::{
    AVMediaType, av_interleaved_write_frame, av_packet_alloc, av_packet_free, av_packet_ref,
    av_packet_rescale_ts,
};
use ffmpeg_next::packet::Ref;
use ffmpeg_next::{Dictionary, Packet, Rational, codec, format};

use crate::encode::StreamInfo;

pub struct Mp4Writer {
    output: format::context::Output,
    /// The container: `mp4` unless made with [`Mp4Writer::create_container`].
    container: &'static str,
    /// Time base of the packets handed to `write`, per stream.
    source_time_bases: Vec<Rational>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    FastStart,
    Plain,
    Fragmented,
}

/// MP4 and its relatives take the `movflags` and track-name boxes; other containers do not.
fn is_mp4_family(container: &str) -> bool {
    matches!(container, "mp4" | "mov" | "ipod")
}

/// File-level tags Umbra writes start with this, so they are told apart from other tags.
pub const TAG_PREFIX: &str = "umbra_";

impl Mp4Writer {
    pub fn create(path: &Path, streams: &[StreamInfo]) -> Result<Self> {
        Self::create_with_tags(path, streams, &[])
    }

    /// Like `create`, and stores `tags` (name, value) in the file. Names get [`TAG_PREFIX`]
    /// unless they already have it.
    pub fn create_with_tags(
        path: &Path,
        streams: &[StreamInfo],
        tags: &[(String, String)],
    ) -> Result<Self> {
        Self::open(path, streams, tags, Layout::FastStart)
    }

    /// The index at the end, written in one pass. Players read it from there just as fast;
    /// only a file streamed over a slow network wants it in front.
    pub fn create_plain(
        path: &Path,
        streams: &[StreamInfo],
        tags: &[(String, String)],
    ) -> Result<Self> {
        Self::open(path, streams, tags, Layout::Plain)
    }

    /// A file written as it is recorded, in fragments of about a second: whatever was
    /// written stays playable if Umbra or Windows stops before the end. It ends with an index
    /// for seeking.
    pub fn create_fragmented(
        path: &Path,
        streams: &[StreamInfo],
        tags: &[(String, String)],
    ) -> Result<Self> {
        Self::open(path, streams, tags, Layout::Fragmented)
    }

    /// A file in another container, by FFmpeg muxer name: `mov`, `matroska`, `webm`, `mp3`,
    /// `ipod` (M4A), `wav`, `flac` or `ogg`. MP4-family files get the index in front.
    pub fn create_container(
        path: &Path,
        streams: &[StreamInfo],
        container: &'static str,
    ) -> Result<Self> {
        Self::open_as(path, streams, &[], Layout::FastStart, container)
    }

    fn open(
        path: &Path,
        streams: &[StreamInfo],
        tags: &[(String, String)],
        layout: Layout,
    ) -> Result<Self> {
        Self::open_as(path, streams, tags, layout, "mp4")
    }

    fn open_as(
        path: &Path,
        streams: &[StreamInfo],
        tags: &[(String, String)],
        layout: Layout,
        container: &'static str,
    ) -> Result<Self> {
        let mp4 = is_mp4_family(container);
        let mut output = format::output_as(path, container)
            .with_context(|| format!("cannot create {}", path.display()))?;
        for info in streams {
            let mut stream = output.add_stream(None::<codec::codec::Codec>)?;
            stream.set_parameters(info.params.clone());
            stream.set_time_base(info.time_base);
            if let Some(title) = &info.title {
                let mut metadata = Dictionary::new();
                metadata.set("title", title);
                // The MP4 muxer writes this as the track name.
                if mp4 {
                    metadata.set("handler_name", title);
                }
                stream.set_metadata(metadata);
            }
            unsafe {
                let params = (*stream.as_mut_ptr()).codecpar;
                // Let the muxer pick the tag, except HEVC: `hvc1` plays in more places than `hev1`.
                (*params).codec_tag = 0;
                if mp4
                    && (*params).codec_type == AVMediaType::AVMEDIA_TYPE_VIDEO
                    && (*params).codec_id == codec::Id::HEVC.into()
                {
                    (*params).codec_tag = u32::from_le_bytes(*b"hvc1");
                }
            }
        }
        let mut options = Dictionary::new();
        // Moves the index to the front of the file so clips start playing immediately. A
        // fragmented file has its header at the front anyway.
        let layout = match layout {
            Layout::FastStart => "+faststart",
            Layout::Plain => "",
            // `delay_moov`: a stream that starts late (the camera, opening while the screen
            // already records) keeps its start. Without it, its first frame is put at zero.
            Layout::Fragmented => "+frag_keyframe+empty_moov+default_base_moof+delay_moov",
        };
        if !mp4 {
            // Other containers have no such options.
        } else if tags.is_empty() {
            if !layout.is_empty() {
                options.set("movflags", layout);
            }
        } else {
            let mut metadata = Dictionary::new();
            for (name, value) in tags {
                if name.starts_with(TAG_PREFIX) {
                    metadata.set(name, value);
                } else {
                    metadata.set(&format!("{TAG_PREFIX}{name}"), value);
                }
            }
            output.set_metadata(metadata);
            // Free-form tags need the muxer's metadata keys box.
            options.set("movflags", &format!("{layout}+use_metadata_tags"));
        }
        output
            .write_header_with(options)
            .context("cannot write the file header")?;
        Ok(Self {
            output,
            container,
            source_time_bases: streams.iter().map(|s| s.time_base).collect(),
        })
    }

    /// Writes one packet. `offset` is subtracted from its timestamps (in the stream's
    /// source time base) so a clip cut from the middle of a recording starts at zero.
    pub fn write(&mut self, stream: usize, packet: &Packet, offset: i64) -> Result<()> {
        let source = *self
            .source_time_bases
            .get(stream)
            .ok_or_else(|| anyhow!("no stream {stream}"))?;
        let target = self
            .output
            .stream(stream)
            .ok_or_else(|| anyhow!("no stream {stream}"))?
            .time_base();
        unsafe {
            // Reference the data instead of copying it; the muxer consumes the reference.
            let mut copy = av_packet_alloc();
            if copy.is_null() {
                return Err(anyhow!("av_packet_alloc failed"));
            }
            let mut code = av_packet_ref(copy, packet.as_ptr());
            if code >= 0 {
                (*copy).stream_index = stream as i32;
                (*copy).pts -= offset;
                (*copy).dts -= offset;
                av_packet_rescale_ts(copy, source.into(), target.into());
                code = av_interleaved_write_frame(self.output.as_mut_ptr(), copy);
            }
            av_packet_free(&mut copy);
            if code < 0 {
                return Err(anyhow!("write packet: {}", ffmpeg_next::Error::from(code)));
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.output
            .write_trailer()
            .with_context(|| format!("cannot finish the {} file", self.container))
    }
}
