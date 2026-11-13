//! Exposure Grain on the selected device.
use super::super::paged::ImagePage;
use super::raster::{RasterCommands, RasterStorage, paged_transport, transport, uniform};
use super::*;
use spektrafilm_backend::ExposureGrainGpuParams;
use spektrafilm_math::grain::exposure::{
    ExposureTables, PERMUTATION_LEN, SPECTRAL_LEN, exposure_coefficients,
};
use spektrafilm_math::precision::{from_f32, to_f32};

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderParams {
    dims: [u32; 4],
    coeff: [f32; 4],
    shape: [f32; 4],
    page: [u32; 4],
}

/// Conservative additional device capacity for one resident Exposure pass.
/// The output raster, three transport windows, the generated tables, and the
/// two per-page uniforms are all owner-local.
pub(in crate::wgpu_backend) fn exposure_workspace_bytes(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
) -> Result<u64, String> {
    let n = width
        .checked_mul(height)
        .ok_or("grain_exposure: dimensions overflow")?;
    if n == 0 {
        return Err("grain_exposure: dimensions overflow".into());
    }
    let limit = super::super::bounded::storage_binding_limit(backend)?;
    let page = (limit / 12).min(u64::from(n)).max(1);
    let pages = u64::from(n).div_ceil(page);
    let tables = (SPECTRAL_LEN * 3 + PERMUTATION_LEN * 2) as u64 * 4 + 5 * 4;
    let rasters = u64::from(n) * 12 + page * 12 * 3;
    let uniforms = pages * 256;
    rasters
        .checked_add(tables)
        .and_then(|total| total.checked_add(uniforms))
        .ok_or_else(|| "grain_exposure: workspace bytes overflow".into())
}

fn validate(params: &ExposureGrainGpuParams) -> Result<(), String> {
    if [
        params.strength,
        params.size,
        params.format_mm,
        params.absolute_size_px,
        params.roughness,
        params.shadows,
        params.midtones,
        params.highlights,
        params.push,
        params.render_scale,
    ]
    .iter()
    .any(|value| !value.is_finite())
        || params.format_mm <= 0.0
        || params.render_scale <= 0.0
    {
        return Err("grain_exposure: controls must be finite and size/format positive".into());
    }
    Ok(())
}

impl WgpuBackend {
    pub fn grain_exposure_gpu(
        &self,
        img: &ImageBuf,
        params: &ExposureGrainGpuParams,
    ) -> Result<ImageBuf, String> {
        if !self.device.context.enabled() || img.width == 0 || img.height == 0 {
            return self.grain_exposure_gpu_inner(img, params);
        }
        let (stage, _stage) = self.observed_stage(
            "grain_exposure",
            spektrafilm_backend::telemetry::GpuStage::GrainExposure,
        );
        let (backend, _batch) = stage.observed_batch("grain_exposure");
        backend.grain_exposure_gpu_inner(img, params)
    }

    fn grain_exposure_gpu_inner(
        &self,
        img: &ImageBuf,
        params: &ExposureGrainGpuParams,
    ) -> Result<ImageBuf, String> {
        if img.width == 0 || img.height == 0 || params.strength <= 0.0 {
            return Ok(img.clone());
        }
        validate(params)?;
        let coefficients = exposure_coefficients(
            params.auto_size,
            params.size,
            params.format_mm,
            params.absolute_size_px,
            params.strength,
            params.roughness,
            params.shadows,
            params.midtones,
            params.highlights,
            params.push,
            params.render_scale,
            img.width,
            img.height,
        );
        let tables =
            ExposureTables::new(params.seed, coefficients.size_low, coefficients.size_high);
        if tables.weights.len() != 5 {
            return Err("grain_exposure: expected five octave weights".into());
        }
        let input: Vec<f32> = img.data.iter().map(|value| to_f32(*value)).collect();
        let count = u64::from(img.width) * u64::from(img.height);
        if count > u64::from(u32::MAX) {
            return Err("grain_exposure: image exceeds device pixel indexing".into());
        }
        let page_pixels =
            (super::super::bounded::storage_binding_limit(self)? / 12).min(count) as usize;
        if page_pixels == 0 {
            return Err("grain_exposure: device storage limit cannot hold one pixel".into());
        }
        let t1 = GpuBuffer::storage_ro(bytemuck::cast_slice(&tables.t1));
        let t2 = GpuBuffer::storage_ro(bytemuck::cast_slice(&tables.t2));
        let t3 = GpuBuffer::storage_ro(bytemuck::cast_slice(&tables.t3));
        let perm = GpuBuffer::storage_ro(bytemuck::cast_slice(&tables.perm));
        let perm2 = GpuBuffer::storage_ro(bytemuck::cast_slice(&tables.perm2));
        let weights = GpuBuffer::storage_ro(bytemuck::cast_slice(&tables.weights));
        let mut output = Vec::with_capacity(input.len());
        for (page_index, source_slice) in input.chunks(page_pixels * 3).enumerate() {
            let n_pixels = (source_slice.len() / 3) as u32;
            let source = GpuBuffer::storage_ro(bytemuck::cast_slice(source_slice));
            let destination = GpuBuffer::storage_rw(&vec![0u8; n_pixels as usize * 12]);
            let uniform = ShaderParams {
                dims: [img.width, img.height, n_pixels, 0],
                coeff: [
                    coefficients.shadow_amplitude,
                    coefficients.midtone_amplitude,
                    coefficients.highlight_amplitude,
                    coefficients.frequency_gain,
                ],
                shape: [
                    coefficients.roughness_effective,
                    coefficients.size_low,
                    coefficients.size_high,
                    0.0,
                ],
                page: [(page_index * page_pixels) as u32, 0, 0, 0],
            };
            let params_binding = GpuBuffer::uniform(bytemuck::bytes_of(&uniform));
            let bindings = [
                params_binding,
                source,
                t1.clone(),
                t2.clone(),
                t3.clone(),
                perm.clone(),
                perm2.clone(),
                weights.clone(),
                destination,
            ];
            let result = self
                .dispatch_compute(
                    include_str!("exposure.wgsl"),
                    &bindings,
                    n_pixels,
                    8,
                    "grain_exposure",
                )?
                .ok_or_else(|| {
                    "grain_exposure: selected device could not dispatch the exposure grain pass"
                        .to_owned()
                })?;
            output.extend_from_slice(&result);
        }
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            output.into_iter().map(from_f32).collect(),
        ))
    }
}

