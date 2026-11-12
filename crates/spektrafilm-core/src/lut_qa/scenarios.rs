//! The pinned print QA scenarios and the cube measurements they share.

use super::color::color;
use super::convex_hull::volume;
use super::{
    QaResult, bundle_input_gain, cct_xy, delta_e_2000, diff, display, dynamic_range, face_flips,
    failed_scenario, fft_ratios, grid, hue_delta, ictcp, input_compression, inside, interp_pairs,
    limits, locus, max, mean, oklab, percentile, picture, plot, png, process_linear, reference,
    result, singular, tetra, with_figure, xy, xyz, xyz_to_lab,
};
use crate::{
    gamut_compression::OutputGamutCompress,
    lut_baker::{Bundle, Lut},
    lut_transport::ColorSpaceEntry,
    math::mat_vec,
    runtime::Runtime,
};
use serde_json::{Value, json};
use spektrafilm_backend::ComputeBackend;
use std::{collections::BTreeMap, path::Path};

/// Shared per-cube axis-neighbor measurements consumed by monotonicity and
/// total variation.
struct CubeAxisStats {
    violations: usize,
    worst: f64,
    tv: [f64; 3],
    counts: [usize; 3],
}

/// Run the fixed print scenarios against one resolved LUT.
pub(super) struct PrintQa<'a> {
    bundle: &'a Bundle,
    lut: &'a Lut,
    input: &'a ColorSpaceEntry,
    output: &'a ColorSpaceEntry,
    pipeline: &'a Runtime,
    unbounded: &'a Runtime,
    backend: &'a dyn ComputeBackend,
    data: &'a Path,
    root: &'a Path,
    n: usize,
    gain: f64,
    inputs: Vec<[f64; 3]>,
    samples: Vec<[f64; 3]>,
    locus: Vec<[f64; 2]>,
}

