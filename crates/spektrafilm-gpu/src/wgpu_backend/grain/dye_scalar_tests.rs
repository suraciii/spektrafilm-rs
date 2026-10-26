use super::*;
use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};
use spektrafilm_backend::DyeResponseGpuParams;
use spektrafilm_math::precision::{from_f64, to_f64};

fn params(width: u32, height: u32) -> DyeGpuParams {
    DyeGpuParams {
        law: DyeFieldLaw::ScalarRecord {
            event_contribution_d_um2: 0.125,
        },
        cell_um: 0.75,
        taps: vec![0.125, 0.25, 0.25, 0.25, 0.125],
        source_pitch_um: 1.25,
        grid: DyeReadoutGrid {
            origin_um: [-0.375, -0.625],
            pitch_um: [1.125, 0.875],
            width,
            height,
        },
        seed: 0x1234_5678_9abc_def0,
        channel_correlation: 0.45,
        expectation: true,
        monochrome: true,
    }
}

fn source() -> ImageBuf {
    ImageBuf::from_data(
        7,
        5,
        (0..35)
            .flat_map(|i| [from_f64(0.03 + f64::from((i * 17 + i / 7) % 23) * 0.11); 3])
            .collect(),
    )
}

fn overlap(a: f64, b: f64, c: f64, d: f64) -> f64 {
    (b.min(d) - a.max(c)).max(0.0)
}

// Independent f64 rectangle oracle: no device geometry, remap, or optical helper.
fn cell_mean(image: &ImageBuf, p: &DyeGpuParams, i: i32, j: i32) -> f64 {
    let h = p.cell_um;
    let pitch = p.source_pitch_um;
    let x0 = f64::from(i) * h;
    let y0 = f64::from(j) * h;
    let mut sum = 0.0;
    for y in (y0 / pitch).floor() as i32..((y0 + h) / pitch).ceil() as i32 {
        for x in (x0 / pitch).floor() as i32..((x0 + h) / pitch).ceil() as i32 {
            let sx = x.clamp(0, image.width as i32 - 1) as usize;
            let sy = y.clamp(0, image.height as i32 - 1) as usize;
            let weight = overlap(x0, x0 + h, f64::from(x) * pitch, f64::from(x + 1) * pitch)
                * overlap(y0, y0 + h, f64::from(y) * pitch, f64::from(y + 1) * pitch)
                / (h * h);
            sum += weight * to_f64(image.data[(sy * image.width as usize + sx) * 3]);
        }
    }
    sum
}

fn formed(image: &ImageBuf, p: &DyeGpuParams, i: i32, j: i32) -> f64 {
    let radius = (p.taps.len() / 2) as i32;
    let mut density = 0.0;
    for (ky, &wy) in p.taps.iter().enumerate() {
        for (kx, &wx) in p.taps.iter().enumerate() {
            density +=
                wy * wx * cell_mean(image, p, i + radius - kx as i32, j + radius - ky as i32);
        }
    }
    density
}

fn oracle(image: &ImageBuf, p: &DyeGpuParams, optical: impl Fn(f64) -> [f64; 3]) -> Vec<f64> {
    let mut result = Vec::new();
    for y in 0..p.grid.height {
        for x in 0..p.grid.width {
            let x0 = p.grid.origin_um[0] + f64::from(x) * p.grid.pitch_um[0];
            let y0 = p.grid.origin_um[1] + f64::from(y) * p.grid.pitch_um[1];
            let x1 = x0 + p.grid.pitch_um[0];
            let y1 = y0 + p.grid.pitch_um[1];
            let mut sum = [0.0; 3];
            for j in (y0 / p.cell_um).floor() as i32..(y1 / p.cell_um).ceil() as i32 {
                for i in (x0 / p.cell_um).floor() as i32..(x1 / p.cell_um).ceil() as i32 {
                    let weight = overlap(
                        x0,
                        x1,
                        f64::from(i) * p.cell_um,
                        f64::from(i + 1) * p.cell_um,
                    ) * overlap(
                        y0,
                        y1,
                        f64::from(j) * p.cell_um,
                        f64::from(j + 1) * p.cell_um,
                    ) / (p.grid.pitch_um[0] * p.grid.pitch_um[1]);
                    let linear = optical(formed(image, p, i, j));
                    for c in 0..3 {
                        sum[c] += weight * linear[c];
                    }
                }
            }
            result.extend(sum);
        }
    }
    result
}

