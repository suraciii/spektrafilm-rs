use super::blur::{self, BlurJob, DispatchJob};
use super::*;

/// Pre-built unsharp mask pass — owns the blur and the combine dispatch.
///
/// CPU equivalent: `spektrafilm_model::optics::apply_unsharp_mask`.
/// The combine shader writes to a temp buffer (avoids same-pass
/// aliasing); `encode_passes` then `copy_buffer_to_buffer`s the result
/// back into buf_b so the rest of the pipeline doesn't need to know
/// about the temporary.
pub(super) struct UnsharpState {
    blur: BlurJob,
    combine: DispatchJob,
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
    out_buf: wgpu::Buffer, // combine target; copied back to buf_b at the end
}

pub(super) fn build_unsharp_state(
    device: &ObservedDevice,
    up: &crate::UnsharpGpuParams,
    width: u32,
    height: u32,
    buf_a: &wgpu::Buffer,
    buf_b: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> UnsharpState {
    let n_pixels = (width as usize) * (height as usize);
    let img_bytes = (n_pixels * 3 * 4) as u64;

    // Output buffer for the combine pass. Cleared each render — fine,
    // it's only used between the combine dispatch and the copy_buffer.
    let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("unsharp_out"),
        size: img_bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    // ── Blur (buf_b → buf_a (mid) → out_buf as the blurred destination
    // tmp). We re-use out_buf for two purposes: it first holds the
    // blurred image, then the combine overwrites it with the final
    // sharpened image.
    let blur_pipe_h = backend.cached_pipeline(
        include_str!("../blur/gaussian_blur_h.wgsl"),
        &blur::BLUR_LAYOUT,
    );
    let blur_pipe_v = backend.cached_pipeline(
        include_str!("../blur/gaussian_blur_v.wgsl"),
        &blur::BLUR_LAYOUT,
    );

    // Need a separate buffer for blur output since out_buf is the
    // combine destination. Use a small fresh allocation.
    let blur_dst = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("unsharp_blur_dst"),
        size: img_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let blur = blur::make_blur_job(
        backend,
        up.sigma_px.max(0.01),
        width,
        height,
        buf_b,
        buf_a,
        &blur_dst,
        "unsharp",
        &blur_pipe_h,
        &blur_pipe_v,
    );

    // ── Combine: out = (1+amount)*orig - amount*blurred
    let combine_pipe = backend.cached_pipeline(
        include_str!("unsharp_combine.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct CombineParams {
        n_pixels: u32,
        amount: f32,
        _pad0: u32,
        _pad1: u32,
    }
    let combine_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("unsharp_combine_params"),
        contents: bytemuck::bytes_of(&CombineParams {
            n_pixels: n_pixels as u32,
            amount: up.amount,
            _pad0: 0,
            _pad1: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let combine_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("unsharp_combine_bg"),
        layout: &combine_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: combine_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_b.as_entire_binding(),
            }, // a = original
            wgpu::BindGroupEntry {
                binding: 2,
                resource: blur_dst.as_entire_binding(),
            }, // b = blurred
            wgpu::BindGroupEntry {
                binding: 3,
                resource: out_buf.as_entire_binding(),
            }, // out = sharpened
        ],
    });
    let combine = DispatchJob {
        _params_buf: combine_params,
        pipeline: combine_pipe,
        bg: combine_bg,
    };

    // Drop blur_dst's strong ref into UnsharpState via an owned vec.
    // We need to keep it alive for the encoder lifetime; tucking it
    // into the BlurJob is awkward, so use a separate field.
    let mut state = UnsharpState {
        blur,
        combine,
        blur_pipe_h,
        blur_pipe_v,
        out_buf,
    };
    // Leak the blur output into the combine's owned buffers list by
    // creating it inside the state; here we tuck it into a global
    // owner. Simpler: keep it alive via a sidecar field.
    // (Reusing _params_buf would be confusing; just attach as another
    // implicit slot.)
    let _ = blur_dst; // moved into bind groups, lives via wgpu's Arc
    // wgpu::Buffer is Arc-backed under the hood — the bind groups keep
    // it alive. Nothing to leak here.
    let _ = &mut state;
    state
}

impl UnsharpState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut ObservedEncoder,
        n_pixels: u32,
        wg_xy: (u32, u32),
        buf_b: &wgpu::Buffer,
        img_bytes: u64,
    ) {
        // Blur H + V.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("unsharp_blur_h"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blur_pipe_h.pipeline);
            pass.set_bind_group(0, &self.blur.bg_h, &[]);
            pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("unsharp_blur_v"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blur_pipe_v.pipeline);
            pass.set_bind_group(0, &self.blur.bg_v, &[]);
            pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
        }
        // Combine.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("unsharp_combine"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.combine.pipeline.pipeline);
            pass.set_bind_group(0, &self.combine.bg, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        // Copy sharpened result back into buf_b so the downstream
        // readback sees it without needing to know we used a temp.
        encoder.copy_buffer_to_buffer(&self.out_buf, 0, buf_b, 0, img_bytes);
    }
}

