//! HDR games. With Windows' HDR on, the desktop is composed in a wide, bright colour space
//! (scRGB: linear, BT.709 primaries, 1.0 = 80 nits, highlights far above 1). Captured as
//! ordinary 8-bit it looks washed out or clipped, so on an HDR screen Umbra captures 16-bit
//! float frames and converts them on the GPU, either
//!
//! - to SDR, for videos that look right everywhere: brightness is rolled off with the BT.2390
//!   curve (in the PQ domain, like TVs and players do), with Windows' "SDR content brightness"
//!   as white and the screen's peak as the brightest, then written as ordinary sRGB; or
//! - to HDR10: BT.2020 primaries, PQ, 10-bit 4:2:0 (P010), for HEVC Main 10 or AV1.
//!
//! The conversion also scales the picture to the recording's size, fitted or stretched.

use std::ffi::c_void;

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_SRV_DIMENSION_TEXTURE2D, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BUFFER_DESC, D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RENDER_TARGET_VIEW_DESC_0, D3D11_RTV_DIMENSION_TEXTURE2D,
    D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0,
    D3D11_TEX2D_RTV, D3D11_TEX2D_SRV, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_VIEWPORT, ID3D11Buffer, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11SamplerState, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_P010, DXGI_FORMAT_R16_UNORM, DXGI_FORMAT_R16G16_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, IDXGIOutput6};
use windows::core::{Interface, PCSTR, s};

use crate::d3d::Gpu;

/// What Windows says about an HDR screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrInfo {
    /// How bright SDR white is on it (Windows' "SDR content brightness"), in nits.
    pub sdr_white_nits: f32,
    /// The brightest it shows, in nits.
    pub peak_nits: f32,
}

/// The HDR state of the monitor (raw HMONITOR): `None` when its HDR is off.
pub fn monitor_hdr(monitor: isize) -> Option<HdrInfo> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut a = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a) {
            a += 1;
            let mut o = 0;
            while let Ok(output) = adapter.EnumOutputs(o) {
                o += 1;
                let Ok(desc) = output.GetDesc() else { continue };
                if desc.Monitor.0 as isize != monitor {
                    continue;
                }
                let output6: IDXGIOutput6 = output.cast().ok()?;
                let desc1 = output6.GetDesc1().ok()?;
                if desc1.ColorSpace != DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 {
                    return None;
                }
                let len = desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(32);
                let device = String::from_utf16_lossy(&desc.DeviceName[..len]);
                let sdr_white_nits = sdr_white_level(&device).unwrap_or(200.0);
                let peak_nits = if desc1.MaxLuminance > 80.0 {
                    desc1.MaxLuminance
                } else {
                    1000.0
                };
                return Some(HdrInfo {
                    sdr_white_nits,
                    peak_nits: peak_nits.max(sdr_white_nits),
                });
            }
        }
        None
    }
}

/// Windows' "SDR content brightness" for the display `device` (`\\.\DISPLAY1`), in nits.
fn sdr_white_level(device: &str) -> Option<f32> {
    unsafe {
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
            .ok()
            .ok()?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
        .ok()
        .ok()?;
        for path in &paths[..path_count as usize] {
            let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: path.sourceInfo.adapterId,
                    id: path.sourceInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 {
                continue;
            }
            let name = &source.viewGdiDeviceName;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            if !String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case(device) {
                continue;
            }
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut white.header) != 0 || white.SDRWhiteLevel == 0 {
                return None;
            }
            // In thousandths of 80 nits.
            return Some(white.SDRWhiteLevel as f32 / 1000.0 * 80.0);
        }
        None
    }
}

/// What the conversion writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrOutput {
    /// Tone-mapped 8-bit sRGB (BGRA), for any codec.
    Sdr,
    /// HDR10: BT.2020, PQ, 10-bit 4:2:0 (P010), for HEVC Main 10 or AV1.
    Hdr10,
}

