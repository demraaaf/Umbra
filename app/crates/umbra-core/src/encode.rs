//! Hardware video encoding through FFmpeg. Frames stay on the GPU: captured textures are
//! copied into an FFmpeg D3D11 frame pool and handed to the encoder without a CPU round trip.

use std::ffi::{CString, c_void};
use std::ptr;

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next::ffi::{
    AV_CODEC_FLAG_GLOBAL_HEADER, AV_HWFRAME_MAP_DIRECT, AVBufferRef, AVCodecContext,
    AVColorPrimaries, AVColorRange, AVColorSpace, AVColorTransferCharacteristic, AVERROR_EOF,
    AVFrame, AVHWDeviceContext, AVHWDeviceType, AVHWFramesContext, AVPictureType, AVPixelFormat,
    AVRational, av_buffer_ref, av_buffer_unref, av_frame_alloc, av_frame_clone, av_frame_free,
    av_hwdevice_ctx_alloc, av_hwdevice_ctx_create_derived, av_hwdevice_ctx_init,
    av_hwframe_ctx_alloc, av_hwframe_ctx_create_derived, av_hwframe_ctx_init,
    av_hwframe_get_buffer, av_hwframe_map, av_opt_set, avcodec_alloc_context3,
    avcodec_find_encoder_by_name, avcodec_free_context, avcodec_open2,
    avcodec_parameters_from_context, avcodec_receive_packet, avcodec_send_frame,
};
use ffmpeg_next::packet::Mut;
use ffmpeg_next::{Packet, Rational, codec::Parameters};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, ID3D11Resource,
    ID3D11Texture2D,
};
use windows::core::Interface;

use crate::d3d::{Gpu, VENDOR_AMD, VENDOR_INTEL, VENDOR_NVIDIA};
use crate::settings::{Multipass, Tune};

const AVERROR_EAGAIN: i32 = -11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
}

impl VideoCodec {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "h264" | "avc" => Some(Self::H264),
            "hevc" | "h265" => Some(Self::Hevc),
            "av1" => Some(Self::Av1),
            _ => None,
        }
    }

    fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Av1 => "av1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// Constant bitrate in kbit/s.
    Cbr { kbps: u32 },
    /// Variable bitrate with a target and a peak, in kbit/s.
    Vbr { kbps: u32, max_kbps: u32 },
    /// Constant quantiser. Lower is higher quality; values near 0 are close to lossless.
    ConstQp { qp: u8 },
    /// Constant quality: the bitrate follows the picture, capped at `max_kbps`.
    ConstQuality { cq: u8, max_kbps: u32 },
}

/// Finer encoder controls. The defaults are what Umbra used before they could be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderTuning {
    pub tune: Tune,
    pub multipass: Multipass,
    /// Frames to look ahead, 0 for none.
    pub lookahead: u8,
    pub spatial_aq: bool,
    pub temporal_aq: bool,
    /// 1 to 15.
    pub aq_strength: u8,
}

