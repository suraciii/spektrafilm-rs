use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn params(expectation: bool) -> RecordFieldParams {
    RecordFieldParams {
        event_contribution_d_um2: 0.2,
        support_um: 1.0,
        cell_um: 0.5,
        seed: 42,
        expectation,
    }
}

fn scalar_image(width: u32, height: u32, density: impl Fn(u32, u32) -> f64) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set(x, y, [from_f64(density(x, y)); 3]);
        }
    }
    image
}

#[test]
fn fractional_last_footprint_uses_the_planned_endpoint() {
    // Frozen-CLI regression: the old last lo+pitch rounded above the tile end,
    // requiring a second cell that the tile had not allocated.
    let h = 4.999999888241291;
    let p = RecordFieldParams {
        support_um: 10.049999775364993,
        cell_um: h,
        ..params(true)
    };
    let grid = ReadoutGrid {
        origin_um: [0.0; 2],
        pitch_um: [0.45454544438557193, h],
        width: 11,
        height: 1,
    };
    let target = scalar_image(11, 1, |_, _| 0.4);
    let calls = AtomicUsize::new(0);
    let actual = apply_record_field_with_response(&target, grid.pitch_um[0], grid, &p, |formed| {
        calls.fetch_add(1, Ordering::Relaxed);
        let mut light = formed.clone();
        for value in &mut light.data {
            *value = from_f64(10.0_f64.powf(-to_f64(*value)));
        }
        Ok(light)
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    for &value in &actual.data {
        assert!((to_f64(value) - 10.0_f64.powf(-to_f64(target.data[0]))).abs() < 3e-7);
    }
}

#[test]
fn late_invalid_tile_is_rejected_before_any_response() {
    let p = params(true);
    let radius = 9.0; // ceil(4*sigma/h)+1 for the explicit parameters above.
    let last_address = i32::MAX as f64 - radius - 2.0;
    let grid = ReadoutGrid {
        // Tile 0 has 128 valid pixels. Tile 1 exceeds the global address bound.
        origin_um: [(last_address - 128.0) * p.cell_um, 0.0],
        pitch_um: [p.cell_um; 2],
        width: 129,
        height: 1,
    };
    let target = scalar_image(1, 1, |_, _| 0.4);
    let calls = AtomicUsize::new(0);
    let error = apply_record_field_with_response(&target, 0.5, grid, &p, |formed| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(formed.clone())
    })
    .err()
    .unwrap();
    assert!(error.contains("address range"), "{error}");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn source_output_and_stencil_budgets_precede_callbacks() {
    let p = params(true);
    let target = scalar_image(1, 1, |_, _| 0.4);
    let grid = ReadoutGrid {
        origin_um: [0.0; 2],
        pitch_um: [0.5; 2],
        width: 1,
        height: 1,
    };
    let calls = AtomicUsize::new(0);
    let response = |formed: &ImageBuf| {
        calls.fetch_add(1, Ordering::Relaxed);
        Ok(formed.clone())
    };
    let oversized_source = ImageBuf {
        width: (MAX_SOURCE_PIXELS + 1) as u32,
        height: 1,
        data: Vec::new(),
    };
    assert!(apply_record_field_with_response(&oversized_source, 0.5, grid, &p, response).is_err());
    let oversized_output = ReadoutGrid {
        width: (MAX_OUTPUT_PIXELS + 1) as u32,
        ..grid
    };
    assert!(
        apply_record_field_with_response(&target, 0.5, oversized_output, &p, response).is_err()
    );
    // A small source image with very fine source pixels still has a bounded
    // stencil; a genuinely wide source axis must fail before allocating it.
    let wide_source = scalar_image(20_000, 1, |_, _| 0.4);
    assert!(apply_record_field_with_response(&wide_source, 0.00001, grid, &p, response).is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn cropped_fractional_readout_retains_global_events_across_tile_seams() {
    let p = params(false);
    let target = scalar_image(19, 7, |x, y| 0.1 + 0.035 * f64::from(x + y));
    let grid = ReadoutGrid {
        origin_um: [-1.1, 0.27],
        pitch_um: [0.3, 0.7],
        width: 260,
        height: 3,
    };
    let full = apply_record_field(&target, 0.73, grid, &p).unwrap();
    let crop = ReadoutGrid {
        origin_um: [
            grid.origin_um[0] + 120.0 * grid.pitch_um[0],
            grid.origin_um[1],
        ],
        width: 17,
        ..grid
    };
    let actual = apply_record_field(&target, 0.73, crop, &p).unwrap();
    for y in 0..crop.height {
        for x in 0..crop.width {
            for c in 0..3 {
                let a = to_f64(actual.data[actual.idx(x, y, c)]);
                let b = to_f64(full.data[full.idx(x + 120, y, c)]);
                assert!((a - b).abs() < 3e-7, "({x},{y},{c}): {a} != {b}");
            }
        }
    }
}

// Independent oracle: integrate the triangular overlap of two cells against
// the truncated Gaussian with composite Simpson quadrature. It uses neither
// production support_taps/tap/double_cdf nor production remap/readout stencils.
fn oracle_taps(sigma: f64, h: f64) -> Vec<(i64, f64)> {
    let radius = (4.0 * sigma / h).ceil() as i64 + 1;
    let normalization =
        sigma * (2.0 * std::f64::consts::PI).sqrt() * libm::erf(4.0 / std::f64::consts::SQRT_2);
    (-radius..=radius)
        .map(|offset| {
            let delta = offset as f64 * h;
            let low = (-h).max(-4.0 * sigma - delta);
            let high = h.min(4.0 * sigma - delta);
            let integrand = |u: f64| {
                (1.0 - u.abs() / h) * (-0.5 * ((delta + u) / sigma).powi(2)).exp() / normalization
            };
            let integrate = |a: f64, b: f64| {
                if b <= a {
                    return 0.0;
                }
                const N: usize = 256;
                let step = (b - a) / N as f64;
                let mut sum = integrand(a) + integrand(b);
                for k in 1..N {
                    sum += if k % 2 == 0 { 2.0 } else { 4.0 } * integrand(a + k as f64 * step);
                }
                sum * step / 3.0
            };
            let value = if low < 0.0 && high > 0.0 {
                integrate(low, 0.0) + integrate(0.0, high)
            } else {
                integrate(low, high)
            };
            (offset, value)
        })
        .collect()
}

fn oracle_source_mean(target: &ImageBuf, pitch: f64, h: f64, x: i64, y: i64) -> f64 {
    let overlap = |cell: i64, pixel: u32, extent: u32| {
        let lo = if pixel == 0 {
            f64::NEG_INFINITY
        } else {
            f64::from(pixel) * pitch
        };
        let hi = if pixel + 1 == extent {
            f64::INFINITY
        } else {
            f64::from(pixel + 1) * pitch
        };
        (((cell + 1) as f64 * h).min(hi) - (cell as f64 * h).max(lo)).max(0.0) / h
    };
    let mut mean = 0.0;
    for sy in 0..target.height {
        for sx in 0..target.width {
            mean += overlap(x, sx, target.width)
                * overlap(y, sy, target.height)
                * to_f64(target.data[target.idx(sx, sy, 0)]);
        }
    }
    mean
}

fn oracle_footprint(lo: f64, hi: f64, h: f64) -> Vec<(i64, f64)> {
    let first = (lo / h).floor() as i64 - 1;
    let past = (hi / h).ceil() as i64 + 1;
    (first..past)
        .filter_map(|cell| {
            let overlap = (hi.min((cell + 1) as f64 * h) - lo.max(cell as f64 * h)).max(0.0);
            (overlap > 0.0).then_some((cell, overlap / (hi - lo)))
        })
        .collect()
}

fn optical(density: f64) -> [f64; 3] {
    [
        (-density).exp(),
        10.0_f64.powf(-2.0 * density),
        0.2 + 0.7 * (-3.0 * density).exp(),
    ]
}

#[test]
fn nonuniform_optical_readout_matches_independent_formation_integral() {
    let p = params(true);
    let target = scalar_image(5, 3, |x, y| {
        0.03 + 0.16 * f64::from(x) + 0.23 * f64::from(y)
    });
    let pitch = 0.73;
    let grid = ReadoutGrid {
        origin_um: [-0.31, 0.27],
        pitch_um: [1.17, 0.91],
        width: 4,
        height: 3,
    };
    let actual = apply_record_field_with_response(&target, pitch, grid, &p, |formed| {
        let mut light = formed.clone();
        for pixel in light.data.as_chunks_mut::<3>().0 {
            let values = optical(to_f64(pixel[0]));
            for (lane, value) in pixel.iter_mut().zip(values) {
                *lane = from_f64(value);
            }
        }
        Ok(light)
    })
    .unwrap();
    let taps = oracle_taps(p.support_um, p.cell_um);
    let mut nonlinear_gap: f64 = 0.0;
    for y in 0..grid.height {
        for x in 0..grid.width {
            let xs = oracle_footprint(
                grid.origin_um[0] + f64::from(x) * grid.pitch_um[0],
                grid.origin_um[0] + f64::from(x + 1) * grid.pitch_um[0],
                p.cell_um,
            );
            let ys = oracle_footprint(
                grid.origin_um[1] + f64::from(y) * grid.pitch_um[1],
                grid.origin_um[1] + f64::from(y + 1) * grid.pitch_um[1],
                p.cell_um,
            );
            let mut expected = [0.0; 3];
            let mut mean_density = 0.0;
            for &(cy, b) in &ys {
                for &(cx, a) in &xs {
                    let mut formed = 0.0;
                    for &(dy, ky) in &taps {
                        for &(dx, kx) in &taps {
                            formed += kx
                                * ky
                                * oracle_source_mean(&target, pitch, p.cell_um, cx - dx, cy - dy);
                        }
                    }
                    mean_density += a * b * formed;
                    for (value, light) in expected.iter_mut().zip(optical(formed)) {
                        *value += a * b * light;
                    }
                }
            }
            nonlinear_gap = nonlinear_gap.max((expected[1] - optical(mean_density)[1]).abs());
            for (c, expected) in expected.into_iter().enumerate() {
                let value = to_f64(actual.data[actual.idx(x, y, c)]);
                assert!(
                    (value - expected).abs() < 3e-7,
                    "({x},{y},{c}): {value} != {expected}"
                );
            }
        }
    }
    // This fixture distinguishes E[response(D)] from response(E[D]).
    assert!(
        nonlinear_gap > 1e-4,
        "fixture lost optical noncommutativity"
    );
}

#[test]
fn seeded_boundary_ensemble_has_poisson_mean_and_variance() {
    let mut p = params(false);
    let target = scalar_image(3, 2, |x, y| 0.15 + 0.25 * f64::from(x) + 0.1 * f64::from(y));
    let pitch = 0.73;
    let grid = ReadoutGrid {
        origin_um: [-0.2, 0.15],
        pitch_um: [1.1, 0.8],
        width: 1,
        height: 1,
    };
    let taps = oracle_taps(p.support_um, p.cell_um);
    let xs = oracle_footprint(
        grid.origin_um[0],
        grid.origin_um[0] + grid.pitch_um[0],
        p.cell_um,
    );
    let ys = oracle_footprint(
        grid.origin_um[1],
        grid.origin_um[1] + grid.pitch_um[1],
        p.cell_um,
    );
    let radius = taps.last().unwrap().0;
    let mut expected_mean = 0.0;
    let mut expected_variance = 0.0;
    for y in ys.first().unwrap().0 - radius..=ys.last().unwrap().0 + radius {
        for x in xs.first().unwrap().0 - radius..=xs.last().unwrap().0 + radius {
            let mut weight = 0.0;
            for &(cy, b) in &ys {
                for &(cx, a) in &xs {
                    let kx = taps
                        .iter()
                        .find(|&&(offset, _)| offset == cx - x)
                        .map_or(0.0, |t| t.1);
                    let ky = taps
                        .iter()
                        .find(|&&(offset, _)| offset == cy - y)
                        .map_or(0.0, |t| t.1);
                    weight += a * b * kx * ky;
                }
            }
            let mean = oracle_source_mean(&target, pitch, p.cell_um, x, y);
            expected_mean += weight * mean;
            expected_variance +=
                weight * weight * mean * p.event_contribution_d_um2 / p.cell_um.powi(2);
        }
    }
    const SAMPLES: usize = 1024;
    let mut mean = 0.0;
    let mut m2 = 0.0;
    for sample in 0..SAMPLES {
        p.seed = 0xa67d_9b32_4c18_0000 + sample as u64;
        let image = apply_record_field(&target, pitch, grid, &p).unwrap();
        let value = to_f64(image.data[0]);
        let delta = value - mean;
        mean += delta / (sample + 1) as f64;
        m2 += delta * (value - mean);
    }
    let variance = m2 / (SAMPLES - 1) as f64;
    let mean_tolerance = 6.0 * (expected_variance / SAMPLES as f64).sqrt();
    assert!(
        (mean - expected_mean).abs() < mean_tolerance,
        "mean {mean} != {expected_mean}"
    );
    // Approximately six standard errors for variance, with margin for the
    // non-Gaussian Poisson fourth moment. Seeds are fixed, not retried.
    assert!(
        (variance / expected_variance - 1.0).abs() < 0.3,
        "variance {variance} != {expected_variance}"
    );
}

#[test]
fn scalar_contract_and_optical_callback_failures_are_errors() {
    let p = params(true);
    let target = scalar_image(1, 1, |_, _| 0.4);
    let grid = ReadoutGrid {
        origin_um: [0.0; 2],
        pitch_um: [0.5; 2],
        width: 1,
        height: 1,
    };
    for q in [0.0, -0.2, f64::NAN, f64::INFINITY] {
        let invalid = RecordFieldParams {
            event_contribution_d_um2: q,
            ..p
        };
        assert!(apply_record_field(&target, 0.5, grid, &invalid).is_err());
    }
    let mut nonscalar = target.clone();
    nonscalar.data[1] = from_f64(0.5);
    assert!(apply_record_field(&nonscalar, 0.5, grid, &p).is_err());
    let negative = scalar_image(1, 1, |_, _| -0.1);
    assert!(apply_record_field(&negative, 0.5, grid, &p).is_err());
    assert!(
        apply_record_field_with_response(&target, 0.5, grid, &p, |_| { Ok(ImageBuf::new(1, 2)) })
            .is_err()
    );
    assert!(
        apply_record_field_with_response(&target, 0.5, grid, &p, |formed| {
            let mut image = formed.clone();
            image.data[0] = from_f64(f64::NAN);
            Ok(image)
        })
        .is_err()
    );
    let error = apply_record_field_with_response(&target, 0.5, grid, &p, |_| {
        Err("optical response failed".to_owned())
    })
    .err()
    .unwrap();
    assert_eq!(error, "optical response failed");
}

#[test]
fn zero_target_has_no_floor_or_events() {
    let target = scalar_image(2, 3, |_, _| 0.0);
    let grid = ReadoutGrid {
        origin_um: [-0.37, 0.19],
        pitch_um: [0.71, 1.13],
        width: 3,
        height: 2,
    };
    for expectation in [false, true] {
        let image = apply_record_field(&target, 0.73, grid, &params(expectation)).unwrap();
        assert!(image.data.iter().all(|&value| to_f64(value) == 0.0));
    }
}
