//! Direct3D 11 device shared by screen capture and the hardware encoder.

use anyhow::{Context, Result};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, IDXGIAdapter1, IDXGIFactory6,
};
use windows::core::Interface;

pub const VENDOR_NVIDIA: u32 = 0x10DE;
pub const VENDOR_AMD: u32 = 0x1002;
pub const VENDOR_INTEL: u32 = 0x8086;

#[derive(Clone)]
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub adapter_name: String,
    pub vendor_id: u32,
}

impl Gpu {
    /// Creates a device on the high-performance adapter, which is the one the hardware
    /// encoder runs on. Windows copies frames across adapters if the monitor is on another GPU.
    pub fn new() -> Result<Self> {
        unsafe {
            let factory: IDXGIFactory6 = CreateDXGIFactory1().context("CreateDXGIFactory1")?;
            let adapter: IDXGIAdapter1 = factory
                .EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
                .context("no graphics adapter found")?;
            let desc = adapter.GetDesc1()?;
            let name_len = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            let adapter_name = String::from_utf16_lossy(&desc.Description[..name_len]);

            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("D3D11CreateDevice")?;
            let device: ID3D11Device = device.context("no D3D11 device returned")?;
            let context: ID3D11DeviceContext = context.context("no D3D11 context returned")?;

            // Capture, the encoder and our own copies all touch the immediate context
            // from different threads.
            let multithread: ID3D11Multithread = context.cast()?;
            let _ = multithread.SetMultithreadProtected(true);

            Ok(Self {
                device,
                context,
                adapter_name,
                vendor_id: desc.VendorId,
            })
        }
    }
}

/// Colours of the vertical bars drawn by [`Gpu::test_pattern`], left to right, as (R, G, B).
pub const TEST_PATTERN_BARS: [(u8, u8, u8); 8] = [
    (255, 255, 255),
    (255, 255, 0),
    (0, 255, 255),
    (0, 255, 0),
    (255, 0, 255),
    (255, 0, 0),
    (0, 0, 255),
    (64, 64, 64),
];

impl Gpu {
    /// A texture of solid colour bars, used to check colour accuracy through the encoder.
    pub fn test_pattern(&self, width: u32, height: u32) -> Result<ID3D11Texture2D> {
        let bar_width = width as usize / TEST_PATTERN_BARS.len();
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for (i, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let x = i % width as usize;
            let (r, g, b) = TEST_PATTERN_BARS[(x / bar_width).min(TEST_PATTERN_BARS.len() - 1)];
            pixel.copy_from_slice(&[b, g, r, 255]);
        }
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
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe {
            self.device
                .CreateTexture2D(&desc, Some(&data), Some(&mut texture))
                .context("CreateTexture2D")?;
        }
        texture.context("no texture returned")
    }
}
