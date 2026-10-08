use super::*;
use super::blur::DispatchJob;

/// Pre-built output gamut compression pass — a single per-pixel dispatch
/// in place on `buf_b` (the scan output), between glare and unsharp,
/// matching the CPU scanning order. Owns the uniform + `C_max` table
/// buffers so they outlive the encoder.
///
/// CPU equivalent: `OutputGamutCompress::compress`.
#[cfg(feature = "wgpu-backend")]
pub(super) struct GamutState {
    _cmax_buf: wgpu::Buffer,
    dispatch: DispatchJob,
}

#[cfg(feature = "wgpu-backend")]
pub(super) fn build_gamut_state(
    device: &wgpu::Device,
    gp: &crate::GamutGpuParams<'_>,
    buf_b: &wgpu::Buffer,
    n_pixels: u32,
    backend: &WgpuBackend,
) -> GamutState {
    use wgpu::util::DeviceExt;
    let pipe = backend.cached_pipeline(
        include_str!("gamut_compress.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct GamutParams {
        n_pixels: u32,
        mode: u32,
        n_l: u32,
        n_h: u32,
        knee: [f32; 4],
        lightness: [f32; 4],
        lspace: [f32; 4],
    }
    let lightness = match gp.lightness {
        Some([t, l, p]) => [t, l, p, 1.0],
        None => [0.0, 1.0, 1.0, 0.0],
    };
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gamut_params"),
        contents: bytemuck::bytes_of(&GamutParams {
            n_pixels,
            mode: gp.mode,
            n_l: gp.n_l,
            n_h: gp.n_h,
            knee: [gp.knee[0], gp.knee[1], gp.knee[2], 0.0],
            lightness,
            lspace: [gp.l_white, gp.l_min, gp.l_max, 0.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    // aces_rgc carries no table — bind a 1-element placeholder (the shader
    // never reads it on that mode; wgpu requires a non-empty binding).
    let cmax_f32: Vec<f32> = if gp.cmax.is_empty() {
        vec![0.0]
    } else {
        gp.cmax.iter().map(|&v| v as f32).collect()
    };
    let cmax_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gamut_cmax"),
        contents: bytemuck::cast_slice(&cmax_f32),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gamut_bg"),
        layout: &pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: cmax_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_b.as_entire_binding(),
            },
        ],
    });
    GamutState {
        _cmax_buf: cmax_buf,
        dispatch: DispatchJob {
            _params_buf: params_buf,
            pipeline: pipe,
            bg,
        },
    }
}

#[cfg(feature = "wgpu-backend")]
impl GamutState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        n_pixels: u32,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gamut_compress"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.dispatch.pipeline.pipeline);
        pass.set_bind_group(0, &self.dispatch.bg, &[]);
        dispatch_linear(&mut pass, n_pixels);
    }
}