/// Resident-chain Exposure state: encode the linear tail, run the table-backed
/// grain pass, and leave encoded samples in the output pages.
pub(in crate::wgpu_backend) struct ExposureGrainState {
    output: Vec<ImagePage>,
    commands: RasterCommands,
    n_bytes: u64,
}

impl ExposureGrainState {
    pub(in crate::wgpu_backend) fn submit(
        &self,
        backend: &WgpuBackend,
        output: &(impl RasterStorage + ?Sized),
    ) -> Result<(), String> {
        self.commands.submit(backend)?;
        let mut copies = RasterCommands::default();
        copies.copy(&self.output, 0, output, 0, self.n_bytes);
        copies.submit(backend)
    }
}

pub(in crate::wgpu_backend) fn build_grain_exposure_state(
    params: &ExposureGrainGpuParams,
    width: u32,
    height: u32,
    input: &(impl RasterStorage + ?Sized),
    output_space: &spektrafilm_math::colorspace::RgbColorSpace,
    backend: &WgpuBackend,
) -> Result<ExposureGrainState, String> {
    validate(params)?;
    let n = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("grain_exposure: dimensions overflow")?;
    let limit = super::super::bounded::storage_binding_limit(backend)?;
    let page = (limit / 12).min(u64::from(n)) as u32;
    if page == 0 {
        return Err("grain_exposure: device storage limit cannot hold one grain pixel".into());
    }
    let n_bytes = u64::from(n) * 12;
    let output = paged_transport(backend, n_bytes, "grain_exposure_output")?;
    let source_window = transport(backend, u64::from(page) * 12, "grain_exposure_source")?;
    let encoded_window = transport(backend, u64::from(page) * 12, "grain_exposure_encoded")?;
    let result_window = transport(backend, u64::from(page) * 12, "grain_exposure_result")?;
    let dummy_weights = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_exposure_dummy_weights"),
            contents: bytemuck::cast_slice(&[0.0f32, 0.0f32]),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let encode_pipeline = backend.cached_pipeline(
        include_str!("film_resolution.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let grain_pipeline = backend.cached_pipeline(
        include_str!("exposure.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let coefficients = exposure_coefficients(
        params.auto_size,
        params.size,
        params.format_mm,
        params.absolute_size_px,
        params.strength,
        params.roughness,
        params.shadows,
        params.midtones,
        params.highlights,
        params.push,
        params.render_scale,
        width,
        height,
    );
    let tables = ExposureTables::new(params.seed, coefficients.size_low, coefficients.size_high);
    if tables.weights.len() != 5 {
        return Err("grain_exposure: expected five octave weights".into());
    }
    let table_buffer = |label: &str, bytes: &[u8]| {
        backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage: wgpu::BufferUsages::STORAGE,
            })
    };
    let t1 = table_buffer("grain_exposure_t1", bytemuck::cast_slice(&tables.t1));
    let t2 = table_buffer("grain_exposure_t2", bytemuck::cast_slice(&tables.t2));
    let t3 = table_buffer("grain_exposure_t3", bytemuck::cast_slice(&tables.t3));
    let perm = table_buffer("grain_exposure_perm", bytemuck::cast_slice(&tables.perm));
    let perm2 = table_buffer("grain_exposure_perm2", bytemuck::cast_slice(&tables.perm2));
    let weights = table_buffer(
        "grain_exposure_weights",
        bytemuck::cast_slice(&tables.weights),
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct EncodeParams {
        dimensions: [u32; 4],
        taps: [u32; 4],
        matrix: [[f32; 4]; 3],
        window: [u32; 4],
    }
    let matrix = output_space
        .rgb_to_rgb_identity()
        .map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.0]);
    let mut commands = RasterCommands::default();
    for first in (0..n).step_by(page as usize) {
        let count = page.min(n - first);
        let source_uniform = uniform(
            backend,
            &EncodeParams {
                dimensions: [width, height, 2, 0],
                taps: [0, output_space.cctf.code(), 0, 0],
                matrix,
                window: [first, count, first, 0],
            },
        );
        commands.copy(
            input,
            u64::from(first) * 12,
            &source_window,
            0,
            u64::from(count) * 12,
        );
        commands.dispatch_named(
            &encode_pipeline,
            &[
                &source_uniform,
                &source_window,
                &dummy_weights,
                &encoded_window,
            ],
            count,
            false,
            backend,
            Some("grain_exposure_encode"),
        );
        let grain_uniform = uniform(
            backend,
            &ShaderParams {
                dims: [width, height, count, 0],
                coeff: [
                    coefficients.shadow_amplitude,
                    coefficients.midtone_amplitude,
                    coefficients.highlight_amplitude,
                    coefficients.frequency_gain,
                ],
                shape: [
                    coefficients.roughness_effective,
                    coefficients.size_low,
                    coefficients.size_high,
                    0.0,
                ],
                page: [first, 0, 0, 0],
            },
        );
        commands.dispatch_named(
            &grain_pipeline,
            &[
                &grain_uniform,
                &encoded_window,
                &t1,
                &t2,
                &t3,
                &perm,
                &perm2,
                &weights,
                &result_window,
            ],
            count,
            false,
            backend,
            Some("grain_exposure"),
        );
        commands.copy(
            &result_window,
            0,
            &output,
            u64::from(first) * 12,
            u64::from(count) * 12,
        );
    }
    Ok(ExposureGrainState {
        output,
        commands,
        n_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::from_f32;
    use spektrafilm_model::grain::exposure::{ExposureGrainParams, apply_cpu};

    #[test]
    fn device_matches_cpu_reference_on_a_bounded_raster() {
        let backend = crate::test_support::adapter();
        let image = ImageBuf::from_data(
            4,
            3,
            (0..3)
                .flat_map(|y| {
                    (0..4).flat_map(move |x| {
                        [
                            from_f32(0.1 + x as f32 * 0.2 + y as f32 * 0.02),
                            from_f32(0.2 + x as f32 * 0.1 + y as f32 * 0.05),
                            from_f32(0.3 + x as f32 * 0.05 + y as f32 * 0.1),
                        ]
                    })
                })
                .collect(),
        );
        let cpu_params = ExposureGrainParams {
            auto_size: false,
            absolute_size_px: 2.0,
            seed: 7,
            ..Default::default()
        };
        let gpu_params = ExposureGrainGpuParams {
            strength: cpu_params.strength,
            size: cpu_params.size,
            auto_size: cpu_params.auto_size,
            format_mm: cpu_params.format.short_edge_mm(),
            absolute_size_px: cpu_params.absolute_size_px,
            roughness: cpu_params.roughness,
            shadows: cpu_params.shadows,
            midtones: cpu_params.midtones,
            highlights: cpu_params.highlights,
            push: cpu_params.push,
            seed: cpu_params.seed,
            render_scale: cpu_params.render_scale,
        };
        let expected = apply_cpu(&image, cpu_params);
        let actual = backend.grain_exposure_gpu(&image, &gpu_params).unwrap();
        for (index, (&actual, &expected)) in actual.data.iter().zip(&expected.data).enumerate() {
            let actual = to_f32(actual);
            let expected = to_f32(expected);
            assert!(
                (actual - expected).abs() < 3e-5,
                "sample {index}: {actual} != {expected}"
            );
        }
    }

    #[test]
    fn device_is_deterministic_and_zero_strength_bypasses() {
        let backend = crate::test_support::adapter();
        let image = ImageBuf::from_data(5, 4, vec![from_f32(0.42); 5 * 4 * 3]);
        let params = ExposureGrainGpuParams {
            strength: 100.0,
            size: 1.0,
            auto_size: true,
            format_mm: 24.0,
            absolute_size_px: 1.5,
            roughness: 50.0,
            shadows: 50.0,
            midtones: 70.0,
            highlights: 30.0,
            push: 0.0,
            seed: 11,
            render_scale: 1.0,
        };
        let first = backend.grain_exposure_gpu(&image, &params).unwrap();
        let second = backend.grain_exposure_gpu(&image, &params).unwrap();
        assert_eq!(first.data, second.data);
        let bypass = ExposureGrainGpuParams {
            strength: 0.0,
            ..params
        };
        assert_eq!(
            backend.grain_exposure_gpu(&image, &bypass).unwrap().data,
            image.data
        );
    }
}