impl Default for EncoderTuning {
    fn default() -> Self {
        Self {
            tune: Tune::Hq,
            multipass: Multipass::Off,
            lookahead: 0,
            spatial_aq: false,
            temporal_aq: false,
            aq_strength: 8,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub rate: RateControl,
    /// Encoder speed and quality preset, 1 (fastest) to 7 (best quality).
    pub preset: u8,
    /// Seconds between keyframes. Clips can only start on a keyframe.
    pub keyframe_interval_secs: u32,
    pub tuning: EncoderTuning,
    /// HDR10: 10-bit P010 frames (BT.2020, PQ). HEVC is written as Main 10.
    pub hdr10: bool,
}

/// Everything a muxer needs to know about an encoded stream.
#[derive(Clone)]
pub struct StreamInfo {
    pub params: Parameters,
    pub time_base: Rational,
    /// Track name shown by players that list tracks, such as "Game" or "Microphone".
    pub title: Option<String>,
}

// Layouts from libavutil/hwcontext_d3d11va.h, which the generated bindings leave out.
#[repr(C)]
struct AVD3D11VADeviceContext {
    device: *mut c_void,
    device_context: *mut c_void,
    video_device: *mut c_void,
    video_context: *mut c_void,
    lock: Option<unsafe extern "C" fn(*mut c_void)>,
    unlock: Option<unsafe extern "C" fn(*mut c_void)>,
    lock_ctx: *mut c_void,
    bind_flags: u32,
    misc_flags: u32,
}

#[repr(C)]
struct AVD3D11VAFramesContext {
    texture: *mut c_void,
    bind_flags: u32,
    misc_flags: u32,
    texture_infos: *mut c_void,
}

fn av_check(code: i32, what: &str) -> Result<i32> {
    if code < 0 {
        Err(anyhow!("{what}: {}", ffmpeg_next::Error::from(code)))
    } else {
        Ok(code)
    }
}

/// Sets an encoder option. One the encoder doesn't know is noted in `ignored` and skipped.
unsafe fn set_opt(ctx: *mut AVCodecContext, key: &str, value: &str, ignored: &mut Vec<String>) {
    let k = CString::new(key).unwrap();
    let v = CString::new(value).unwrap();
    let code = unsafe { av_opt_set((*ctx).priv_data, k.as_ptr(), v.as_ptr(), 0) };
    if code < 0 {
        log::warn!("encoder ignored option {key}={value}");
        ignored.push(format!("{key}={value}"));
    }
}

/// The graphics card's own encoder: NVIDIA NVENC, AMD AMF or Intel Quick Sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Nvenc,
    Amf,
    Qsv,
}

impl Backend {
    fn suffix(self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::Amf => "amf",
            Self::Qsv => "qsv",
        }
    }
}

/// NVIDIA's options: presets p1 to p7, tuning, multipass, look-ahead, AQ.
unsafe fn nvenc_options(ctx: *mut AVCodecContext, s: &VideoSettings, ignored: &mut Vec<String>) {
    unsafe {
        set_opt(
            ctx,
            "preset",
            &format!("p{}", s.preset.clamp(1, 7)),
            ignored,
        );
        let t = &s.tuning;
        let tune = match t.tune {
            Tune::Hq => "hq",
            Tune::Ll => "ll",
            Tune::Ull => "ull",
        };
        set_opt(ctx, "tune", tune, ignored);
        set_opt(ctx, "forced-idr", "1", ignored);
        let multipass = match t.multipass {
            Multipass::Off => "disabled",
            Multipass::Quarter => "qres",
            Multipass::Full => "fullres",
        };
        set_opt(ctx, "multipass", multipass, ignored);
        if t.lookahead > 0 {
            set_opt(ctx, "rc-lookahead", &t.lookahead.to_string(), ignored);
        }
        if t.spatial_aq {
            set_opt(ctx, "spatial-aq", "1", ignored);
            set_opt(
                ctx,
                "aq-strength",
                &t.aq_strength.clamp(1, 15).to_string(),
                ignored,
            );
        }
        if t.temporal_aq {
            set_opt(ctx, "temporal-aq", "1", ignored);
        }
        match s.rate {
            RateControl::Cbr { kbps } => {
                let bits = kbps as i64 * 1000;
                set_opt(ctx, "rc", "cbr", ignored);
                (*ctx).bit_rate = bits;
                (*ctx).rc_max_rate = bits;
                (*ctx).rc_buffer_size = bits.min(i32::MAX as i64) as i32;
            }
            RateControl::Vbr { kbps, max_kbps } => {
                set_opt(ctx, "rc", "vbr", ignored);
                (*ctx).bit_rate = kbps as i64 * 1000;
                (*ctx).rc_max_rate = max_kbps.max(kbps) as i64 * 1000;
            }
            RateControl::ConstQp { qp } => {
                set_opt(ctx, "rc", "constqp", ignored);
                set_opt(ctx, "qp", &qp.to_string(), ignored);
            }
            RateControl::ConstQuality { cq, max_kbps } => {
                // VBR with no target: the quality value decides, the peak caps it.
                set_opt(ctx, "rc", "vbr", ignored);
                set_opt(ctx, "cq", &cq.clamp(1, 51).to_string(), ignored);
                (*ctx).bit_rate = 0;
                let peak = max_kbps as i64 * 1000;
                (*ctx).rc_max_rate = peak;
                (*ctx).rc_buffer_size = (peak * 2).min(i32::MAX as i64) as i32;
            }
        }
    }
}

