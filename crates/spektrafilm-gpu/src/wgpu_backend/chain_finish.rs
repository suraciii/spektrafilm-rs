//! Film-chain tail states over bounded device windows. The transport buffer
//! remains resident; each state applies the existing f32 kernel in place and
//! completes every window before the next photographic stage is encoded.
use super::chain_chunks::PointwisePass;
use super::grain::RasterStorage;
use super::*;

impl WgpuBackend {
    /// Apply prepared finishing in-place on bounded resident RGB windows.
    pub(super) fn chain_finishing(
        &self,
        image: &(impl RasterStorage + ?Sized),
        n_pixels: u32,
        params: &spektrafilm_backend::FinishingGpuParams,
    ) -> Result<PointwisePass, String> {
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct FinishingUniform {
            n_pixels: u32,
            expand_enabled: u32,
            black: f32,
            inverse_span: f32,
            tone: [f32; 4],
            color: [f32; 4],
            to_srgb: [[f32; 4]; 3],
            from_srgb: [[f32; 4]; 3],
            rgb_to_lms: [[f32; 4]; 3],
            lms_to_lab: [[f32; 4]; 3],
            lab_to_lms: [[f32; 4]; 3],
            lms_to_rgb: [[f32; 4]; 3],
        }
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("finishing_params"),
                contents: bytemuck::bytes_of(&FinishingUniform {
                    n_pixels,
                    expand_enabled: u32::from(params.expand_enabled),
                    black: params.black,
                    inverse_span: params.inverse_span,
                    tone: params.tone,
                    color: params.color,
                    to_srgb: pack_matrix_columns(&params.to_srgb),
                    from_srgb: pack_matrix_columns(&params.from_srgb),
                    rgb_to_lms: pack_matrix_columns(&params.rgb_to_lms),
                    lms_to_lab: pack_matrix_columns(&params.lms_to_lab),
                    lab_to_lms: pack_matrix_columns(&params.lab_to_lms),
                    lms_to_rgb: pack_matrix_columns(&params.lms_to_rgb),
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
            });
        let pipeline = self.cached_pointwise_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/finishing.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            &[(1, "data")],
        );
        let pages = image.pages();
        self.chain_pointwise(
            pipeline,
            &[&uniform, &pages],
            &[(1, super::page_views::Access::ReadWrite)],
            n_pixels,
            false,
            "finishing",
        )
    }

    /// Build native output encoding or decoding without binding the complete
    /// transport raster. Encode remains matrix + CCTF; decode remains CCTF only.
    pub(super) fn chain_output_transfer(
        &self,
        image: &(impl RasterStorage + ?Sized),
        n_pixels: u32,
        params: &spektrafilm_backend::OutputTransferGpuParams,
    ) -> Result<PointwisePass, String> {
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct TransferParams {
            n_pixels: u32,
            mode: u32,
            cctf_kind: u32,
            _pad: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }
        let columns = pack_matrix_columns(&params.matrix);
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cctf_transfer_params"),
                contents: bytemuck::bytes_of(&TransferParams {
                    n_pixels,
                    mode: u32::from(params.decode),
                    cctf_kind: params.cctf_kind,
                    _pad: 0,
                    col0: columns[0],
                    col1: columns[1],
                    col2: columns[2],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
            });
        let pipeline = self.cached_pointwise_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_transfer.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            &[(1, "data")],
        );
        let pages = image.pages();
        self.chain_pointwise(
            pipeline,
            &[&uniform, &pages],
            &[(1, super::page_views::Access::ReadWrite)],
            n_pixels,
            false,
            "output_transfer",
        )
    }

    /// Build the terminal CMYK appearance in its existing encoded/linear
    /// domain, retaining the same matrices, mix strength, and f32 shader.
    pub(super) fn chain_cmyk(
        &self,
        image: &(impl RasterStorage + ?Sized),
        n_pixels: u32,
        params: &spektrafilm_backend::CmykGpuParams,
    ) -> Result<PointwisePass, String> {
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct CmykParams {
            n_pixels: u32,
            cctf_kind: u32,
            encoded: u32,
            _pad0: u32,
            strength_pad: [f32; 4],
            to_col0: [f32; 4],
            to_col1: [f32; 4],
            to_col2: [f32; 4],
            from_col0: [f32; 4],
            from_col1: [f32; 4],
            from_col2: [f32; 4],
        }
        let to = pack_matrix_columns(&params.to_lab);
        let from = pack_matrix_columns(&params.from_lab);
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cmyk_params"),
                contents: bytemuck::bytes_of(&CmykParams {
                    n_pixels,
                    cctf_kind: params.cctf_kind,
                    encoded: u32::from(params.encoded),
                    _pad0: 0,
                    strength_pad: [params.strength, 0.0, 0.0, 0.0],
                    to_col0: to[0],
                    to_col1: to[1],
                    to_col2: to[2],
                    from_col0: from[0],
                    from_col1: from[1],
                    from_col2: from[2],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
            });
        let pipeline = self.cached_pointwise_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/cmyk_ink.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            &[(1, "data")],
        );
        let pages = image.pages();
        self.chain_pointwise(
            pipeline,
            &[&uniform, &pages],
            &[(1, super::page_views::Access::ReadWrite)],
            n_pixels,
            false,
            "cmyk",
        )
    }
}