const SHADER: &str = r#"
Texture2D<float4> src : register(t0);
SamplerState smp : register(s0);
cbuffer Params : register(b0) {
    float2 srcScale;   // content size / texture size
    float sdrWhite;    // nits
    float peak;        // nits
};
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
VSOut vs(uint id : SV_VertexID) {
    VSOut o;
    o.uv = float2((id << 1) & 2, id & 2);
    o.pos = float4(o.uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}

static const float m1 = 0.1593017578125, m2 = 78.84375;
static const float c1 = 0.8359375, c2 = 18.8515625, c3 = 18.6875;
float pq(float l) {            // l: 0..1 of 10000 nits
    float p = pow(max(l, 0), m1);
    return pow((c1 + c2 * p) / (1 + c3 * p), m2);
}
float3 pq3(float3 l) { return float3(pq(l.r), pq(l.g), pq(l.b)); }
float pqInv(float e) {
    float p = pow(max(e, 0), 1 / m2);
    return pow(max(p - c1, 0) / (c2 - c3 * p), 1 / m1);
}

// Linear light in nits, BT.709 primaries.
float3 nits(float2 uv) { return max(src.Sample(smp, uv * srcScale).rgb, 0) * 80.0; }

// BT.2390: rolls brightness from the screen's peak down to SDR white, in the PQ domain.
float eetf(float l) {
    float srcMax = pq(peak / 10000);
    float e = pq(min(l, peak) / 10000) / srcMax;
    float maxLum = pq(sdrWhite / 10000) / srcMax;
    float ks = 1.5 * maxLum - 0.5;
    if (e > ks && ks < 1) {
        float t = (e - ks) / (1 - ks);
        float t2 = t * t, t3 = t2 * t;
        e = (2 * t3 - 3 * t2 + 1) * ks + (t3 - 2 * t2 + t) * (1 - ks) + (-2 * t3 + 3 * t2) * maxLum;
    }
    return pqInv(e * srcMax) * 10000;
}
float srgb(float c) { return c <= 0.0031308 ? 12.92 * c : 1.055 * pow(c, 1 / 2.4) - 0.055; }

float4 ps_sdr(VSOut i) : SV_Target {
    float3 c = nits(i.uv);
    float y = dot(c, float3(0.2126, 0.7152, 0.0722));
    if (peak > sdrWhite && y > 0) c *= eetf(y) / y;
    c = saturate(c / sdrWhite);
    return float4(srgb(c.r), srgb(c.g), srgb(c.b), 1);
}

// HDR10: BT.2020 primaries, PQ, then Y'CbCr (BT.2020 non-constant luminance), limited range,
// as P010 stores it: 10 bits in the top of 16.
float3 ycbcr(float2 uv) {
    float3 c = nits(uv);
    float3 c2020 = float3(
        dot(c, float3(0.6274, 0.3293, 0.0433)),
        dot(c, float3(0.0691, 0.9195, 0.0114)),
        dot(c, float3(0.0164, 0.0880, 0.8956)));
    float3 e = pq3(max(c2020, 0) / 10000);
    float y = dot(e, float3(0.2627, 0.6780, 0.0593));
    return float3(y, (e.b - y) / 1.8814, (e.r - y) / 1.4746);
}
static const float P010 = 64.0 / 65535.0;
float ps_y(VSOut i) : SV_Target { return (64 + 876 * ycbcr(i.uv).x) * P010; }
float2 ps_uv(VSOut i) : SV_Target {
    float3 v = ycbcr(i.uv);
    return float2(512 + 896 * v.y, 512 + 896 * v.z) * P010;
}
"#;

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SHADER.as_ptr() as *const c_void,
            SHADER.len(),
            None,
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(err) = result {
        let detail = errors
            .map(|e| unsafe {
                let bytes = std::slice::from_raw_parts(
                    e.GetBufferPointer() as *const u8,
                    e.GetBufferSize(),
                );
                String::from_utf8_lossy(bytes).into_owned()
            })
            .unwrap_or_default();
        bail!("cannot compile the HDR shader: {err} {detail}");
    }
    code.ok_or_else(|| anyhow!("the HDR shader compiled to nothing"))
}

fn bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

#[repr(C)]
struct Params {
    src_scale: [f32; 2],
    sdr_white: f32,
    peak: f32,
}