/// AMD's options. Presets 1 to 7 map onto its three quality levels; the first pass is its
/// pre-analysis and spatial AQ its VBAQ. Constant quality uses fixed quantisers, which AMF
/// has on every card.
unsafe fn amf_options(ctx: *mut AVCodecContext, s: &VideoSettings, ignored: &mut Vec<String>) {
    unsafe {
        set_opt(ctx, "usage", "transcoding", ignored);
        set_opt(ctx, "forced_idr", "1", ignored);
        let quality = match s.preset {
            0..=2 => "speed",
            3..=5 => "balanced",
            _ => "quality",
        };
        set_opt(ctx, "quality", quality, ignored);
        let t = &s.tuning;
        if t.multipass != Multipass::Off || t.lookahead > 0 {
            set_opt(ctx, "preanalysis", "1", ignored);
        }
        if t.spatial_aq {
            set_opt(ctx, "vbaq", "1", ignored);
        }
        let fixed_qp = |qp: u8, ignored: &mut Vec<String>| {
            set_opt(ctx, "rc", "cqp", ignored);
            let qp = qp.clamp(0, 51).to_string();
            for key in ["qp_i", "qp_p", "qp_b"] {
                set_opt(ctx, key, &qp, ignored);
            }
        };
        match s.rate {
            RateControl::Cbr { kbps } => {
                let bits = kbps as i64 * 1000;
                set_opt(ctx, "rc", "cbr", ignored);
                (*ctx).bit_rate = bits;
                (*ctx).rc_max_rate = bits;
                (*ctx).rc_buffer_size = bits.min(i32::MAX as i64) as i32;
            }
            RateControl::Vbr { kbps, max_kbps } => {
                set_opt(ctx, "rc", "vbr_peak", ignored);
                (*ctx).bit_rate = kbps as i64 * 1000;
                (*ctx).rc_max_rate = max_kbps.max(kbps) as i64 * 1000;
            }
            RateControl::ConstQp { qp } => fixed_qp(qp, ignored),
            RateControl::ConstQuality { cq, .. } => fixed_qp(cq, ignored),
        }
    }
}

/// Intel's options. Presets 1 to 7 are its veryfast to veryslow; constant quality is its
/// ICQ mode and a fixed quantiser its CQP.
unsafe fn qsv_options(ctx: *mut AVCodecContext, s: &VideoSettings, ignored: &mut Vec<String>) {
    unsafe {
        let preset = match s.preset {
            0 | 1 => "veryfast",
            2 => "faster",
            3 => "fast",
            4 => "medium",
            5 => "slow",
            6 => "slower",
            _ => "veryslow",
        };
        set_opt(ctx, "preset", preset, ignored);
        set_opt(ctx, "forced_idr", "1", ignored);
        let t = &s.tuning;
        if t.lookahead > 0 {
            set_opt(ctx, "look_ahead", "1", ignored);
            set_opt(
                ctx,
                "look_ahead_depth",
                &t.lookahead.max(10).to_string(),
                ignored,
            );
        }
        if t.spatial_aq {
            set_opt(ctx, "adaptive_i", "1", ignored);
            set_opt(ctx, "adaptive_b", "1", ignored);
        }
        match s.rate {
            RateControl::Cbr { kbps } => {
                // Equal target and peak is Quick Sync's constant bitrate.
                let bits = kbps as i64 * 1000;
                (*ctx).bit_rate = bits;
                (*ctx).rc_max_rate = bits;
                (*ctx).rc_buffer_size = bits.min(i32::MAX as i64) as i32;
            }
            RateControl::Vbr { kbps, max_kbps } => {
                (*ctx).bit_rate = kbps as i64 * 1000;
                (*ctx).rc_max_rate = max_kbps.max(kbps) as i64 * 1000;
            }
            RateControl::ConstQp { qp } => {
                (*ctx).flags |= ffmpeg_next::ffi::AV_CODEC_FLAG_QSCALE as i32;
                (*ctx).global_quality = qp as i32 * ffmpeg_next::ffi::FF_QP2LAMBDA as i32;
            }
            RateControl::ConstQuality { cq, max_kbps } => {
                // Intelligent constant quality, kept under the peak where the card allows it.
                (*ctx).global_quality = cq.clamp(1, 51) as i32;
                (*ctx).rc_max_rate = max_kbps as i64 * 1000;
            }
        }
    }
}

