//! Resizing captured frames on the GPU, for recording at a resolution other than the screen's.

use std::mem::ManuallyDrop;

use anyhow::{Context, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0,
    D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::core::Interface;

use crate::d3d::Gpu;

/// Scales BGRA textures to a fixed output size using the GPU's video processor.
pub struct GpuScaler {
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    /// Keep the picture's shape with black bars instead of stretching it to fill.
    fit: bool,
    pub width: u32,
    pub height: u32,
}

// Used from the recording thread only; the D3D11 context is multithread-protected.
unsafe impl Send for GpuScaler {}

impl GpuScaler {
    pub fn new(
        gpu: &Gpu,
        in_width: u32,
        in_height: u32,
        out_width: u32,
        out_height: u32,
        fit: bool,
    ) -> Result<Self> {
        unsafe {
            let video_device: ID3D11VideoDevice =
                gpu.device.cast().context("GPU has no video processor")?;
            let video_context: ID3D11VideoContext = gpu.context.cast()?;

            let rate = DXGI_RATIONAL {
                Numerator: 60,
                Denominator: 1,
            };
            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: rate,
                InputWidth: in_width,
                InputHeight: in_height,
                OutputFrameRate: rate,
                OutputWidth: out_width,
                OutputHeight: out_height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator = video_device.CreateVideoProcessorEnumerator(&content)?;
            let processor = video_device.CreateVideoProcessor(&enumerator, 0)?;
            // Plain resize: no automatic sharpening, denoising or colour changes.
            video_context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            // What is outside the picture when it is fitted: black.
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA {
                        R: 0.0,
                        G: 0.0,
                        B: 0.0,
                        A: 1.0,
                    },
                },
            };
            video_context.VideoProcessorSetOutputBackgroundColor(&processor, false, &black);

            let desc = D3D11_TEXTURE2D_DESC {
                Width: out_width,
                Height: out_height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                ..Default::default()
            };
            let mut output = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut output))?;
            let output = output.context("no output texture")?;

            let view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output_view = None;
            video_device.CreateVideoProcessorOutputView(
                &output,
                &enumerator,
                &view_desc,
                Some(&mut output_view),
            )?;

            Ok(Self {
                video_device,
                video_context,
                enumerator,
                processor,
                output,
                output_view: output_view.context("no output view")?,
                fit,
                width: out_width,
                height: out_height,
            })
        }
    }

    /// Scales the top-left `width` x `height` pixels of `source` into the scaler's own output
    /// texture and returns it. A captured window's texture can be larger than the window's
    /// current content, so only the content part is used. The returned texture is reused by
    /// the next call, so consume it before scaling again.
    pub fn scale(
        &self,
        source: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Result<&ID3D11Texture2D> {
        unsafe {
            // A frame's content size can be ahead of its texture while a resized window's
            // capture pool is still catching up, so never ask for more than the texture has.
            let mut actual = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut actual);
            let content = RECT {
                left: 0,
                top: 0,
                right: width.min(actual.Width) as i32,
                bottom: height.min(actual.Height) as i32,
            };
            self.video_context.VideoProcessorSetStreamSourceRect(
                &self.processor,
                0,
                true,
                Some(&content),
            );
            // Without a destination rectangle the picture fills the whole output.
            let target = self.fit.then(|| {
                fit_rect(
                    (content.right as u32, content.bottom as u32),
                    (self.width, self.height),
                )
            });
            self.video_context.VideoProcessorSetStreamDestRect(
                &self.processor,
                0,
                target.is_some(),
                target.as_ref().map(|r| r as *const RECT),
            );
            let view_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let mut input_view = None;
            self.video_device.CreateVideoProcessorInputView(
                source,
                &self.enumerator,
                &view_desc,
                Some(&mut input_view),
            )?;

            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(input_view),
                ..Default::default()
            };
            let result = self.video_context.VideoProcessorBlt(
                &self.processor,
                &self.output_view,
                0,
                std::slice::from_ref(&stream),
            );
            // Release the input view we put in the stream.
            drop(ManuallyDrop::into_inner(stream.pInputSurface));
            result?;
            Ok(&self.output)
        }
    }
}