/// Converts captured HDR frames to the recording's picture.
pub struct HdrConverter {
    gpu: Gpu,
    output: HdrOutput,
    info: HdrInfo,
    vs: ID3D11VertexShader,
    /// SDR: one shader; HDR10: luma, then chroma.
    pixel: Vec<ID3D11PixelShader>,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    texture: ID3D11Texture2D,
    /// One per plane.
    targets: Vec<ID3D11RenderTargetView>,
    width: u32,
    height: u32,
    fit: bool,
}

// Used from the recording thread only; the D3D11 context is multithread-protected.
unsafe impl Send for HdrConverter {}

impl HdrConverter {
    pub fn new(
        gpu: &Gpu,
        output: HdrOutput,
        info: HdrInfo,
        (width, height): (u32, u32),
        fit: bool,
    ) -> Result<Self> {
        let device = &gpu.device;
        unsafe {
            let vs_code = compile(s!("vs"), s!("vs_5_0"))?;
            let mut vs = None;
            device.CreateVertexShader(bytes(&vs_code), None, Some(&mut vs))?;
            let entries: &[PCSTR] = match output {
                HdrOutput::Sdr => &[s!("ps_sdr")],
                HdrOutput::Hdr10 => &[s!("ps_y"), s!("ps_uv")],
            };
            let mut pixel = Vec::new();
            for entry in entries {
                let code = compile(*entry, s!("ps_5_0"))?;
                let mut ps = None;
                device.CreatePixelShader(bytes(&code), None, Some(&mut ps))?;
                pixel.push(ps.context("no pixel shader")?);
            }
            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    ComparisonFunc: D3D11_COMPARISON_NEVER,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            let mut params = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: size_of::<Params>().next_multiple_of(16) as u32,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut params),
            )?;