impl<'a> PrintQa<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        bundle: &'a Bundle,
        lut: &'a Lut,
        input: &'a ColorSpaceEntry,
        output: &'a ColorSpaceEntry,
        pipeline: &'a Runtime,
        unbounded: &'a Runtime,
        backend: &'a dyn ComputeBackend,
        data: &'a Path,
        root: &'a Path,
    ) -> Result<Self, String> {
        let gain = bundle_input_gain(bundle, input)?;
        let n = lut.resolution;
        let inputs = grid(n);
        let mut rng = super::stimulus::Pcg::seeded(20260515);
        let samples: Vec<_> = (0..50_000)
            .map(|_| std::array::from_fn(|_| rng.uniform() as f32 as f64))
            .collect();
        Ok(Self {
            bundle,
            lut,
            input,
            output,
            pipeline,
            unbounded,
            backend,
            data,
            root,
            n,
            gain,
            inputs,
            samples,
            locus: locus(),
        })
    }

    /// Push every result in the order pinned by the Python acceptance suite.
    pub(super) fn run(self) -> Result<Vec<QaResult>, String> {
        let mut results = Vec::new();
        results.push(self.off_grid_identity());
        let axis = self.cube_axis_stats();
        results.push(self.monotonicity(&axis));
        results.push(self.jacobian_condition());
        results.push(self.total_variation(&axis));
        results.push(self.output_gamut_compression());
        results.push(self.characteristic_curve());
        match dynamic_range(self.lut, self.input, self.output, self.root) {
            Ok(r) => results.push(r),
            Err(error) => results.push(failed_scenario("dynamic_range_usage", error)),
        }
        results.push(self.planckian_sweep());
        results.push(self.hue_twist_oklab());
        results.push(self.spectral_locus_envelope());
        results.extend(input_compression(
            self.bundle,
            &self.inputs,
            self.input,
            self.data,
            self.root,
            &self.locus,
        )?);
        results.extend(picture(
            self.bundle,
            self.lut,
            self.input,
            self.output,
            self.pipeline,
            self.backend,
            self.root,
        )?);
        Ok(results)
    }

    fn off_grid_identity(&self) -> QaResult {
        match reference(
            self.pipeline,
            &self.samples,
            self.input,
            self.output,
            self.gain,
            self.backend,
        ) {
            Ok(truth) => {
                let mut tri = Vec::with_capacity(self.samples.len());
                let mut tet = Vec::with_capacity(self.samples.len());
                let mut itp = Vec::with_capacity(self.samples.len());
                for (&p, &t) in self.samples.iter().zip(&truth) {
                    let a = xyz(self.output, self.lut.sample(p));
                    let b = xyz(self.output, t);
                    tri.push(delta_e_2000(xyz_to_lab(a), xyz_to_lab(b)));
                    tet.push(delta_e_2000(
                        xyz_to_lab(xyz(self.output, tetra(self.lut, p))),
                        xyz_to_lab(b),
                    ));
                    let a = ictcp(a);
                    let b = ictcp(b);
                    itp.push(
                        720. * ((a[0] - b[0]).powi(2)
                            + 0.25 * (a[1] - b[1]).powi(2)
                            + (a[2] - b[2]).powi(2))
                        .sqrt(),
                    );
                }
                let mut r = result(
                    "off_grid_identity",
                    json!({"trilinear_dE2000_max":max(&tri),"trilinear_dE2000_p99":percentile(&tri,0.99),"trilinear_dE2000_p50":percentile(&tri,0.5),"tetrahedral_dE2000_max":max(&tet),"tetrahedral_dE2000_p99":percentile(&tet,0.99),"tetrahedral_dE2000_p50":percentile(&tet,0.5),"trilinear_dITP_max":max(&itp),"trilinear_dITP_p99":percentile(&itp,0.99)}),
                    Some(
                        max(&tri) <= 2.
                            && percentile(&tri, 0.99) <= 1.
                            && max(&tet) <= 2.
                            && percentile(&tet, 0.99) <= 1.,
                    ),
                    "ΔE₀₀",
                    "Interpolation error against the full scientific pipeline, including input exposure and output transport scaling.",
                );
                limits(
                    &mut r,
                    &[
                        ("trilinear_dE2000_max", "≤ 2.0"),
                        ("trilinear_dE2000_p99", "≤ 1.0"),
                        ("tetrahedral_dE2000_max", "≤ 2.0"),
                        ("tetrahedral_dE2000_p99", "≤ 1.0"),
                    ],
                );
                let points: Vec<_> = self.samples.iter().map(|p| [p[0], p[1]]).collect();
                let colors: Vec<_> = tri
                    .iter()
                    .map(|v| [(v / 2.).clamp(0., 1.), 0.2, 1. - (v / 2.).clamp(0., 1.)])
                    .collect();
                let figure = plot(self.root, &r.name, &points, &colors);
                with_figure(r, figure)
            }
            Err(error) => failed_scenario("off_grid_identity", error),
        }
    }

    fn cube_axis_stats(&self) -> CubeAxisStats {
        let mut stats = CubeAxisStats {
            violations: 0,
            worst: 0.,
            tv: [0.; 3],
            counts: [0; 3],
        };
        for r in 0..self.n {
            for g in 0..self.n {
                for b in 0..self.n {
                    let p = [r, g, b];
                    for a in 0..3 {
                        if p[a] + 1 < self.n {
                            let mut q = p;
                            q[a] += 1;
                            let x = self.lut.at(r, g, b);
                            let y = self.lut.at(q[0], q[1], q[2]);
                            let d = y[a] - x[a];
                            if d < 0. {
                                stats.violations += 1;
                                stats.worst = stats.worst.min(d);
                            }
                            for c in 0..3 {
                                stats.tv[a] += (y[c] - x[c]).abs();
                                stats.counts[a] += 1;
                            }
                        }
                    }
                }
            }
        }
        stats
    }

    fn monotonicity(&self, axis: &CubeAxisStats) -> QaResult {
        let pin = self.input.encode(0.18 / self.gain);
        let mut curves = Vec::new();
        let mut center_violations = 0;
        for channel in 0..3 {
            let mut previous = None;
            for i in 0..65 {
                let t = i as f64 / 64.;
                let mut p = [pin; 3];
                p[channel] = t;
                let y = self.lut.sample(p);
                if previous.is_some_and(|v| y[channel] < v) {
                    center_violations += 1;
                }
                previous = Some(y[channel]);
                curves.push([t, 1. - y[channel]]);
            }
        }
        let mut r = result(
            "monotonicity",
            json!({"violations":axis.violations,"worst_negative_diff":axis.worst,"centerline_pin_encoded":pin,"centerline_violations":center_violations}),
            Some(axis.violations == 0),
            "cells",
            "Diagonal axis-channel negative differences are cube-wide fold-backs.",
        );
        limits(
            &mut r,
            &[
                ("violations", "== 0"),
                ("worst_negative_diff", "== 0 when violations == 0"),
            ],
        );
        let figure = plot(
            self.root,
            &r.name,
            &curves,
            &vec![[0.7, 0.8, 0.9]; curves.len()],
        );
        with_figure(r, figure)
    }

    fn jacobian_condition(&self) -> QaResult {
        let mut cond = Vec::new();
        for r in 1..self.n - 1 {
            for g in 1..self.n - 1 {
                for b in 1..self.n - 1 {
                    let derivatives = [
                        diff(self.lut.at(r + 1, g, b), self.lut.at(r - 1, g, b)),
                        diff(self.lut.at(r, g + 1, b), self.lut.at(r, g - 1, b)),
                        diff(self.lut.at(r, g, b + 1), self.lut.at(r, g, b - 1)),
                    ];
                    let s = singular(std::array::from_fn(|i| {
                        std::array::from_fn(|j| derivatives[j][i])
                    }));
                    cond.push((s[0] / s[2].max(1e-12)).max(1.).log10());
                }
            }
        }
        let r = result(
            "jacobian_condition",
            json!({"max_log10_cond":max(&cond),"p99_log10_cond":percentile(&cond,0.99),"p50_log10_cond":percentile(&cond,0.5)}),
            None,
            "log10(cond J)",
            "Central-difference Jacobian conditioning; near-singular shoulders and compression shells are informational.",
        );
        let pix: Vec<_> = cond
            .iter()
            .map(|v| [(v / 4.).clamp(0., 1.), 0.2, 0.4])
            .collect();
        let figure = png(
            self.root,
            &r.name,
            (self.n - 2) * (self.n - 2),
            self.n - 2,
            &pix,
        );
        with_figure(r, figure)
    }

    fn total_variation(&self, axis: &CubeAxisStats) -> QaResult {
        let mut tv = axis.tv;
        for (a, value) in tv.iter_mut().enumerate() {
            *value /= axis.counts[a] as f64;
        }
        let fft = fft_ratios(self.lut);
        let r = result(
            "total_variation",
            json!({"tv":tv.iter().sum::<f64>(),"tv_r":tv[0],"tv_g":tv[1],"tv_b":tv[2],"axial_highband_ratio_r":fft[2],"axial_highband_ratio_g":fft[1],"axial_highband_ratio_b":fft[0],"axial_highband_ratio_mean":mean(&fft)}),
            None,
            "",
            "Mean absolute cube finite differences and upper-half axial FFT magnitude energy (Python axis-label convention).",
        );
        let figure = png(
            self.root,
            &r.name,
            self.n * self.n,
            self.n,
            &self
                .lut
                .table
                .iter()
                .map(|&p| display(p, self.output))
                .collect::<Vec<_>>(),
        );
        with_figure(r, figure)
    }

    fn output_gamut_compression(&self) -> QaResult {
        let gamut_result = (|| -> Result<QaResult, String> {
            let flips = face_flips(self.lut);
            let indices: Vec<_> = super::stimulus::hull_indices(self.inputs.len())
                .into_iter()
                .map(|i| {
                    let b = i / (self.n * self.n);
                    let g = i / self.n % self.n;
                    let r = i % self.n;
                    (r * self.n + g) * self.n + b
                })
                .collect();
            let projected_in: Vec<_> = indices
                .iter()
                .map(|&i| oklab(xyz(self.output, self.inputs[i])))
                .collect();
            let projected_out: Vec<_> = indices
                .iter()
                .map(|&i| oklab(xyz(self.output, self.lut.table[i])))
                .collect();
            let in_volume = volume(&projected_in)?;
            let out_volume = volume(&projected_out);
            let ratio = out_volume.as_ref().map(|v| v / in_volume).unwrap_or(0.);
            let mut rim = Vec::new();
            for axis in 0..3 {
                for a in [0., 1.] {
                    for b in [0., 1.] {
                        for i in 0..96 {
                            let mut p = [0.; 3];
                            p[axis] = i as f64 / 95.;
                            p[(axis + 1) % 3] = a;
                            p[(axis + 2) % 3] = b;
                            rim.push(p);
                        }
                    }
                }
            }
            let linear: Vec<_> = rim.iter().map(|&p| self.input.decode_rgb(p)).collect();
            let before = process_linear(self.unbounded, &linear, self.backend)?;
            let compressor = OutputGamutCompress::build(
                &self.bundle.spec.output_gamut_compress,
                self.output.primaries,
            )?;
            let after: Vec<_> = before.iter().map(|&p| compressor.compress(p)).collect();
            let mut bright = 0;
            let mut oog = 0;
            let mut displacement = Vec::new();
            for (&a, &b) in before.iter().zip(&after) {
                let ach = max(&a);
                if ach > 1e-2 {
                    bright += 1;
                    if max(&a.map(|v| (ach - v) / ach)) > 1. {
                        oog += 1;
                        displacement.push(super::distance(a, b));
                    }
                }
            }
            let mut r = result(
                "output_gamut_compression",
                json!({"fold_triangles":flips.0,"fold_fraction":flips.0 as f64/flips.1 as f64,"input_hull_volume":in_volume,"output_hull_volume":out_volume.as_ref().ok(),"compression_ratio":ratio,"compression_algorithm":self.bundle.spec.output_gamut_compress.algorithm,"rim_oog_fraction":oog as f64/bright.max(1) as f64,"rim_oog_samples":oog,"rim_max_displacement":if displacement.is_empty(){0.}else{max(&displacement)},"rim_mean_displacement":if displacement.is_empty(){0.}else{mean(&displacement)}}),
                Some(flips.0 == 0 && (0.05..=1.05).contains(&ratio) && out_volume.is_ok()),
                "",
                "Cube face folds and OkLab convex-hull compression budget, plus unbounded/compressed scientific rim probes.",
            );
            limits(
                &mut r,
                &[
                    ("fold_triangles", "== 0"),
                    ("compression_ratio", "in [0.05, 1.05]"),
                ],
            );
            let pts: Vec<_> = before
                .iter()
                .chain(&after)
                .map(|&p| xy(mat_vec(&color(self.output).m, p)))
                .collect();
            let colors = vec![[1., 0.4, 0.2]; before.len()]
                .into_iter()
                .chain(vec![[0.2, 0.7, 1.]; after.len()])
                .collect::<Vec<_>>();
            r.figure_path = Some(plot(self.root, &r.name, &pts, &colors)?);
            Ok(r)
        })();
        gamut_result.unwrap_or_else(|error| failed_scenario("output_gamut_compression", error))
    }

    fn characteristic_curve(&self) -> QaResult {
        let neutral: Vec<_> = (0..self.n).map(|i| self.lut.at(i, i, i)).collect();
        let density: Vec<_> = neutral
            .iter()
            .map(|p| p.map(|v| -v.clamp(1e-4, 1.).log10()))
            .collect();
        let spread = density
            .iter()
            .map(|p| max(p) - p.iter().copied().fold(f64::INFINITY, f64::min))
            .fold(0., f64::max);
        let mid = self.n / 2;
        let axis = |i: usize| 1e-6 + (1. - 1e-6) * i as f64 / (self.n - 1) as f64;
        let gamma = (mean(&neutral[mid + 1]).clamp(1e-4, 1.).log10()
            - mean(&neutral[mid - 1]).clamp(1e-4, 1.).log10())
            / (axis(mid + 1).log10() - axis(mid - 1).log10());
        let r = result(
            "characteristic_curve",
            json!({"system_gamma_at_mid":gamma,"max_channel_density_spread":spread}),
            None,
            "density",
            "Neutral characteristic response and channel-density divergence; off-diagonal sweeps use neutral-density-inverted pins.",
        );
        let mut pts = Vec::new();
        let mut colors = Vec::new();
        let probe: Vec<_> = (0..257)
            .map(|i| {
                let t = i as f64 / 256.;
                (t, -mean(&self.lut.sample([t; 3])).clamp(1e-4, 1.).log10())
            })
            .collect();
        for target in [0.2, 0.4, 0.6, 0.8, 1.] {
            let pin = interp_pairs(
                &probe.iter().rev().map(|&(x, y)| (y, x)).collect::<Vec<_>>(),
                target,
            );
            for a in 0..3 {
                for i in 0..65 {
                    let x = i as f64 / 64.;
                    let mut p = [pin; 3];
                    p[a] = x;
                    let y = self.lut.sample(p);
                    for (c, value) in y.iter().enumerate() {
                        pts.push([x, 1. + value.clamp(1e-4, 1.).log10() / 4.]);
                        colors.push(match c {
                            0 => [1., 0.2, 0.2],
                            1 => [0.2, 1., 0.2],
                            _ => [0.2, 0.2, 1.],
                        });
                    }
                }
            }
        }
        let figure = plot(self.root, &r.name, &pts, &colors);
        with_figure(r, figure)
    }

    fn planckian_sweep(&self) -> QaResult {
        let mut planck = Vec::new();
        for i in 0..16 {
            let t = 2700. + 7300. * i as f64 / 15.;
            let (x, y) = cct_xy(t);
            let rgb = mat_vec(&color(self.input).inv, [x / y, 1., (1. - x - y) / y]);
            let peak = max(&rgb).max(1e-6);
            planck.push(
                self.input
                    .encode_rgb(rgb.map(|v| (v / peak).clamp(0., 1.) / self.gain))
                    .map(|v| v as f32 as f64),
            );
        }
        let outxy: Vec<_> = planck
            .iter()
            .map(|&p| xy(xyz(self.output, self.lut.sample(p))))
            .collect();
        let vectors: Vec<_> = outxy
            .windows(2)
            .map(|p| [p[1][0] - p[0][0], p[1][1] - p[0][1]])
            .collect();
        let bend = vectors
            .windows(2)
            .map(|p| {
                let dot = p[0][0] * p[1][0] + p[0][1] * p[1][1];
                (dot / ((p[0][0].hypot(p[0][1]) + 1e-12) * (p[1][0].hypot(p[1][1]) + 1e-12)))
                    .clamp(-1., 1.)
                    .acos()
                    .to_degrees()
            })
            .fold(0., f64::max);
        let mut r = result(
            "planckian_sweep",
            json!({"max_bend_angle_deg":bend,"cct_range_k":"2700-10000"}),
            Some(bend <= 30.),
            "degrees",
            "Consecutive chromaticity-segment bend for daylight/Planckian white surfaces.",
        );
        limits(&mut r, &[("max_bend_angle_deg", "≤ 30°")]);
        let figure = plot(self.root, &r.name, &outxy, &vec![[1., 0.8, 0.3]; 16]);
        with_figure(r, figure)
    }

    fn hue_twist_oklab(&self) -> QaResult {
        let mut input_lab = Vec::new();
        let mut output_lab = Vec::new();
        let mut filtered = 0;
        for (&a, &b) in self.inputs.iter().zip(&self.lut.table) {
            let a = oklab(xyz(self.input, a));
            if a[1].hypot(a[2]) <= 0.6 {
                input_lab.push(a);
                output_lab.push(oklab(xyz(self.output, b)));
            } else {
                filtered += 1;
            }
        }
        let cmax = input_lab
            .iter()
            .map(|p| p[1].hypot(p[2]))
            .fold(0., f64::max);
        let mut summary = BTreeMap::<String, Value>::new();
        let mut worst = 0f64;
        for (lo, hi) in [(0.20, 0.35), (0.35, 0.50), (0.50, 0.70), (0.70, 1.001)] {
            let values: Vec<_> = input_lab
                .iter()
                .zip(&output_lab)
                .filter(|(p, _)| {
                    let c = p[1].hypot(p[2]);
                    c >= lo * cmax && c < hi * cmax
                })
                .map(|(&a, &b)| hue_delta(a, b))
                .collect();
            if !values.is_empty() {
                let m = max(&values);
                worst = worst.max(m);
                summary.insert(format!("max_rotation_chroma_{lo:.2}_{hi:.2}"), json!(m));
            }
        }
        summary.insert("max_hue_rotation_deg".into(), json!(worst));
        summary.insert("samples_in_locus".into(), json!(input_lab.len()));
        summary.insert("samples_filtered_out_of_locus".into(), json!(filtered));
        let r = result(
            "hue_twist_oklab",
            serde_json::to_value(summary).unwrap(),
            None,
            "degrees",
            "Stock-specific hue rotations in four chroma bands, excluding out-of-locus OkLab inputs.",
        );
        let pts: Vec<_> = output_lab
            .iter()
            .map(|p| [0.5 + p[1], 0.5 + p[2]])
            .collect();
        let figure = plot(self.root, &r.name, &pts, &vec![[0.4, 0.8, 1.]; pts.len()]);
        with_figure(r, figure)
    }

    fn spectral_locus_envelope(&self) -> QaResult {
        let mut valid_xy = Vec::new();
        let mut colors = Vec::new();
        let mut near = 0;
        let mut in_gamut = 0;
        for &p in &self.lut.table {
            let v = xyz(self.output, p);
            let q = xy(v);
            if v[1] > 1e-4 && q.iter().all(|v| v.is_finite()) {
                if inside(q, &color(self.output).pri) {
                    in_gamut += 1;
                }
                if self
                    .locus
                    .iter()
                    .map(|a| (q[0] - a[0]).hypot(q[1] - a[1]))
                    .fold(f64::INFINITY, f64::min)
                    < 0.02
                {
                    near += 1;
                }
                valid_xy.push(q);
                colors.push(display(p, self.output));
            }
        }
        let valid = valid_xy.len();
        let r = result(
            "spectral_locus_envelope",
            json!({"cube_resolution":self.n,"cube_cells":self.n.pow(3),"valid_cells":valid,"inside_output_gamut_fraction":in_gamut as f64/valid.max(1) as f64,"near_locus_fraction":near as f64/valid.max(1) as f64}),
            None,
            "",
            "Full cube chromaticity footprint with near-black cells excluded.",
        );
        let figure = plot(self.root, &r.name, &valid_xy, &colors);
        with_figure(r, figure)
    }
}