fn assert_close(actual: &[f32], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&a, &e)) in actual.iter().zip(expected).enumerate() {
        assert!(
            a.is_finite() && (f64::from(a) - e).abs() <= tolerance * (1.0 + e.abs()),
            "sample {index}: {a} != {e}, budget {tolerance}"
        );
    }
}

fn paged_input(backend: &WgpuBackend, image: &ImageBuf, page_pixels: usize) -> BufferPayload {
    let pages = image
        .data
        .chunks(page_pixels * 3)
        .enumerate()
        .map(|(i, samples)| {
            let values = scalars_to_f32(samples);
            ImagePage {
                first_pixel: (i * page_pixels) as u32,
                n_pixels: (samples.len() / 3) as u32,
                buffer: backend
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("scalar_record_test_source_page"),
                        contents: bytemuck::cast_slice(&values),
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_SRC
                            | wgpu::BufferUsages::COPY_DST,
                    }),
            }
        })
        .collect();
    BufferPayload::from_pages(backend.backend_id(), pages, image.width, image.height)
}

fn paged_output(
    backend: &WgpuBackend,
    image: &ImageBuf,
    p: &DyeGpuParams,
    response: Option<&DyeResponseGpuParams<'_>>,
    page_pixels: usize,
) -> Vec<f32> {
    let input = paged_input(backend, image, page_pixels);
    let state = backend
        .prepare_dye_paged(image.width, image.height, &input, p, response)
        .unwrap();
    state.submit(backend).unwrap();
    backend.check_dye_status(&state.status_buffer()).unwrap();
    state
        .output_pages
        .iter()
        .flat_map(|page| {
            backend
                .pipe
                .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)
        })
        .collect()
}

#[test]
fn scalar_expectation_matches_rectangle_oracle_across_pages_bands_and_fractional_edges() {
    let backend = crate::test_support::adapter();
    let image = source();
    let p = params(9, 133); // crosses the production 128-output-row boundary
    let actual = paged_output(&backend, &image, &p, None, 11); // pages split raster rows
    let expected = oracle(&image, &p, |d| [d; 3]);
    assert_close(&actual, &expected, 3e-6);
    assert!(
        actual
            .as_chunks::<3>()
            .0
            .iter()
            .all(|v| v[0] == v[1] && v[0] == v[2])
    );
    let contiguous = paged_output(&backend, &image, &p, None, 35);
    assert_eq!(actual, contiguous);
}

#[test]
fn scalar_stochastic_crop_keeps_global_cell_streams_and_correlation_is_unused() {
    let backend = crate::test_support::adapter();
    let image = source();
    let mut p = params(9, 133);
    p.expectation = false;
    let full = paged_output(&backend, &image, &p, None, 11);
    p.grid.origin_um[0] += 3.0 * p.grid.pitch_um[0];
    p.grid.origin_um[1] += 127.0 * p.grid.pitch_um[1];
    p.grid.width = 4;
    p.grid.height = 6;
    p.channel_correlation = f32::NAN; // scalar law has no channel-correlation parameter
    let crop = paged_output(&backend, &image, &p, None, 13);
    for y in 0..6usize {
        assert_eq!(
            &crop[y * 12..(y + 1) * 12],
            &full[((y + 127) * 9 + 3) * 3..((y + 127) * 9 + 7) * 3]
        );
    }
}

