//! AAC encoding of 48 kHz stereo audio with FFmpeg's built-in encoder.

use std::ffi::CString;
use std::ptr;

use anyhow::{Result, anyhow, bail};
use ffmpeg_next::ffi::{
    AV_CODEC_FLAG_GLOBAL_HEADER, AVCodecContext, AVFrame, AVRational, AVSampleFormat,
    av_channel_layout_copy, av_channel_layout_default, av_frame_alloc, av_frame_free,
    av_frame_get_buffer, av_frame_make_writable, avcodec_alloc_context3,
    avcodec_find_encoder_by_name, avcodec_free_context, avcodec_open2,
    avcodec_parameters_from_context, avcodec_receive_packet, avcodec_send_frame,
};
use ffmpeg_next::packet::Mut;
use ffmpeg_next::{Packet, Rational, codec::Parameters};

use crate::align::{CHUNK, SAMPLE_RATE};
use crate::encode::StreamInfo;

const AVERROR_EAGAIN: i32 = -11;

pub struct AacEncoder {
    ctx: *mut AVCodecContext,
    frame: *mut AVFrame,
}

// Used from one thread at a time.
unsafe impl Send for AacEncoder {}

fn check(code: i32, what: &str) -> Result<()> {
    if code < 0 {
        Err(anyhow!("{what}: {}", ffmpeg_next::Error::from(code)))
    } else {
        Ok(())
    }
}

impl AacEncoder {
    pub fn new(bitrate_kbps: u32, title: &str) -> Result<(Self, StreamInfo)> {
        ffmpeg_next::init()?;
        unsafe {
            let name = CString::new("aac").unwrap();
            let codec = avcodec_find_encoder_by_name(name.as_ptr());
            if codec.is_null() {
                bail!("this FFmpeg build has no AAC encoder");
            }
            let mut this = Self {
                ctx: avcodec_alloc_context3(codec),
                frame: ptr::null_mut(),
            };
            if this.ctx.is_null() {
                bail!("avcodec_alloc_context3 failed");
            }
            let ctx = this.ctx;
            (*ctx).sample_fmt = AVSampleFormat::AV_SAMPLE_FMT_FLTP;
            (*ctx).sample_rate = SAMPLE_RATE as i32;
            av_channel_layout_default(&mut (*ctx).ch_layout, 2);
            (*ctx).bit_rate = bitrate_kbps as i64 * 1000;
            (*ctx).time_base = AVRational {
                num: 1,
                den: SAMPLE_RATE as i32,
            };
            (*ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            check(
                avcodec_open2(ctx, codec, ptr::null_mut()),
                "could not open the AAC encoder",
            )?;
            if (*ctx).frame_size as usize != CHUNK {
                bail!(
                    "AAC encoder wants {} samples per frame, not {CHUNK}",
                    (*ctx).frame_size
                );
            }

            this.frame = av_frame_alloc();
            if this.frame.is_null() {
                bail!("av_frame_alloc failed");
            }
            let frame = this.frame;
            (*frame).format = AVSampleFormat::AV_SAMPLE_FMT_FLTP as i32;
            (*frame).nb_samples = CHUNK as i32;
            (*frame).sample_rate = SAMPLE_RATE as i32;
            check(
                av_channel_layout_copy(&mut (*frame).ch_layout, &(*ctx).ch_layout),
                "av_channel_layout_copy",
            )?;
            check(av_frame_get_buffer(frame, 0), "av_frame_get_buffer")?;

            let mut params = Parameters::new();
            check(
                avcodec_parameters_from_context(params.as_mut_ptr(), ctx),
                "avcodec_parameters_from_context",
            )?;
            let info = StreamInfo {
                params,
                time_base: Rational::new(1, SAMPLE_RATE as i32),
                title: Some(title.to_string()),
            };
            Ok((this, info))
        }
    }

    /// Encodes one chunk of `CHUNK` interleaved stereo samples. `index` is the chunk's
    /// position on the timeline; the packet timestamps are in samples.
    pub fn encode(
        &mut self,
        index: u64,
        interleaved: &[f32],
        sink: &mut dyn FnMut(Packet),
    ) -> Result<()> {
        debug_assert_eq!(interleaved.len(), CHUNK * 2);
        unsafe {
            let frame = self.frame;
            check(av_frame_make_writable(frame), "av_frame_make_writable")?;
            let left = std::slice::from_raw_parts_mut((*frame).data[0] as *mut f32, CHUNK);
            let right = std::slice::from_raw_parts_mut((*frame).data[1] as *mut f32, CHUNK);
            for (i, pair) in interleaved.chunks_exact(2).enumerate() {
                left[i] = pair[0];
                right[i] = pair[1];
            }
            (*frame).pts = (index * CHUNK as u64) as i64;

            check(avcodec_send_frame(self.ctx, frame), "avcodec_send_frame")?;
            loop {
                let mut packet = Packet::empty();
                let code = avcodec_receive_packet(self.ctx, packet.as_mut_ptr());
                if code == AVERROR_EAGAIN {
                    return Ok(());
                }
                check(code, "avcodec_receive_packet")?;
                sink(packet);
            }
        }
    }
}

impl AacEncoder {
    /// Sends what the encoder still holds. Call once, after the last chunk.
    pub fn finish(&mut self, sink: &mut dyn FnMut(Packet)) -> Result<()> {
        unsafe {
            let code = avcodec_send_frame(self.ctx, ptr::null());
            if code < 0 && code != ffmpeg_next::ffi::AVERROR_EOF {
                check(code, "avcodec_send_frame")?;
            }
            loop {
                let mut packet = Packet::empty();
                let code = avcodec_receive_packet(self.ctx, packet.as_mut_ptr());
                if code == AVERROR_EAGAIN || code == ffmpeg_next::ffi::AVERROR_EOF {
                    return Ok(());
                }
                check(code, "avcodec_receive_packet")?;
                sink(packet);
            }
        }
    }
}

impl Drop for AacEncoder {
    fn drop(&mut self) {
        unsafe {
            if !self.frame.is_null() {
                av_frame_free(&mut self.frame);
            }
            if !self.ctx.is_null() {
                avcodec_free_context(&mut self.ctx);
            }
        }
    }
}
