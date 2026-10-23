//! Frame-level automatic setup. The inverse stays record-independent; only
//! this stage measures the whole frame. Constants are product choices, not a
//! calibrated Frontier/Noritsu response.
use super::interpretation::DensityCalibration;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::to_f64;

const EXPOSURE_SLOPE: f64 = 0.75;
const COLOR_SLOPE: f64 = 0.9;
const MIN_COLOR_WEIGHT: f64 = 0.2;

fn chroma(d: [f64; 3]) -> [f64; 2] {
    [
        (d[0] - d[1]) / 2.0_f64.sqrt(),
        (d[0] + d[1] - 2.0 * d[2]) / 6.0_f64.sqrt(),
    ]
}

/// One immutable result per executed scan; reused for every pixel. The input
/// density is already optically blurred. Samples are accumulated in raster
/// order in f64, independent of Rayon worker scheduling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FrameSetup {
    pub offset: [f64; 3],
    #[cfg(test)]
    pub color_weight: f64,
    #[cfg(test)]
    pub high_contrast: bool,
}

impl FrameSetup {
    pub fn measure(density: &ImageBuf, calibration: &DensityCalibration) -> Result<Self, String> {
        if density.data.is_empty() || !density.data.len().is_multiple_of(3) {
            return Err("standard setup requires a nonempty RGB density frame".into());
        }
        let aim_x = (0.16_f64 / 0.184).log10();
        let aim = calibration.neutral_density(aim_x).iter().sum::<f64>() / 3.0;
        let gamma = calibration.neutral_gamma(aim_x);
        if !aim.is_finite() || !gamma.is_finite() || gamma <= 0.0 {
            return Err(
                "standard setup requires a finite positive neutral-curve slope at 16%".into(),
            );
        }
        let mut neutrals = Vec::with_capacity(density.data.len() / 3);
        let mut sum_n = 0.0;
        let mut sum_cast = [0.0; 2];
        for px in density.data.as_chunks::<3>().0.iter() {
            let d = [to_f64(px[0]), to_f64(px[1]), to_f64(px[2])];
            if d.iter().any(|v| !v.is_finite()) {
                return Err("standard setup requires finite film record densities".into());
            }
            let x = calibration.estimate(d)?;
            let mean_x = x.iter().sum::<f64>() / 3.0;
            // Stock neutral curves need not be parallel. Compare each sample
            // to the neutral locus at its own exposure, not a fixed mask aim:
            // otherwise a neutral gray ramp itself would trigger a cast.
            let neutral = calibration.neutral_density(mean_x);
            let residual = std::array::from_fn(|c| d[c] - neutral[c]);
            let cast = chroma(residual);
            for c in 0..2 {
                sum_cast[c] += cast[c];
            }
            let n = d.iter().sum::<f64>() / 3.0;
            sum_n += n;
            neutrals.push(n);
        }
        let count = neutrals.len() as f64;
        if !sum_n.is_finite() || sum_cast.iter().any(|v| !v.is_finite()) {
            return Err("standard setup statistics exceed finite density range".into());
        }
        // Only the fifth percentile is needed; select rather than sort the
        // entire image. Linear interpolation matches the declared percentile.
        let p = (neutrals.len() - 1) as f64 * 0.05;
        let lo = p.floor() as usize;
        let (_, value, tail) = neutrals.select_nth_unstable_by(lo, f64::total_cmp);
        let n5 = *value
            + (p - lo as f64) * (tail.iter().copied().reduce(f64::min).unwrap_or(*value) - *value);
        let nmin = neutrals.iter().copied().reduce(f64::min).unwrap();
        let nmax = neutrals.iter().copied().reduce(f64::max).unwrap();
        let high = nmax - n5 >= 1.8;
        let w = if high {
            [0.30, 0.35, 0.29]
        } else {
            [0.33, 0.30, 0.35]
        };
        // The two published branches sum to 0.94 and 0.98. Normalize both;
        // a constant 16% field must give zero exposure correction.
        let en = (w[0] * (sum_n / count) + w[1] * nmax + w[2] * nmin) / w.iter().sum::<f64>();
        let dx = EXPOSURE_SLOPE * (aim - en) / gamma;
        let cast = sum_cast.map(|v| v / count);
        let weight = (1.0 - 40.0 * (cast[0].powi(2) + cast[1].powi(2))).max(MIN_COLOR_WEIGHT);
        let [u, v] = cast.map(|v| -COLOR_SLOPE * weight * v / gamma);
        let offset = [
            dx + u / 2.0_f64.sqrt() + v / 6.0_f64.sqrt(),
            dx - u / 2.0_f64.sqrt() + v / 6.0_f64.sqrt(),
            dx - 2.0 * v / 6.0_f64.sqrt(),
        ];
        if offset.iter().any(|v| !v.is_finite()) {
            return Err("standard setup correction exceeds finite exposure range".into());
        }
        tracing::debug!(target: "spektrafilm_core::scan_setup", offset_log10 = ?offset,
            color_weight = weight, high_contrast = high, "standard frame setup");
        Ok(Self {
            offset,
            #[cfg(test)]
            color_weight: weight,
            #[cfg(test)]
            high_contrast: high,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    fn film() -> &'static crate::profile::Profile {
        static FILM: std::sync::OnceLock<crate::profile::Profile> = std::sync::OnceLock::new();
        FILM.get_or_init(|| {
            crate::profile::load_profile_by_name(
                &crate::test_support::data_dir(),
                "kodak_portra_400",
            )
            .unwrap()
        })
    }

    fn density(cal: &DensityCalibration, exposures: &[[f64; 3]]) -> ImageBuf {
        let data = exposures
            .iter()
            .flat_map(|x| {
                let d: [f64; 3] = std::array::from_fn(|c| cal.neutral_density(x[c])[c]);
                d.map(from_f64)
            })
            .collect();
        ImageBuf::from_data(exposures.len() as u32, 1, data)
    }

    #[test]
    fn standard_renders_gray_ramps_retains_exposure_and_is_frame_dependent() {
        let mut profile = film().clone();
        // Known linear curves make the slope contract exact, independent of
        // toe/shoulder behaviour. Unequal stock curves catch gray-ramp casts.
        profile.data.log_exposure = vec![-4.0, 4.0];
        profile.data.density_curves = vec![vec![0.0, 0.0, 0.0], vec![4.0, 4.4, 4.8]];
        let cal = DensityCalibration::prepare(&profile).unwrap();
        let aim_x = (0.16_f64 / 0.184).log10();
        let aim_frame = density(&cal, &[[aim_x; 3]; 4]);
        let setup = FrameSetup::measure(&aim_frame, &cal).unwrap();
        assert!(setup.offset.iter().all(|v| v.abs() < 2e-7), "{setup:?}");
        for shift_ev in [-2.0, -1.0, 0.0, 1.0, 2.0] {
            let x = aim_x + shift_ev * std::f64::consts::LOG10_2;
            let frame = density(&cal, &[[x; 3]; 4]);
            let measured = FrameSetup::measure(&frame, &cal).unwrap();
            for offset in measured.offset {
                let remaining = shift_ev + offset / std::f64::consts::LOG10_2;
                assert!((remaining - 0.25 * shift_ev).abs() < 2e-6, "{measured:?}");
            }
        }
        let xs: Vec<_> = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0]
            .map(|ev| [ev * std::f64::consts::LOG10_2; 3])
            .into();
        let frame = density(&cal, &xs);
        let saved = frame.data.clone();
        let a = FrameSetup::measure(&frame, &cal).unwrap();
        let b = FrameSetup::measure(&frame, &cal).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            frame.data, saved,
            "setup must not rewrite density/tap evidence"
        );
        assert!(
            a.offset.windows(2).all(|w| (w[0] - w[1]).abs() < 2e-7),
            "gray ramp cast: {a:?}"
        );
        let subset = density(&cal, &xs[..2]);
        assert_ne!(
            a.offset,
            FrameSetup::measure(&subset, &cal).unwrap().offset,
            "per-tile setup must be observably different from whole-frame setup"
        );

        let mut params = crate::params::RuntimeParams::default();
        params.scanner.scan_output = "negative_to_positive".into();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.unsharp_mask = [0.0; 2];
        let run = |params: &crate::params::RuntimeParams| {
            super::super::interpret_negative_density_stage(
                &frame,
                &profile,
                &super::super::style::STANDARD,
                params,
                &spektrafilm_backend::cpu_backend::CpuBackend,
            )
            .unwrap()
        };
        let rendered = run(&params);
        for px in rendered.data.as_chunks::<3>().0.iter() {
            assert!(
                px.iter()
                    .all(|v| v.is_finite() && *v >= from_f64(0.0) && *v <= from_f64(1.0))
            );
            assert!((to_f64(px[0]) - to_f64(px[2])).abs() < 1e-6, "{px:?}");
        }
        assert!(
            rendered
                .data
                .as_chunks::<3>()
                .0
                .iter()
                .map(|px| to_f64(px[0]))
                .collect::<Vec<_>>()
                .windows(2)
                .all(|w| w[0] < w[1])
        );
        params.camera.exposure_compensation_ev = 2.0;
        assert_eq!(
            run(&params).data,
            rendered.data,
            "standard must not subtract camera exposure again after auto setup"
        );
    }