#[test]
fn scalar_scan_integrates_optics_before_footprints_and_falls_back_for_nonconstant_basis() {
    let backend = crate::test_support::adapter();
    let image = source();
    let mut p = params(5, 4);
    p.grid.pitch_um = [2.125, 1.875];
    let mut cd = [[0.83, 0.0, 0.0]; 6];
    cd[1] = [0.83, f64::INFINITY, f64::NAN]; // the whole wavelength is missing
    let base = [0.1, 0.2, 0.5, f64::NAN, 0.3, 0.07];
    let lamp = [70.0, 110.0, 90.0, 130.0, f64::NAN, 60.0];
    let normalization = 0.37;
    let matrix = [[1.2, -0.1, 0.03], [0.04, 0.9, -0.02], [-0.01, 0.03, 1.1]];
    for generic in [false, true] {
        if generic {
            cd[5][0] = 1.17;
        }
        let response = DyeResponseGpuParams::Scan {
            channel_density: &cd,
            base_density: &base,
            illuminant: &lamp,
            normalization,
            xyz_to_rgb: &matrix,
        };
        let setup = optical::ResponseSetup::new(&backend, &p, &response).unwrap();
        assert_eq!(setup.uses_scalar_scan(), !generic);
        let actual = paged_output(&backend, &image, &p, Some(&response), 11);
        let response_at = |d: f64| {
            let mut xyz = [0.0; 3];
            for wl in 0..6 {
                let light = 10.0f64.powf(-(d * cd[wl][0] + base[wl])) * lamp[wl];
                if cd[wl].iter().any(|v| v.is_nan()) || light.is_nan() {
                    continue;
                }
                xyz[0] += light * f64::from(spektrafilm_math::spectral::CMF_X[wl]);
                xyz[1] += light * f64::from(spektrafilm_math::spectral::CMF_Y[wl]);
                xyz[2] += light * f64::from(spektrafilm_math::spectral::CMF_Z[wl]);
            }
            std::array::from_fn(|c| (0..3).map(|j| matrix[c][j] * xyz[j] / normalization).sum())
        };
        let expected = oracle(&image, &p, response_at);
        assert_close(&actual, &expected, 3e-6);
        let wrong = oracle(&image, &p, |d| [d; 3])
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|density| response_at(density[0]))
            .collect::<Vec<_>>();
        assert!(
            expected
                .iter()
                .zip(wrong)
                .any(|(a, b)| (a - b).abs() > 1e-5),
            "fixture must reject density-then-optics ordering"
        );
    }
}

#[test]
fn scalar_full_source_validation_and_device_rate_limits_do_not_use_capacity_or_floor() {
    let backend = crate::test_support::adapter();
    let mut p = params(1, 1);
    p.cell_um = 1.0;
    p.source_pitch_um = 1.0;
    p.grid.origin_um = [0.0; 2];
    p.grid.pitch_um = [1.0; 2];
    p.taps = vec![1.0];
    let image = ImageBuf::from_data(64, 3, vec![from_f64(0.2); 64 * 3 * 3]);
    let input = paged_input(&backend, &image, 11);
    let last = input.pages.last().unwrap();
    let offset = u64::from(last.n_pixels - 1) * 12;
    for invalid in [
        [-0.1, -0.1, -0.1],
        [f32::NAN; 3],
        [f32::INFINITY; 3],
        [0.2, 0.2 + 2e-7, 0.2],
        [0.0, f32::from_bits(1), 0.0],
    ] {
        backend
            .queue
            .write_buffer(&last.buffer, offset, bytemuck::cast_slice(&invalid));
        let state = backend
            .prepare_dye_paged(image.width, image.height, &input, &p, None)
            .unwrap();
        state.submit(&backend).unwrap();
        let error = backend
            .check_dye_status(&state.status_buffer())
            .unwrap_err();
        assert!(
            error.contains("finite, nonnegative, and identical"),
            "{error}"
        );
    }
    backend.queue.write_buffer(
        &last.buffer,
        offset,
        bytemuck::cast_slice(&[0.0_f32, -0.0, 0.0]),
    );
    let state = backend
        .prepare_dye_paged(image.width, image.height, &input, &p, None)
        .unwrap();
    state.submit(&backend).unwrap();
    backend.check_dye_status(&state.status_buffer()).unwrap();
    backend.queue.write_buffer(
        &last.buffer,
        offset,
        bytemuck::cast_slice(&[f32::from_bits(1); 3]),
    );
    let state = backend
        .prepare_dye_paged(image.width, image.height, &input, &p, None)
        .unwrap();
    state.submit(&backend).unwrap();
    assert!(
        backend
            .check_dye_status(&state.status_buffer())
            .unwrap_err()
            .contains("rate")
    );
    for q in [1e-30, 1.0 / 65537.0] {
        p.law = DyeFieldLaw::ScalarRecord {
            event_contribution_d_um2: q,
        };
        let dense = ImageBuf::from_data(1, 1, vec![from_f64(1.0); 3]);
        for expectation in [false, true] {
            p.expectation = expectation;
            let error = backend.dye_field(&dense, &p).unwrap_err();
            assert!(error.contains("rate"), "{error}");
        }
    }
    for q in [0.0, f64::NAN, f64::INFINITY, 1e-40, 1e-300] {
        p.law = DyeFieldLaw::ScalarRecord {
            event_contribution_d_um2: q,
        };
        assert!(backend.validate_dye_resources(1, 1, &p).is_err());
    }
    p.law = DyeFieldLaw::ScalarRecord {
        event_contribution_d_um2: 0.125,
    };
    let clear = ImageBuf::from_data(1, 1, vec![from_f64(0.0); 3]);
    assert!(
        backend
            .dye_field(&clear, &p)
            .unwrap()
            .data
            .iter()
            .all(|&v| v == from_f64(0.0))
    );
    let high = ImageBuf::from_data(1, 1, vec![from_f64(100.0); 3]);
    p.expectation = true;
    assert_close(
        &scalars_to_f32(&backend.dye_field(&high, &p).unwrap().data),
        &[100.0; 3],
        1e-6,
    );
}

