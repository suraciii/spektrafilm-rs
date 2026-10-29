use super::*;

fn parameters() -> ScannerDensityGpuParams {
    let pivot = (0.16_f64 / 0.184).log10();
    let gamma = 0.55;
    ScannerDensityGpuParams {
        density: vec![0.0, 4.0, 0.0, 4.4, 0.0, 4.8],
        exposure: vec![-4.0, 4.0, -4.0, 4.0, -4.0, 4.0],
        offsets: [0, 2, 4],
        counts: [2; 3],
        contrast: [0.73551400058216643; 3],
        dmax: [1.5625841465572565; 3],
        brightness_ev: [0.8781766618; 3],
        denominator: [-(-std::f64::consts::LN_10 * 1.5625841465572565).exp_m1(); 3],
        camera_ev: 8.0,
        lens_blur: 0.0,
        standard_setup: true,
        setup_aim_density: (pivot + 4.0) * gamma,
        setup_gamma: gamma,
        operator_pivot: pivot,
        operator_contrast_delta: 0.23,
        operator_exposure_ev: -0.31,
        operator_color: [0.12, -0.04, -0.08],
        tone_to_output: [[1.1, -0.1, 0.0], [-0.03, 1.03, 0.0], [0.0, -0.04, 1.04]],
    }
}

fn reference(data: &[f32], p: &ScannerDensityGpuParams) -> (Vec<f32>, [f64; 3], bool) {
    let slopes = [0.5, 0.55, 0.6];
    let mut ns = Vec::new();
    let mut sum = 0.0;
    let mut cast = [0.0; 2];
    for d in data.as_chunks::<3>().0 {
        let d = [f64::from(d[0]), f64::from(d[1]), f64::from(d[2])];
        let x: [f64; 3] = std::array::from_fn(|c| d[c] / slopes[c] - 4.0);
        let mean = x.iter().sum::<f64>() / 3.0;
        let r: [f64; 3] = std::array::from_fn(|c| d[c] - (mean + 4.0) * slopes[c]);
        cast[0] += (r[0] - r[1]) / 2.0_f64.sqrt();
        cast[1] += (r[0] + r[1] - 2.0 * r[2]) / 6.0_f64.sqrt();
        let n = d.iter().sum::<f64>() / 3.0;
        sum += n;
        ns.push(n);
    }
    ns.sort_by(f64::total_cmp);
    let position = (ns.len() - 1) as f64 * 0.05;
    let lo = position.floor() as usize;
    let n5 = ns[lo] + (position - lo as f64) * (ns[(lo + 1).min(ns.len() - 1)] - ns[lo]);
    let high = ns[ns.len() - 1] - n5 >= 1.8;
    let w = if high {
        [0.30, 0.35, 0.29]
    } else {
        [0.33, 0.30, 0.35]
    };
    let en = (w[0] * (sum / ns.len() as f64) + w[1] * ns[ns.len() - 1] + w[2] * ns[0])
        / w.iter().sum::<f64>();
    let dx = 0.75 * (p.setup_aim_density - en) / p.setup_gamma;
    let cast = cast.map(|v| v / ns.len() as f64);
    let weight = (1.0 - 40.0 * (cast[0] * cast[0] + cast[1] * cast[1])).max(0.2);
    let [u, v] = cast.map(|v| -0.9 * weight * v / p.setup_gamma);
    let offset = [
        dx + u / 2.0_f64.sqrt() + v / 6.0_f64.sqrt(),
        dx - u / 2.0_f64.sqrt() + v / 6.0_f64.sqrt(),
        dx - 2.0 * v / 6.0_f64.sqrt(),
    ];
    let mut output = Vec::new();
    for d in data.as_chunks::<3>().0 {
        let stops: [f64; 3] = std::array::from_fn(|c| {
            (f64::from(d[c]) / slopes[c] - 4.0 + offset[c] - p.operator_pivot)
                / std::f64::consts::LOG10_2
        });
        let common =
            p.operator_contrast_delta * (stops.iter().sum::<f64>() / 3.0) + p.operator_exposure_ev;
        let rgb: [f64; 3] = std::array::from_fn(|c| {
            let x = p.operator_pivot
                + std::f64::consts::LOG10_2 * (stops[c] + common + p.operator_color[c]);
            let z = p.contrast[c] * (x / std::f64::consts::LOG10_2 + p.brightness_ev[c]);
            let t = if z >= 0.0 {
                1.0 / (1.0 + (-z).exp())
            } else {
                let e = z.exp();
                e / (1.0 + e)
            };
            let density = p.dmax[c] * (1.0 - t);
            ((-std::f64::consts::LN_10 * density).exp_m1() + p.denominator[c]) / p.denominator[c]
        });
        for row in p.tone_to_output {
            output.push((row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]) as f32);
        }
    }
    (output, offset, high)
}