    #[test]
    fn standard_guards_dominant_color_and_handles_both_contrast_branches_and_errors() {
        let cal = DensityCalibration::prepare(film()).unwrap();
        let x = (0.16_f64 / 0.184).log10();
        let neutral = density(&cal, &[[x; 3]; 8]);
        let n = FrameSetup::measure(&neutral, &cal).unwrap();
        assert!(!n.high_contrast);
        assert!((n.color_weight - 1.0).abs() < 1e-6);
        let slight =
            FrameSetup::measure(&density(&cal, &[[x + 0.02, x - 0.01, x - 0.01]; 8]), &cal)
                .unwrap();
        let strong =
            FrameSetup::measure(&density(&cal, &[[x + 0.8, x - 0.4, x - 0.4]; 8]), &cal).unwrap();
        assert!(slight.color_weight > strong.color_weight);
        assert_eq!(strong.color_weight, MIN_COLOR_WEIGHT);
        assert!(strong.offset.iter().all(|v| v.is_finite()));
        let extremes = density(&cal, &[[-4.0; 3], [4.0; 3]]);
        assert!(FrameSetup::measure(&extremes, &cal).unwrap().high_contrast);
        assert!(FrameSetup::measure(&ImageBuf::new(0, 0), &cal).is_err());
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut frame = neutral.clone();
            frame.data[0] = from_f64(invalid);
            assert!(FrameSetup::measure(&frame, &cal).is_err());
        }
        let backlight = density(&cal, &[[0.6; 3], [0.6; 3], [0.6; 3], [0.6; 3], [-0.45; 3]]);
        let back = FrameSetup::measure(&backlight, &cal).unwrap();
        assert!(back.offset.iter().all(|v| v.is_finite()));
        let params = crate::params::RuntimeParams::default();
        let tone = &super::super::style::STANDARD[0];
        let background = super::super::positive_tone(0.6 + back.offset[0], tone, &params);
        let subject = super::super::positive_tone(-0.45 + back.offset[0], tone, &params);
        assert!(subject > 0.0 && subject < background && background <= 1.0);
        let night = FrameSetup::measure(&density(&cal, &[[x - 0.6; 3]; 8]), &cal).unwrap();
        let normal_v = super::super::positive_tone(x + n.offset[0], tone, &params);
        let night_v = super::super::positive_tone(x - 0.6 + night.offset[0], tone, &params);
        assert!(
            night_v < normal_v,
            "night must not become day: {night_v} vs {normal_v}"
        );
    }
}