pub struct VideoEncoder {
    gpu: Gpu,
    ctx: *mut AVCodecContext,
    device_ref: *mut AVBufferRef,
    frames_ref: *mut AVBufferRef,
    /// Quick Sync works on its own device and frames, made from the D3D11 ones; each captured
    /// frame is mapped across without a copy.
    qsv_device: *mut AVBufferRef,
    qsv_frames: *mut AVBufferRef,
    pub backend: Backend,
    /// Last frame sent, kept so it can be sent again when the screen did not change.
    last: *mut AVFrame,
    info: StreamInfo,
    pub encoder_name: String,
    pub settings: VideoSettings,
    /// Options this encoder did not accept, such as temporal AQ on a codec without it.
    pub ignored_options: Vec<String>,
    /// Set by [`time_keyframes`](Self::time_keyframes): ticks between forced keyframes, and
    /// the tick the next one is due.
    key_every: Option<i64>,
    next_key: i64,
}

// The encoder is used from one thread at a time; FFmpeg contexts are not tied to a thread.
unsafe impl Send for VideoEncoder {}

impl VideoEncoder {
    pub fn new(gpu: &Gpu, settings: VideoSettings) -> Result<Self> {
        ffmpeg_next::init().context("ffmpeg init")?;
        let backend = match gpu.vendor_id {
            VENDOR_NVIDIA => Backend::Nvenc,
            VENDOR_AMD => Backend::Amf,
            VENDOR_INTEL => Backend::Qsv,
            other => bail!(
                "{} (vendor {other:#06x}) has no hardware encoder Umbra can use",
                gpu.adapter_name
            ),
        };
        let encoder_name = format!("{}_{}", settings.codec.ffmpeg_name(), backend.suffix());

        let mut this = Self {
            gpu: gpu.clone(),
            ctx: ptr::null_mut(),
            device_ref: ptr::null_mut(),
            frames_ref: ptr::null_mut(),
            qsv_device: ptr::null_mut(),
            qsv_frames: ptr::null_mut(),
            backend,
            last: ptr::null_mut(),
            info: StreamInfo {
                params: Parameters::new(),
                time_base: Rational::new(1, settings.fps as i32),
                title: None,
            },
            encoder_name,
            settings,
            ignored_options: Vec::new(),
            key_every: None,
            next_key: 0,
        };
        // On error, Drop frees whatever was allocated so far.
        unsafe { this.open()? };
        Ok(this)
    }

