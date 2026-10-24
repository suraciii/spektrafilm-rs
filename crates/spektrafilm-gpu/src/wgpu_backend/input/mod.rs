use super::grain::RasterStorage;
use super::*;

/// Pre-built input transfer decoding over bounded device pages, in place on
/// `buf_a` before the front transform.
///
/// CPU equivalent: the `input_cctf_decoding` branch of the resident route,
/// which decodes the input with the input colour space's CCTF before the
/// front pass. Owning the uniform here keeps it alive for the encoder.
pub(super) struct InputDecodeState {
    dispatch: super::chain_chunks::PointwisePass,
}

pub(super) fn build_input_decode_state(
    device: &ObservedDevice,
    params: &spektrafilm_backend::InputDecodeGpuParams,
    buf_a: &(impl RasterStorage + ?Sized),
    n_pixels: u32,
    backend: &WgpuBackend,
) -> Result<InputDecodeState, String> {
    let pipe = backend.cached_pipeline(
        concat!(
            include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
            include_str!("../../../../spektrafilm-shaders/wgsl/decode_input.wgsl")
        ),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct DecodeParams {
        n_pixels: u32,
        cctf_kind: u32,
    }
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("input_decode_params"),
        contents: bytemuck::bytes_of(&DecodeParams {
            n_pixels,
            cctf_kind: params.cctf_kind,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
    });
    let pages = buf_a.pages();
    Ok(InputDecodeState {
        dispatch: backend.chain_pointwise(
            pipe,
            &[&params_buf, &pages],
            &[(1, true)],
            n_pixels,
            false,
            "input_decode",
        )?,
    })
}

impl InputDecodeState {
    /// Submit bounded input windows before the chain's front transform.
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.dispatch.submit(backend)
    }
}

#[cfg(test)]
mod tests;
