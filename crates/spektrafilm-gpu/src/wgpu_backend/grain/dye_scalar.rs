//! Scalar-record coefficients and full-source device validation.
use super::*;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RecordParams {
    seed: [u32; 2],
    density_per_event: f32,
    expectation: u32,
}

pub(super) const PARAMS_BYTES: u64 = std::mem::size_of::<RecordParams>() as u64;

pub(super) fn coefficients(backend: &WgpuBackend, params: &DyeGpuParams, q: f64) -> wgpu::Buffer {
    backend.pipe.static_buffer(
        "dye_record_params",
        bytemuck::bytes_of(&RecordParams {
            seed: [params.seed as u32, (params.seed >> 32) as u32],
            density_per_event: q as f32 / (params.cell_um * params.cell_um) as f32,
            expectation: u32::from(params.expectation),
        }),
        wgpu::BufferUsages::UNIFORM,
    )
}

pub(super) fn validate_source(
    backend: &WgpuBackend,
    pixels: u32,
    input: &wgpu::Buffer,
    status: &wgpu::Buffer,
) -> AccumulatePass {
    let params = backend.pipe.static_buffer(
        "dye_record_validation_params",
        bytemuck::cast_slice(&[pixels, 0u32, 0, 0]),
        wgpu::BufferUsages::UNIFORM,
    );
    let pipeline = backend.cached_pipeline(
        include_str!("dye_scalar_validate.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let entries: Vec<_> = [&params, input, status]
        .into_iter()
        .enumerate()
        .map(|(i, buffer)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let group = backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_record_validate_source"),
            layout: &pipeline.layout,
            entries: &entries,
        });
    AccumulatePass {
        pipeline,
        group,
        cells: pixels,
    }
}

/// Bounded synthetic first-use check, independent of any request's image.
/// The count-law certification owns statistics; this check proves scalar
/// addressing, event conversion, one-lane formation, and RGB publication.
pub(super) fn certify(backend: &WgpuBackend) -> Result<(), String> {
    use super::super::dye_decisions::DyeDecisionCase;
    let mut p = DyeGpuParams {
        law: DyeFieldLaw::ScalarRecord {
            event_contribution_d_um2: 0.125,
        },
        cell_um: 1.0,
        taps: vec![0.25, 0.5, 0.25],
        source_pitch_um: 1.0,
        grid: DyeReadoutGrid {
            origin_um: [-2.0, -3.0],
            pitch_um: [1.0; 2],
            width: 2,
            height: 2,
        },
        seed: 0xfedc_ba98_7654_3210,
        channel_correlation: 0.0,
        expectation: true,
        monochrome: true,
    };
    let cases: Vec<_> = (0..4)
        .map(|i| DyeDecisionCase::new(10.0, p.seed, 0, (-2 + i % 2, -3 + i / 2)))
        .collect();
    let (counts, _) = backend.dye_decision_counts(&cases)?;
    let source = backend.pipe.static_buffer(
        "dye_record_check_source",
        bytemuck::cast_slice(&[1.25_f32; 12]),
        wgpu::BufferUsages::STORAGE,
    );
    let scratch = |label, size| {
        backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    };
    for expectation in [true, false] {
        p.expectation = expectation;
        if !expectation {
            p.taps = vec![1.0];
        }
        let plan = DyePlan::new(2, 2, &p, &backend.device.raw.limits())?;
        let [_, _, c, h, f] = plan.max_band_bytes;
        let cells = scratch("dye_record_check_cells", c);
        let horizontal = scratch("dye_record_check_horizontal", h);
        let formed = scratch("dye_record_check_formed", f);
        let output = scratch("dye_record_check_output", plan.max_output_bytes);
        let status = scratch("dye_record_check_status", 4);
        let record = coefficients(backend, &p, 0.125);
        let taps = dye_spatial::formation_taps(backend, &p);
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        validate_source(backend, 4, &source, &status).encode(&mut encoder);
        for band in &plan.bands {
            let spatial = dye_spatial::build_scalar_spatial(
                backend,
                &p,
                band,
                [2, 2],
                &source,
                &cells,
                &horizontal,
                &formed,
                &output,
                &record,
                &status,
                &taps,
            );
            spatial.encode_remap(&mut encoder);
            spatial.encode_formation(&mut encoder);
            dye_spatial::build_band_readout(
                backend,
                &p,
                band,
                [2, 2],
                &formed,
                &output,
                false,
                true,
            )
            .encode(&mut encoder);
        }
        backend.submit_commands(Some(encoder.finish()))?;
        backend.check_dye_status(&status)?;
        let actual = backend
            .pipe
            .readback_buffer_cancellable::<f32>(&output, 48)?;
        for (i, pixel) in actual.as_chunks::<3>().0.iter().enumerate() {
            let expected = if expectation {
                1.25
            } else {
                counts[i] as f32 * 0.125
            };
            if pixel
                .iter()
                .any(|&v| !v.is_finite() || (v - expected).abs() > 2e-6)
            {
                return Err("film grain Dye: device scalar-record self-check failed".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "dye_scalar_tests.rs"]
mod tests;