    unsafe fn open(&mut self) -> Result<()> {
        unsafe {
            let s = &self.settings;
            let name = CString::new(self.encoder_name.as_str()).unwrap();
            let codec = avcodec_find_encoder_by_name(name.as_ptr());
            if codec.is_null() {
                bail!("this FFmpeg build has no {} encoder", self.encoder_name);
            }

            // Wrap our D3D11 device so FFmpeg and capture share textures.
            self.device_ref = av_hwdevice_ctx_alloc(AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
            if self.device_ref.is_null() {
                bail!("av_hwdevice_ctx_alloc failed");
            }
            let device_ctx = (*self.device_ref).data as *mut AVHWDeviceContext;
            let d3d_ctx = (*device_ctx).hwctx as *mut AVD3D11VADeviceContext;
            // FFmpeg takes ownership of this reference and releases it with the context.
            (*d3d_ctx).device = self.gpu.device.clone().into_raw();
            av_check(
                av_hwdevice_ctx_init(self.device_ref),
                "av_hwdevice_ctx_init",
            )?;

            self.frames_ref = av_hwframe_ctx_alloc(self.device_ref);
            if self.frames_ref.is_null() {
                bail!("av_hwframe_ctx_alloc failed");
            }
            let frames_ctx = (*self.frames_ref).data as *mut AVHWFramesContext;
            (*frames_ctx).format = AVPixelFormat::AV_PIX_FMT_D3D11;
            (*frames_ctx).sw_format = if s.hdr10 {
                AVPixelFormat::AV_PIX_FMT_P010LE
            } else {
                AVPixelFormat::AV_PIX_FMT_BGRA
            };
            (*frames_ctx).width = s.width as i32;
            (*frames_ctx).height = s.height as i32;
            // Zero means separate textures allocated on demand instead of one texture array.
            // Quick Sync can only take over a fixed pool, so it gets one with room for the
            // frames the encoder holds on to.
            (*frames_ctx).initial_pool_size = if self.backend == Backend::Qsv { 24 } else { 0 };
            let d3d_frames = (*frames_ctx).hwctx as *mut AVD3D11VAFramesContext;
            (*d3d_frames).bind_flags =
                (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
            av_check(av_hwframe_ctx_init(self.frames_ref), "av_hwframe_ctx_init")?;

            let ctx = avcodec_alloc_context3(codec);
            if ctx.is_null() {
                bail!("avcodec_alloc_context3 failed");
            }
            self.ctx = ctx;
            (*ctx).width = s.width as i32;
            (*ctx).height = s.height as i32;
            (*ctx).time_base = AVRational {
                num: 1,
                den: s.fps as i32,
            };
            (*ctx).framerate = AVRational {
                num: s.fps as i32,
                den: 1,
            };
            (*ctx).pix_fmt = if self.backend == Backend::Qsv {
                AVPixelFormat::AV_PIX_FMT_QSV
            } else {
                AVPixelFormat::AV_PIX_FMT_D3D11
            };
            (*ctx).gop_size = (s.fps * s.keyframe_interval_secs.max(1)) as i32;
            // No B-frames: packets leave in display order, which keeps the replay buffer simple.
            (*ctx).max_b_frames = 0;
            // Parameter sets go into the stream header instead of every keyframe (needed for MP4).
            (*ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            if s.hdr10 {
                (*ctx).colorspace = AVColorSpace::AVCOL_SPC_BT2020_NCL;
                (*ctx).color_primaries = AVColorPrimaries::AVCOL_PRI_BT2020;
                (*ctx).color_trc = AVColorTransferCharacteristic::AVCOL_TRC_SMPTE2084;
            } else {
                (*ctx).colorspace = AVColorSpace::AVCOL_SPC_BT709;
                (*ctx).color_primaries = AVColorPrimaries::AVCOL_PRI_BT709;
                (*ctx).color_trc = AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            }
            (*ctx).color_range = AVColorRange::AVCOL_RANGE_MPEG;
            if self.backend == Backend::Qsv {
                av_check(
                    av_hwdevice_ctx_create_derived(
                        &mut self.qsv_device,
                        AVHWDeviceType::AV_HWDEVICE_TYPE_QSV,
                        self.device_ref,
                        0,
                    ),
                    "cannot open Quick Sync on this graphics card",
                )?;
                av_check(
                    av_hwframe_ctx_create_derived(
                        &mut self.qsv_frames,
                        AVPixelFormat::AV_PIX_FMT_QSV,
                        self.qsv_device,
                        self.frames_ref,
                        AV_HWFRAME_MAP_DIRECT as i32,
                    ),
                    "cannot share frames with Quick Sync",
                )?;
                (*ctx).hw_device_ctx = av_buffer_ref(self.qsv_device);
                (*ctx).hw_frames_ctx = av_buffer_ref(self.qsv_frames);
            } else {
                (*ctx).hw_device_ctx = av_buffer_ref(self.device_ref);
                (*ctx).hw_frames_ctx = av_buffer_ref(self.frames_ref);
            }

            let ignored = &mut self.ignored_options;
            match self.backend {
                Backend::Nvenc => nvenc_options(ctx, s, ignored),
                Backend::Amf => amf_options(ctx, s, ignored),
                Backend::Qsv => qsv_options(ctx, s, ignored),
            }
            if s.hdr10 && s.codec == VideoCodec::Hevc {
                set_opt(ctx, "profile", "main10", ignored);
            }

            av_check(
                avcodec_open2(ctx, codec, ptr::null_mut()),
                &format!("could not open {}", self.encoder_name),
            )?;

            av_check(
                avcodec_parameters_from_context(self.info.params.as_mut_ptr(), ctx),
                "avcodec_parameters_from_context",
            )?;
            self.info.time_base = Rational::new((*ctx).time_base.num, (*ctx).time_base.den);
            Ok(())
        }
    }

    pub fn stream_info(&self) -> StreamInfo {
        self.info.clone()
    }

    /// Encodes a new frame from `texture`. `pts` counts frames at the configured FPS.
    pub fn encode_texture(
        &mut self,
        texture: &ID3D11Texture2D,
        pts: i64,
        sink: &mut dyn FnMut(Packet),
    ) -> Result<()> {
        unsafe {
            let mut frame = av_frame_alloc();
            if frame.is_null() {
                bail!("av_frame_alloc failed");
            }
            if let Err(err) = av_check(
                av_hwframe_get_buffer(self.frames_ref, frame, 0),
                "av_hwframe_get_buffer",
            ) {
                av_frame_free(&mut frame);
                return Err(err);
            }

            let dst_raw = (*frame).data[0] as *mut c_void;
            let dst_index = (*frame).data[1] as usize as u32;
            let dst = ID3D11Texture2D::from_raw_borrowed(&dst_raw)
                .context("encoder frame has no texture")?;
            let dst: ID3D11Resource = dst.cast()?;
            let src: ID3D11Resource = texture.cast()?;

            let device_ctx = (*self.device_ref).data as *mut AVHWDeviceContext;
            let d3d_ctx = (*device_ctx).hwctx as *mut AVD3D11VADeviceContext;
            if let Some(lock) = (*d3d_ctx).lock {
                lock((*d3d_ctx).lock_ctx);
            }
            // Only the top-left region the encoder was built for, so a source one pixel
            // wider than an even size is cropped instead of rejected.
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: self.settings.width,
                bottom: self.settings.height,
                back: 1,
            };
            self.gpu.context.CopySubresourceRegion(
                &dst,
                dst_index,
                0,
                0,
                0,
                &src,
                0,
                Some(&region),
            );
            if let Some(unlock) = (*d3d_ctx).unlock {
                unlock((*d3d_ctx).lock_ctx);
            }

            if !self.qsv_frames.is_null() {
                // The same texture, seen as a Quick Sync surface; the D3D11 frame stays
                // referenced by the mapped one.
                let mut mapped = av_frame_alloc();
                if mapped.is_null() {
                    av_frame_free(&mut frame);
                    bail!("av_frame_alloc failed");
                }
                (*mapped).format = AVPixelFormat::AV_PIX_FMT_QSV as i32;
                (*mapped).hw_frames_ctx = av_buffer_ref(self.qsv_frames);
                let code = av_hwframe_map(mapped, frame, AV_HWFRAME_MAP_DIRECT as i32);
                av_frame_free(&mut frame);
                if let Err(err) = av_check(code, "cannot hand the frame to Quick Sync") {
                    av_frame_free(&mut mapped);
                    return Err(err);
                }
                frame = mapped;
            }
            (*frame).pts = pts;
            if !self.last.is_null() {
                av_frame_free(&mut self.last);
            }
            self.last = frame;
            self.send(frame, sink)
        }
    }

    /// Places keyframes by time instead of by frame count, for a recording that leaves out
    /// frames of a still screen: a clip can then still start within the keyframe interval.
    // ponytail: relies on a forced keyframe restarting the encoder's own GOP count (NVENC,
    // AMF and Quick Sync do); if one does not, it adds a spare keyframe now and then.
    pub fn time_keyframes(&mut self) {
        let s = &self.settings;
        self.key_every = Some((s.fps * s.keyframe_interval_secs.max(1)) as i64);
    }

    /// Encodes the previous frame again under a new timestamp. Returns false if there is
    /// no previous frame yet.
    pub fn encode_repeat(&mut self, pts: i64, sink: &mut dyn FnMut(Packet)) -> Result<bool> {
        if self.last.is_null() {
            return Ok(false);
        }
        unsafe {
            let mut frame = av_frame_clone(self.last);
            if frame.is_null() {
                bail!("av_frame_clone failed");
            }
            (*frame).pts = pts;
            let result = self.send(frame, sink);
            av_frame_free(&mut frame);
            result.map(|_| true)
        }
    }

    /// Flushes frames still inside the encoder. Call once, at the end.
    pub fn finish(&mut self, sink: &mut dyn FnMut(Packet)) -> Result<()> {
        unsafe { self.send(ptr::null_mut(), sink) }
    }

    unsafe fn send(&mut self, frame: *mut AVFrame, sink: &mut dyn FnMut(Packet)) -> Result<()> {
        unsafe {
            if let Some(every) = self.key_every
                && !frame.is_null()
            {
                (*frame).pict_type = if (*frame).pts >= self.next_key {
                    self.next_key = (*frame).pts + every;
                    AVPictureType::AV_PICTURE_TYPE_I
                } else {
                    AVPictureType::AV_PICTURE_TYPE_NONE
                };
            }
            loop {
                let code = avcodec_send_frame(self.ctx, frame);
                if code == AVERROR_EAGAIN {
                    // Output queue is full; make room and retry.
                    self.drain(sink)?;
                    continue;
                }
                if code == AVERROR_EOF && frame.is_null() {
                    break;
                }
                av_check(code, "avcodec_send_frame")?;
                break;
            }
            self.drain(sink)
        }
    }

    unsafe fn drain(&mut self, sink: &mut dyn FnMut(Packet)) -> Result<()> {
        unsafe {
            loop {
                let mut packet = Packet::empty();
                let code = avcodec_receive_packet(self.ctx, packet.as_mut_ptr());
                if code == AVERROR_EAGAIN || code == AVERROR_EOF {
                    return Ok(());
                }
                av_check(code, "avcodec_receive_packet")?;
                if packet.duration() == 0 {
                    // One frame, so the last packet of a clip has a length too.
                    packet.set_duration(1);
                }
                sink(packet);
            }
        }
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        unsafe {
            if !self.last.is_null() {
                av_frame_free(&mut self.last);
            }
            if !self.ctx.is_null() {
                avcodec_free_context(&mut self.ctx);
            }
            if !self.qsv_frames.is_null() {
                av_buffer_unref(&mut self.qsv_frames);
            }
            if !self.qsv_device.is_null() {
                av_buffer_unref(&mut self.qsv_device);
            }
            if !self.frames_ref.is_null() {
                av_buffer_unref(&mut self.frames_ref);
            }
            if !self.device_ref.is_null() {
                av_buffer_unref(&mut self.device_ref);
            }
        }
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;

    /// Every fine-tuning option, on each codec: `cargo test -p umbra-core encode -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn every_option_is_accepted() {
        let gpu = Gpu::new().unwrap();
        for codec in [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1] {
            for rate in [
                RateControl::ConstQuality {
                    cq: 23,
                    max_kbps: 80_000,
                },
                RateControl::Vbr {
                    kbps: 40_000,
                    max_kbps: 60_000,
                },
            ] {
                let encoder = VideoEncoder::new(
                    &gpu,
                    VideoSettings {
                        codec,
                        width: 1920,
                        height: 1080,
                        fps: 60,
                        rate,
                        preset: 7,
                        keyframe_interval_secs: 2,
                        hdr10: false,
                        tuning: EncoderTuning {
                            tune: Tune::Hq,
                            multipass: Multipass::Full,
                            lookahead: 16,
                            spatial_aq: true,
                            temporal_aq: true,
                            aq_strength: 12,
                        },
                    },
                )
                .unwrap();
                println!("{codec:?} {rate:?}: ignored {:?}", encoder.ignored_options);
                assert!(encoder.ignored_options.is_empty());
            }
        }
    }
}