/// The largest rectangle with the shape of `picture` that fits in `frame`, centred. Edges
/// land on even pixels so the bars stay clean after colour subsampling.
pub fn fit_rect(picture: (u32, u32), frame: (u32, u32)) -> RECT {
    let (pw, ph) = (picture.0.max(1) as u64, picture.1.max(1) as u64);
    let (fw, fh) = (frame.0 as u64, frame.1 as u64);
    let (w, h) = if pw * fh > ph * fw {
        (fw, (fw * ph + pw / 2) / pw)
    } else {
        ((fh * pw + ph / 2) / ph, fh)
    };
    let (w, h) = ((w & !1).max(2).min(fw), (h & !1).max(2).min(fh));
    let left = ((fw - w) / 2) & !1;
    let top = ((fh - h) / 2) & !1;
    RECT {
        left: left as i32,
        top: top as i32,
        right: (left + w) as i32,
        bottom: (top + h) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(r: RECT) -> (i32, i32, i32, i32) {
        (r.left, r.top, r.right - r.left, r.bottom - r.top)
    }

    #[test]
    fn four_by_three_gets_bars_at_the_sides() {
        assert_eq!(
            size(fit_rect((1440, 1080), (1920, 1080))),
            (240, 0, 1440, 1080)
        );
        assert_eq!(
            size(fit_rect((1280, 1024), (2560, 1440))),
            (380, 0, 1800, 1440)
        );
    }

    #[test]
    fn a_wide_picture_gets_bars_above_and_below() {
        assert_eq!(
            size(fit_rect((2560, 1080), (1920, 1080))),
            (0, 134, 1920, 810)
        );
    }

    #[test]
    fn the_same_shape_fills_the_frame() {
        assert_eq!(
            size(fit_rect((1280, 720), (1920, 1080))),
            (0, 0, 1920, 1080)
        );
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::d3d::TEST_PATTERN_BARS;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_USAGE_STAGING,
    };

    /// The pixel at (x, y) of a BGRA texture, as (R, G, B).
    fn pixel(gpu: &Gpu, texture: &ID3D11Texture2D, x: u32, y: u32) -> (u8, u8, u8) {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            gpu.context.CopyResource(&staging, texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let row = (mapped.pData as *const u8).add((y * mapped.RowPitch) as usize);
            let p = row.add((x * 4) as usize);
            let value = (*p.add(2), *p.add(1), *p);
            gpu.context.Unmap(&staging, 0);
            value
        }
    }

    fn near(a: (u8, u8, u8), b: (u8, u8, u8)) -> bool {
        a.0.abs_diff(b.0) < 24 && a.1.abs_diff(b.1) < 24 && a.2.abs_diff(b.2) < 24
    }

    /// A 4:3 picture (a stretched 1440×1080 game) into a 16:9 video, both ways.
    #[test]
    #[ignore]
    fn stretch_fills_and_fit_adds_bars() {
        let gpu = Gpu::new().unwrap();
        // The video processor takes textures that can be drawn to, as captured frames are.
        let pattern = gpu.test_pattern(1440, 1080).unwrap();
        let source = unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            pattern.GetDesc(&mut desc);
            desc.BindFlags = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
            let mut texture = None;
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .unwrap();
            let texture: ID3D11Texture2D = texture.unwrap();
            gpu.context.CopyResource(&texture, &pattern);
            texture
        };
        let first = TEST_PATTERN_BARS[0];
        let last = TEST_PATTERN_BARS[7];

        let stretch = GpuScaler::new(&gpu, 1440, 1080, 1920, 1080, false).unwrap();
        let out = stretch.scale(&source, 1440, 1080).unwrap().clone();
        assert!(
            near(pixel(&gpu, &out, 100, 540), first),
            "{:?}",
            pixel(&gpu, &out, 100, 540)
        );
        assert!(near(pixel(&gpu, &out, 1850, 540), last));

        let fit = GpuScaler::new(&gpu, 1440, 1080, 1920, 1080, true).unwrap();
        let out = fit.scale(&source, 1440, 1080).unwrap().clone();
        assert!(
            near(pixel(&gpu, &out, 100, 540), (0, 0, 0)),
            "{:?}",
            pixel(&gpu, &out, 100, 540)
        );
        assert!(near(pixel(&gpu, &out, 1850, 540), (0, 0, 0)));
        assert!(
            near(pixel(&gpu, &out, 300, 540), first),
            "{:?}",
            pixel(&gpu, &out, 300, 540)
        );
        assert!(
            near(pixel(&gpu, &out, 1620, 540), last),
            "{:?}",
            pixel(&gpu, &out, 1620, 540)
        );
    }
}