#[cfg(test)]
mod stock_tests {
    use super::*;
    use spektrafilm_math::precision::{from_f64, to_f64};

    #[test]
    fn bundled_negative_stocks_render_neutral_ramps_and_retain_exposure_and_color() {
        let data = crate::test_support::data_dir();
        let mut tested = 0;
        for entry in std::fs::read_dir(data.join("profiles")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let film = crate::profile::load_profile(&path).unwrap();
            if !film.is_negative() || !film.is_film() || !film.is_filming() || !film.is_color() {
                continue;
            }
            let cal = DensityCalibration::prepare(&film).unwrap();
            let density = |xs: &[[f64; 3]]| {
                ImageBuf::from_data(
                    xs.len() as u32,
                    1,
                    xs.iter()
                        .flat_map(|x| {
                            std::array::from_fn::<_, 3, _>(|c| {
                                from_f64(cal.neutral_density(x[c])[c])
                            })
                        })
                        .collect(),
                )
            };
            let aim = (0.16_f64 / 0.184).log10();
            let patch = density(&[[aim; 3]; 4]);
            let setup = FrameSetup::measure(&patch, &cal).unwrap();
            assert!(
                setup.offset.iter().all(|v| v.abs() < 2e-6),
                "{} aim offset {setup:?}",
                path.display()
            );
            let mut params = crate::params::RuntimeParams::default();
            params.workflow.route = "input > film > scan".into();
            params.io.scan_film = true;
            params.scanner.scan_output = "negative_to_positive".into();
            params.scanner.unsharp_mask = [0.0; 2];
            let render = |frame: &ImageBuf| {
                super::super::interpret_negative_density_stage(
                    frame,
                    &film,
                    &super::super::style::STANDARD,
                    &params,
                    &spektrafilm_backend::cpu_backend::CpuBackend,
                )
                .unwrap()
            };
            let exposure_ramp: Vec<_> = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0]
                .map(|ev| [aim + ev * std::f64::consts::LOG10_2; 3])
                .into();
            let ramp = density(&exposure_ramp);
            let rendered = render(&ramp);
            assert_eq!(render(&ramp).data, rendered.data);
            for px in rendered.data.as_chunks::<3>().0.iter() {
                assert!(
                    px.iter()
                        .all(|v| v.is_finite() && *v >= from_f64(0.0) && *v <= from_f64(1.0))
                );
                assert!(
                    (to_f64(px[0]) - to_f64(px[2])).abs() < 2e-5,
                    "{} {px:?}",
                    path.display()
                );
            }
            assert!(
                rendered
                    .data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|v| to_f64(v[0]))
                    .collect::<Vec<_>>()
                    .windows(2)
                    .all(|w| w[0] < w[1]),
                "{} monotonicity",
                path.display()
            );
            // Exact (1-slope) applies only on linear stock curves. Here test
            // observable ordering without inventing that stronger equality.
            let mut values = Vec::new();
            for ev in [-1.0, 0.0, 1.0] {
                let frame = density(&[[aim + ev * std::f64::consts::LOG10_2; 3]; 4]);
                values.push(to_f64(render(&frame).data[0]));
            }
            assert!(
                values.windows(2).all(|w| w[0] < w[1]),
                "{} exposure retention {values:?}",
                path.display()
            );
            let red = density(&[[aim + 0.8, aim - 0.4, aim - 0.4]; 4]);
            let rgb = render(&red);
            assert!(
                rgb.data[0] > rgb.data[1],
                "{} dominant red washed out",
                path.display()
            );
            tested += 1;
        }
        assert!(tested > 1, "exercise more than the single research stock");
        eprintln!("standard setup checked {tested} bundled color-negative filming stocks");
    }
}