#[test]
fn scalar_population_obeys_certified_poisson_mean_variance_and_zero_mass() {
    let backend = crate::test_support::adapter();
    // First field preparation certifies dye_count.wgsl on this adapter.
    // Existing wrong-law controls and this population use that same source.
    let mut p = params(64, 64);
    p.cell_um = 1.0;
    p.source_pitch_um = 1.0;
    p.grid.origin_um = [0.0; 2];
    p.grid.pitch_um = [1.0; 2];
    p.taps = vec![1.0];
    p.expectation = false;
    let image = ImageBuf::from_data(64, 64, vec![from_f64(1.0); 64 * 64 * 3]);
    for rate in [0.03_f64, 29.999, 30.0, 160.0, 65536.0] {
        let q = 1.0 / rate;
        p.law = DyeFieldLaw::ScalarRecord {
            event_contribution_d_um2: q,
        };
        let actual = backend.dye_field(&image, &p).unwrap();
        let counts = actual
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|v| to_f64(v[0]) / f64::from(q as f32))
            .collect::<Vec<_>>();
        let n = counts.len() as f64;
        let mean = counts.iter().sum::<f64>() / n;
        let variance = counts.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let zero = counts.iter().filter(|&&v| v == 0.0).count() as f64 / n;
        let p0 = (-rate).exp();
        assert!(
            (mean - rate).abs() < 7.0 * (rate / n).sqrt() + 0.002,
            "rate {rate}, mean {mean}"
        );
        assert!(
            (variance - rate).abs() < 7.0 * ((rate + 2.0 * rate * rate) / n).sqrt() + 0.002,
            "rate {rate}, variance {variance}"
        );
        assert!(
            (zero - p0).abs() <= 7.0 * (p0 * (1.0 - p0) / n).sqrt() + 1.0 / n,
            "rate {rate}, zero mass {zero}"
        );
    }
    // No positive-density statistical floor: essentially certain zero counts
    // remain zero rather than receiving legacy capacity/background density.
    p.law = DyeFieldLaw::ScalarRecord {
        event_contribution_d_um2: 0.125,
    };
    let near_clear = ImageBuf::from_data(64, 64, vec![from_f64(1e-12); 64 * 64 * 3]);
    let actual = backend.dye_field(&near_clear, &p).unwrap();
    assert!(actual.data.iter().all(|&v| v == from_f64(0.0)));
}

#[test]
fn scalar_scan_extreme_exponents_keep_generic_device_overflow_and_underflow() {
    let backend = crate::test_support::adapter();
    let mut p = params(1, 1);
    p.cell_um = 1.0;
    p.source_pitch_um = 1.0;
    p.grid.origin_um = [0.0; 2];
    p.grid.pitch_um = [1.0; 2];
    p.taps = vec![1.0];
    let image = ImageBuf::from_data(1, 1, vec![from_f64(20.0); 3]);
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for overflow in [false, true] {
        let cd = [[if overflow { -1.0 } else { 1.0 }, 0.0, 0.0]];
        let base = [if overflow { -30.0 } else { 30.0 }];
        let lamp = [if overflow { 1e-30 } else { 1e30 }];
        let response = DyeResponseGpuParams::Scan {
            channel_density: &cd,
            base_density: &base,
            illuminant: &lamp,
            normalization: 1.0,
            xyz_to_rgb: &identity,
        };
        let setup = optical::ResponseSetup::new(&backend, &p, &response).unwrap();
        assert!(
            setup.uses_scalar_scan(),
            "fixture must reach a guarded scalar setup"
        );
        let result = backend.dye_field_with_response(&image, &p, &response);
        if overflow {
            assert!(result.unwrap_err().contains("nonfinite linear"));
        } else {
            assert!(result.unwrap().data.iter().all(|&v| v == from_f64(0.0)));
        }
    }
    let response = DyeResponseGpuParams::Scan {
        channel_density: &[[1.0, 0.0, 0.0]],
        base_density: &[],
        illuminant: &[1e-100],
        normalization: 1.0,
        xyz_to_rgb: &identity,
    };
    let actual = backend
        .dye_field_with_response(&image, &p, &response)
        .unwrap();
    assert!(actual.data.iter().all(|&v| v == from_f64(0.0)));
    let dense = ImageBuf::from_data(1, 1, vec![from_f64(100.0); 3]);
    let response = DyeResponseGpuParams::Scan {
        channel_density: &[[1.0, 0.0, 0.0]],
        base_density: &[-40.0],
        illuminant: &[1.0],
        normalization: 1.0,
        xyz_to_rgb: &identity,
    };
    let actual = backend
        .dye_field_with_response(&dense, &p, &response)
        .unwrap();
    assert!(actual.data.iter().all(|&v| v == from_f64(0.0)));
}

