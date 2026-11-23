//! Single stills of a monitor or window: screenshots, and the small previews in the
//! capture switcher. Each one opens a short capture session, takes the first frame and
//! closes it again, so it works whether or not the recorder is running.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::Interface;

use crate::capture::{Capture, CaptureTarget};
use crate::d3d::Gpu;
use crate::hdr::{HdrConverter, HdrOutput};
use crate::library::{ImageFormat, encode_image};
use crate::scale::GpuScaler;

/// An RGB24 image.
pub struct Still {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// One graphics device for every still: making one takes longer than the still itself, and
/// the panel takes a still of each source every 5 s. A new one after a driver reset.
fn gpu() -> Result<Gpu> {
    static GPU: Mutex<Option<Gpu>> = Mutex::new(None);
    let mut slot = GPU.lock().unwrap();
    if let Some(gpu) = slot
        .as_ref()
        .filter(|g| unsafe { g.device.GetDeviceRemovedReason() }.is_ok())
    {
        return Ok(gpu.clone());
    }
    let gpu = Gpu::new()?;
    *slot = Some(gpu.clone());
    Ok(gpu)
}

/// Takes one frame of `target`. With `max_width`, it is scaled down on the GPU first.
/// Fails if nothing arrives within a second, which is what a minimised window does.
pub fn grab(target: CaptureTarget, max_width: Option<u32>, cursor: bool) -> Result<Still> {
    let gpu = gpu()?;
    // A screen in HDR is captured as such and converted, as the recorder does; taken as 8-bit
    // it looks washed out.
    let screen = match target {
        CaptureTarget::Monitor(m) => m,
        CaptureTarget::Window(w) => crate::games::monitor_of(w),
    };
    let hdr = crate::hdr::monitor_hdr(screen);
    let capture = Capture::start(&gpu, target, cursor, 60, hdr.is_some())?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let frame = loop {
        if let Some(frame) = capture.take_latest() {
            break frame;
        }
        if Instant::now() > deadline {
            bail!("no picture arrived (a minimised window cannot be captured)");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if frame.width == 0 || frame.height == 0 {
        bail!("the captured surface is empty");
    }

    let (width, height) = match max_width {
        Some(max) if frame.width > max => {
            let height = ((frame.height as u64 * max as u64) / frame.width as u64).max(2) as u32;
            (max & !1, height & !1)
        }
        _ => (frame.width, frame.height),
    };
    // The device's drawing state is shared by every still; one at a time.
    static DRAWING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = DRAWING.lock().unwrap();
    let scaler;
    let mut converter;
    let texture: &ID3D11Texture2D = if let Some(info) = hdr {
        converter = HdrConverter::new(&gpu, HdrOutput::Sdr, info, (width, height), false)?;
        converter.convert(&frame.texture, (frame.width, frame.height))?
    } else if (width, height) != (frame.width, frame.height) {
        scaler = GpuScaler::new(&gpu, frame.width, frame.height, width, height, false)?;
        scaler.scale(&frame.texture, frame.width, frame.height)?
    } else {
        &frame.texture
    };
    let rgb = read_back(&gpu, texture, width, height)?;
    Ok(Still { width, height, rgb })
}

/// Copies the top-left `width` x `height` of a BGRA texture to the CPU as RGB24.
fn read_back(gpu: &Gpu, texture: &ID3D11Texture2D, width: u32, height: u32) -> Result<Vec<u8>> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let mut staging = None;
        gpu.device
            .CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging: ID3D11Texture2D = staging.context("no staging texture")?;
        let region = D3D11_BOX {
            left: 0,
            top: 0,
            front: 0,
            right: width,
            bottom: height,
            back: 1,
        };
        let dst: ID3D11Resource = staging.cast()?;
        let src: ID3D11Resource = texture.cast()?;
        gpu.context
            .CopySubresourceRegion(&dst, 0, 0, 0, 0, &src, 0, Some(&region));

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context
            .Map(&dst, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height as usize {
            let row = std::slice::from_raw_parts(
                (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                width as usize * 4,
            );
            for pixel in row.chunks_exact(4) {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
        }
        gpu.context.Unmap(&dst, 0);
        Ok(rgb)
    }
}

/// Saves a full-size still of `target` to `path` as PNG or JPEG (by the extension).
pub fn save_screenshot(target: CaptureTarget, path: &Path, cursor: bool) -> Result<(u32, u32)> {
    let still = grab(target, None, cursor)?;
    let format = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg { qscale: 2 }
    };
    let bytes = encode_image(&still.rgb, still.width, still.height, format)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Same as clips: a half-written file never carries the final name.
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    std::fs::rename(&partial, path)
        .with_context(|| format!("cannot move screenshot to {}", path.display()))?;
    Ok((still.width, still.height))
}

/// A small JPEG of `target` as a `data:` URL, for the capture switcher.
pub fn preview_data_url(target: CaptureTarget, max_width: u32) -> Result<String> {
    let still = grab(target, Some(max_width.clamp(64, 1280)), false)?;
    let jpeg = encode_image(
        &still.rgb,
        still.width,
        still.height,
        ImageFormat::Jpeg { qscale: 5 },
    )?;
    Ok(format!("data:image/jpeg;base64,{}", base64(&jpeg)))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;

    /// Two stills of the main screen from the one shared device:
    /// `cargo test -p umbra-core snapshot -- --ignored`.
    #[test]
    #[ignore]
    fn stills_share_one_device() {
        let monitor = crate::capture::list_monitors()[0].handle;
        for max_width in [Some(320), None] {
            let still = grab(CaptureTarget::Monitor(monitor), max_width, false).unwrap();
            assert_eq!(still.rgb.len(), (still.width * still.height * 3) as usize);
            assert!(still.rgb.iter().any(|&b| b != 0), "a black still");
        }
    }
}