            let format = match output {
                HdrOutput::Sdr => DXGI_FORMAT_B8G8R8A8_UNORM,
                HdrOutput::Hdr10 => DXGI_FORMAT_P010,
            };
            let mut texture = None;
            device
                .CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: width,
                        Height: height,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: format,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut texture),
                )
                .context("cannot make the converted picture (the graphics card may not render to 10-bit video)")?;
            let texture = texture.context("no texture")?;
            let view = |format: DXGI_FORMAT| -> Result<ID3D11RenderTargetView> {
                let mut rtv = None;
                device.CreateRenderTargetView(
                    &texture,
                    Some(&D3D11_RENDER_TARGET_VIEW_DESC {
                        Format: format,
                        ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
                        },
                    }),
                    Some(&mut rtv),
                )?;
                rtv.context("no render target view")
            };
            let targets = match output {
                HdrOutput::Sdr => vec![view(DXGI_FORMAT_B8G8R8A8_UNORM)?],
                // The luma plane through a one-channel view, chroma through a two-channel one.
                HdrOutput::Hdr10 => vec![
                    view(DXGI_FORMAT_R16_UNORM)?,
                    view(DXGI_FORMAT_R16G16_UNORM)?,
                ],
            };
            Ok(Self {
                gpu: gpu.clone(),
                output,
                info,
                vs: vs.context("no vertex shader")?,
                pixel,
                sampler: sampler.context("no sampler")?,
                params: params.context("no constant buffer")?,
                texture,
                targets,
                width,
                height,
                fit,
            })
        }
    }

    pub fn output(&self) -> HdrOutput {
        self.output
    }

    /// Converts the top-left `content` part of `source` (a 16-bit float capture) into the
    /// converter's own texture and returns it; the next call reuses it.
    pub fn convert(
        &mut self,
        source: &ID3D11Texture2D,
        content: (u32, u32),
    ) -> Result<&ID3D11Texture2D> {
        let ctx = &self.gpu.context;
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut desc);
            let mut srv = None;
            self.gpu.device.CreateShaderResourceView(
                source,
                Some(&D3D11_SHADER_RESOURCE_VIEW_DESC {
                    Format: desc.Format,
                    ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_SRV {
                            MostDetailedMip: 0,
                            MipLevels: 1,
                        },
                    },
                }),
                Some(&mut srv),
            )?;
            let params = Params {
                src_scale: [
                    content.0 as f32 / desc.Width.max(1) as f32,
                    content.1 as f32 / desc.Height.max(1) as f32,
                ],
                sdr_white: self.info.sdr_white_nits,
                peak: self.info.peak_nits,
            };
            ctx.UpdateSubresource(
                &self.params,
                0,
                None,
                &params as *const _ as *const c_void,
                0,
                0,
            );

            // Where the picture goes: all of it, or fitted with black bars.
            let (w, h) = (self.width as f32, self.height as f32);
            let (mut x, mut y, mut vw, mut vh) = (0.0, 0.0, w, h);
            if self.fit && content.0 > 0 && content.1 > 0 {
                let scale = (w / content.0 as f32).min(h / content.1 as f32);
                vw = (content.0 as f32 * scale).round();
                vh = (content.1 as f32 * scale).round();
                x = ((w - vw) / 2.0).floor();
                y = ((h - vh) / 2.0).floor();
            }
            let black: [&[f32; 4]; 2] = match self.output {
                HdrOutput::Sdr => [&[0.0, 0.0, 0.0, 1.0], &[0.0; 4]],
                HdrOutput::Hdr10 => [
                    &[64.0 * 64.0 / 65535.0, 0.0, 0.0, 0.0],
                    &[512.0 * 64.0 / 65535.0, 512.0 * 64.0 / 65535.0, 0.0, 0.0],
                ],
            };

            ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShaderResources(0, Some(&[srv]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
            for (plane, (target, ps)) in self.targets.iter().zip(&self.pixel).enumerate() {
                // Chroma is at half size.
                let div = if plane == 0 { 1.0 } else { 2.0 };
                ctx.ClearRenderTargetView(target, black[plane]);
                ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
                ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: x / div,
                    TopLeftY: y / div,
                    Width: vw / div,
                    Height: vh / div,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
                ctx.PSSetShader(ps, None);
                ctx.Draw(3, 0);
            }
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
        }
        Ok(&self.texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SUBRESOURCE_DATA,
        D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;

    /// A 4x1 scRGB texture: black, SDR white, twice SDR white, and far over the peak.
    fn source(gpu: &Gpu, sdr_white: f32) -> ID3D11Texture2D {
        let half = |v: f32| half_bits(v);
        let white = sdr_white / 80.0;
        let pixels: Vec<u16> = [0.0, white, white * 2.0, 100.0]
            .iter()
            .flat_map(|&v| [half(v), half(v), half(v), half(1.0)])
            .collect();
        unsafe {
            let mut texture = None;
            gpu.device
                .CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: 4,
                        Height: 1,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                        ..Default::default()
                    },
                    Some(&D3D11_SUBRESOURCE_DATA {
                        pSysMem: pixels.as_ptr() as *const c_void,
                        SysMemPitch: 4 * 8,
                        SysMemSlicePitch: 0,
                    }),
                    Some(&mut texture),
                )
                .unwrap();
            texture.unwrap()
        }
    }

    /// An f32 as IEEE half bits (enough for these test values).
    fn half_bits(v: f32) -> u16 {
        if v == 0.0 {
            return 0;
        }
        let bits = v.to_bits();
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let mantissa = (bits >> 13) & 0x3ff;
        ((exp.clamp(0, 30) as u16) << 10) | mantissa as u16
    }

    fn read_bgra(gpu: &Gpu, texture: &ID3D11Texture2D) -> Vec<[u8; 4]> {
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
            let row =
                std::slice::from_raw_parts(mapped.pData as *const [u8; 4], desc.Width as usize)
                    .to_vec();
            gpu.context.Unmap(&staging, 0);
            row
        }
    }

    #[test]
    fn hdr_is_rolled_down_to_sdr() {
        let gpu = Gpu::new().unwrap();
        let info = HdrInfo {
            sdr_white_nits: 240.0,
            peak_nits: 1000.0,
        };
        let mut conv = HdrConverter::new(&gpu, HdrOutput::Sdr, info, (4, 1), false).unwrap();
        let src = source(&gpu, 240.0);
        let out = read_bgra(&gpu, conv.convert(&src, (4, 1)).unwrap()).to_vec();
        let grey: Vec<u8> = out.iter().map(|p| p[1]).collect();
        eprintln!("SDR from HDR: black, SDR white, 2x white, far over the peak = {grey:?}");
        // Black stays black; SDR white stays close to white; brighter is brighter still, and
        // nothing past the peak goes over full white.
        assert_eq!(grey[0], 0, "{grey:?}");
        assert!(grey[1] > 225, "{grey:?}");
        assert!(grey[2] > grey[1], "{grey:?}");
        assert!(grey[3] >= grey[2], "{grey:?}");
        assert!(
            out.iter().all(|p| p[0] == p[1] && p[1] == p[2]),
            "a grey stays grey: {out:?}"
        );
    }

    #[test]
    fn hdr10_makes_a_10_bit_picture() {
        let gpu = Gpu::new().unwrap();
        let info = HdrInfo {
            sdr_white_nits: 203.0,
            peak_nits: 1000.0,
        };
        match HdrConverter::new(&gpu, HdrOutput::Hdr10, info, (4, 2), false) {
            Ok(mut conv) => {
                let src = source(&gpu, 203.0);
                conv.convert(&src, (4, 1)).unwrap();
            }
            // Some cards cannot render to P010; the recorder falls back to SDR there.
            Err(err) => eprintln!("no P010 rendering here: {err:#}"),
        }
    }

    /// A synthetic HDR picture converted to 10-bit and encoded as HEVC Main 10: the file says
    /// 10-bit, BT.2020 and PQ, which is what HDR players look for.
    #[test]
    fn hdr10_encodes_as_hevc_main10() {
        use crate::encode::{EncoderTuning, RateControl, VideoCodec, VideoEncoder, VideoSettings};
        let gpu = Gpu::new().unwrap();
        let info = HdrInfo {
            sdr_white_nits: 203.0,
            peak_nits: 1000.0,
        };
        let Ok(mut conv) = HdrConverter::new(&gpu, HdrOutput::Hdr10, info, (640, 360), false)
        else {
            eprintln!("no P010 rendering on this card");
            return;
        };
        let settings = VideoSettings {
            codec: VideoCodec::Hevc,
            width: 640,
            height: 360,
            fps: 30,
            rate: RateControl::Vbr {
                kbps: 4000,
                max_kbps: 6000,
            },
            preset: 4,
            keyframe_interval_secs: 1,
            tuning: EncoderTuning::default(),
            hdr10: true,
        };
        let Ok(mut encoder) = VideoEncoder::new(&gpu, settings) else {
            eprintln!("no hardware HEVC encoder here");
            return;
        };
        let src = source(&gpu, 203.0);
        let mut packets = Vec::new();
        for i in 0..30 {
            let picture = conv.convert(&src, (4, 1)).unwrap();
            encoder
                .encode_texture(picture, i, &mut |p| packets.push(p))
                .unwrap();
        }
        encoder.finish(&mut |p| packets.push(p)).unwrap();
        assert!(!packets.is_empty());

        let dir = crate::library::testutil::TempDir::new("hdr10");
        let path = dir.0.join("hdr10.mp4");
        let mut writer = crate::mux::Mp4Writer::create(&path, &[encoder.stream_info()]).unwrap();
        for p in &packets {
            writer.write(0, p, 0).unwrap();
        }
        writer.finish().unwrap();
        let input = ffmpeg_next::format::input(&path).unwrap();
        let stream = input
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .unwrap();
        let par = unsafe { &*stream.parameters().as_ptr() };
        eprintln!(
            "HDR10 file: format {:?}, primaries {:?}, transfer {:?}, profile {}",
            par.format, par.color_primaries, par.color_trc, par.profile
        );
        assert_eq!(
            par.color_trc,
            ffmpeg_next::ffi::AVColorTransferCharacteristic::AVCOL_TRC_SMPTE2084
        );
        assert_eq!(
            par.color_primaries,
            ffmpeg_next::ffi::AVColorPrimaries::AVCOL_PRI_BT2020
        );
        // HEVC Main 10 is profile 2.
        assert_eq!(par.profile, 2);
    }

    #[test]
    #[ignore = "prints the HDR state of this PC's screens"]
    fn screens() {
        for m in crate::capture::list_monitors() {
            println!(
                "{} {}x{}: {:?}",
                m.device,
                m.width,
                m.height,
                monitor_hdr(m.handle)
            );
        }
    }
}