#[test]
fn scalar_large_footprint_cell_partitions_reuse_only_scalar_scratch() {
    let backend = crate::test_support::adapter();
    let image = source();
    let input = paged_input(&backend, &image, 11);
    let mut p = params(1, 1);
    p.grid.pitch_um = [18.0, 15.0];
    let limits = wgpu::Limits {
        max_buffer_size: 4096,
        max_storage_buffer_binding_size: 4096,
        ..Default::default()
    };
    let plan = DyePlan::new(image.width, image.height, &p, &limits).unwrap();
    assert!(plan.bands.len() > 1);
    assert_eq!(
        plan.max_band_bytes[1], 0,
        "scalar records must not plan cell layer targets"
    );
    assert!(plan.max_band_bytes[0] <= u64::from(image.width) * u64::from(image.height) * 12);
    assert!(plan.max_band_bytes.into_iter().all(|bytes| bytes <= 4096));
    assert_eq!(
        plan.max_band_bytes[2],
        plan.bands
            .iter()
            .map(|b| u64::from(b.ext_cols) * u64::from(b.ext_rows) * 4)
            .max()
            .unwrap()
    );
    assert_eq!(
        plan.max_band_bytes[3],
        plan.bands
            .iter()
            .map(|b| u64::from(b.cols) * u64::from(b.ext_rows) * 4)
            .max()
            .unwrap()
    );
    assert_eq!(
        plan.max_band_bytes[4],
        plan.bands
            .iter()
            .map(|b| u64::from(b.cols) * u64::from(b.rows) * 4)
            .max()
            .unwrap()
    );
    let scratch = |label, size| {
        backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let [source_bytes, _, cell_bytes, horizontal_bytes, formed_bytes] = plan.max_band_bytes;
    let window = scratch("scalar_partition_source", source_bytes);
    let cells = scratch("scalar_partition_cells", cell_bytes);
    let horizontal = scratch("scalar_partition_horizontal", horizontal_bytes);
    let formed = scratch("scalar_partition_formed", formed_bytes);
    let output = scratch("scalar_partition_output", plan.max_output_bytes);
    let status = scratch("scalar_partition_status", 4);
    let taps = dye_spatial::formation_taps(&backend, &p);
    for expectation in [false, true] {
        p.expectation = expectation;
        let record = coefficients(&backend, &p, 0.125);
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&status, 0, None);
        for band in &plan.bands {
            backend
                .copy_window_to(
                    &mut encoder,
                    &input,
                    band.source_x,
                    band.source_y,
                    band.source_cols,
                    band.source_rows,
                    &window,
                )
                .unwrap();
            let spatial = dye_spatial::build_scalar_spatial(
                &backend,
                &p,
                band,
                [image.width, image.height],
                &window,
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
            let readout = dye_spatial::build_band_readout(
                &backend,
                &p,
                band,
                [image.width, image.height],
                &formed,
                &output,
                false,
                true,
            );
            readout.encode(&mut encoder);
        }
        backend.submit_commands(Some(encoder.finish())).unwrap();
        backend.check_dye_status(&status).unwrap();
        let partitioned = backend.pipe.readback_buffer(&output, 12);
        let default = backend.dye_field(&image, &p).unwrap();
        let expected = default.data.iter().map(|&v| to_f64(v)).collect::<Vec<_>>();
        assert_close(&partitioned, &expected, 3e-6);
    }
}
