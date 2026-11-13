use super::*;

fn parameters() -> ScannerDensityGpuParams {
    let pivot = (0.16_f64 / 0.184).log10();
    let _gamma = 0.55;
    ScannerDensityGpuParams {
        density: vec![0.0, 4.0, 0.0, 4.4, 0.0, 4.8],
        exposure: vec![-4.0, 4.0, -4.0, 4.0, -4.0, 4.0],
        offsets: [0, 2, 4],
        counts: [2; 3],
        contrast: [0.73551400058216643; 3],
        dmax: [1.5625841465572565; 3],
        brightness_ev: [0.8781766618; 3],
        denominator: [-(-std::f64::consts::LN_10 * 1.5625841465572565).exp_m1(); 3],
        camera_ev: 0.0,
        lens_blur: 0.0,
        operator_pivot: pivot,
        operator_contrast_delta: 0.23,
        operator_exposure_ev: -0.31,
        operator_color: [0.12, -0.04, -0.08],
        tone_to_output: [[1.1, -0.1, 0.0], [-0.03, 1.03, 0.0], [0.0, -0.04, 1.04]],
    }
}

fn reference(data: &[f32], p: &ScannerDensityGpuParams) -> Vec<f32> {
    let slopes = [0.5, 0.55, 0.6];
    let mut output = Vec::new();
    for d in data.as_chunks::<3>().0 {
        let stops: [f64; 3] = std::array::from_fn(|c| {
            (f64::from(d[c]) / slopes[c] - 4.0 - p.operator_pivot) / std::f64::consts::LOG10_2
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
    output
}

#[test]
fn density_tone_matches_the_scalar_reference_with_operator_destination() {
    let gpu = crate::test_support::adapter();
    let p = parameters();
    for maximum in [1.9, 2.0] {
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
            let expected = reference(&data, &p);
            // Deliberately tiny physical source pages exercise paged reads.
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
            state.submit_inverse(&gpu).unwrap();
            state.check(&gpu, &p, None, 42, 1).unwrap();
            let actual = gpu
                .pipe
                .readback_buffer_as::<f32>(&output, data.len() as u64 * 4)
                .unwrap();
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
