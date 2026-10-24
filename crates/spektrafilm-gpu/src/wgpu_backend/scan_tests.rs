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
            state.submit_inverse(&gpu).unwrap();
            state.check(&gpu, &p, None, 42, 1).unwrap();
            let actual = gpu
                .pipe
                .readback_buffer_as::<f32>(&output, data.len() as u64 * 4);
            for (i, (&a, &b)) in actual.iter().zip(&expected).enumerate() {
                assert!((a - b).abs() < 1e-5, "sample {i}: GPU {a}, CPU {b}");
            }
        }
    }
}

#[test]
fn positive_tone_finite_huge_dmax_white_shoulder_matches_cpu_order() {
    let gpu = crate::test_support::adapter();
    let mut p = parameters();
    p.standard_setup = false;
    p.camera_ev = 0.0;
    p.dmax = [1e308; 3];
    p.contrast = [1.0; 3];
    p.brightness_ev = [1000.0; 3];
    p.denominator = p.dmax.map(|d| -(-std::f64::consts::LN_10 * d).exp_m1());
    p.operator_contrast_delta = 0.0;
    p.operator_exposure_ev = 0.0;
    p.operator_color = [0.0; 3];
    p.tone_to_output = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    // Each calibrated axis reconstructs exposure zero. CPU forms a zero
    // density at this white shoulder before multiplying by LN_10.
    let values = [2.0f32, 2.2, 2.4];
    let source = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("huge_dmax_white_source"),
            contents: bytemuck::cast_slice(&values),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("huge_dmax_white_output"),
        size: 12,
        mapped_at_creation: false,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });
    let state = gpu
        .build_scan_density_state(&p, &source, &output, 1, 1, None)
        .unwrap();
    state.submit_optics(&gpu).unwrap();
    state.submit_setup(&gpu).unwrap();
    state.submit_inverse(&gpu).unwrap();
    state.check(&gpu, &p, None, 1, 1).unwrap();
    let actual = gpu.pipe.readback_buffer_as::<f32>(&output, 12);
    let t = 1.0 / (1.0 + (-1000.0_f64).exp());
    let density = p.dmax[0] * (1.0 - t);
    let expected = (((-std::f64::consts::LN_10 * density).exp_m1() + p.denominator[0])
        / p.denominator[0]) as f32;
    assert_eq!(expected, 1.0);
    assert_eq!(actual, vec![expected; 3]);
}