#[test]
fn standard_fullframe_exact_percentile_neutral_locus_and_operator_destination() {
    let gpu = crate::test_support::adapter();
    let p = parameters();
    for (maximum, expected_high) in [(1.9, false), (2.0, true)] {
        // 42 samples: the fifth percentile interpolates ranks 2 and 3 (2.05).
        // For maximum=1.9 the minimum-only contrast test would choose wrongly.
        for colored in [false, true] {
            let data: Vec<f32> = (0..42)
                .flat_map(|i| {
                    let n = match i {
                        0 => 0.0,
                        1 => 0.05,
                        2 => 0.1,
                        3 => 0.3,
                        _ => maximum,
                    };
                    [0.5, 0.55, 0.6].map(|s| ((n / 0.55) * s) as f32)
                })
                .enumerate()
                .map(|(i, v)| {
                    if colored {
                        v + [-0.07, 0.12, 0.02][i % 3]
                    } else {
                        v
                    }
                })
                .collect();
            let (expected, offset, high) = reference(&data, &p);
            if !colored {
                assert_eq!(high, expected_high);
                assert!(
                    (offset[0] - offset[2]).abs() < 1e-6,
                    "neutral ramp acquired a cast"
                );
            }
            // Deliberately tiny physical source pages split percentile neighbours.
            let source: Vec<_> = data
                .chunks(9)
                .enumerate()
                .map(|(i, values)| super::super::paged::ImagePage {
                    first_pixel: (i * 3) as u32,
                    n_pixels: (values.len() / 3) as u32,
                    buffer: gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("scan_test_source"),
                            contents: bytemuck::cast_slice(values),
                            usage: wgpu::BufferUsages::STORAGE
                                | wgpu::BufferUsages::COPY_SRC
                                | wgpu::BufferUsages::COPY_DST,
                        }),
                })
                .collect();
            let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("scan_test_output"),
                size: data.len() as u64 * 4,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let state = gpu
                .build_scan_density_state(&p, &source, &output, 42, 1, None)
                .unwrap();
            state.submit_optics(&gpu).unwrap();
            state.submit_setup(&gpu).unwrap();
            let setup = gpu
                .pipe
                .readback_buffer_as::<f32>(&state.prepared.setup, 64);
            let offset_error = (0..3)
                .map(|c| (f64::from(setup[5 + c]) - offset[c]).abs())
                .fold(0.0_f64, f64::max);
            // 1e-5 log10 exposure is 3.33e-5 EV; this leaves margin for
            // binary32 calibrated interpolation and whole-frame reduction.
            assert!(offset_error < 1e-5, "setup offset error {offset_error}");
            eprintln!(
                "scan setup maximum error: {offset_error} log10 exposure ({} EV)",
                offset_error / std::f64::consts::LOG10_2
            );
            state.submit_inverse(&gpu).unwrap();
            state.check(&gpu, &p, None, 42, 1).unwrap();
            let actual = gpu
                .pipe
                .readback_buffer_as::<f32>(&output, data.len() as u64 * 4);
            let tone_error = actual
                .iter()
                .zip(&expected)
                .map(|(&a, &b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            eprintln!("scan tone maximum absolute error: {tone_error}");
            for (i, (&a, &b)) in actual.iter().zip(&expected).enumerate() {
                assert!((a - b).abs() < 1e-5, "sample {i}: GPU {a}, CPU {b}");
            }
        }
    }
}

#[test]
fn standard_setup_large_frame_error_budget() {
    let gpu = crate::test_support::adapter();
    let p = parameters();
    let pixels = 262_144u32;
    let data: Vec<f32> = (0..pixels)
        .flat_map(|i| {
            let n = 0.2 + (i % 251) as f32 * 0.007;
            [n * 0.91 + 0.03, n, n * 1.09 - 0.02]
        })
        .collect();
    let (expected, offset, _) = reference(&data, &p);
    let source = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scan_large_frame_source"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scan_large_frame_output"),
        size: data.len() as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let state = gpu
        .build_scan_density_state(&p, &source, &output, pixels, 1, None)
        .unwrap();
    state.submit_setup(&gpu).unwrap();
    let setup = gpu
        .pipe
        .readback_buffer_as::<f32>(&state.prepared.setup, 64);
    let offset_error = (0..3)
        .map(|c| (f64::from(setup[5 + c]) - offset[c]).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        offset_error < 1e-5,
        "large-frame setup offset error {offset_error}"
    );
    state.submit_inverse(&gpu).unwrap();
    state.check(&gpu, &p, None, pixels, 1).unwrap();
    let actual = gpu
        .pipe
        .readback_buffer_as::<f32>(&output, data.len() as u64 * 4);
    let tone_error = actual
        .iter()
        .zip(&expected)
        .map(|(&a, &b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(tone_error < 1e-5, "large-frame tone error {tone_error}");
    eprintln!(
        "scan large-frame max offset error {offset_error} log10 ({} EV), tone error {tone_error}",
        offset_error / std::f64::consts::LOG10_2
    );
}

mod logic_tests {
    use super::*;

    #[test]
    fn positive_tone_rejects_dmax_outside_f32_range() {
        let max = f64::from(f32::MAX);
        for value in [0.0, -0.0, 1.0, -1.0, max, -max] {
            for channel in 0..3 {
                let mut dmax = [1.0; 3];
                dmax[channel] = value;
                assert!(
                    validate_dmax(&dmax).is_ok(),
                    "dmax {value} in channel {channel}"
                );
            }
        }
        let beyond = max * (1.0 + f64::EPSILON);
        for value in [
            beyond,
            -beyond,
            1e308,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ] {
            for channel in 0..3 {
                let mut dmax = [1.0; 3];
                dmax[channel] = value;
                assert_eq!(
                    validate_dmax(&dmax).unwrap_err(),
                    "negative-to-positive dmax exceeds finite f32 range"
                );
            }
        }
    }
}
