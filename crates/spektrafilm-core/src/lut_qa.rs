//! Offline deterministic QA for the pinned Python 0.3.4 LUT suite.
//! References use the scientific runtime, never a freshly baked comparison LUT.
use crate::{
    gamut_compression::OutputGamutCompress,
    lut_baker::{Bundle, Lut, REFERENCE_COMMIT, Topology},
    lut_transport::{self, ColorSpaceEntry},
    neutral_filters::NeutralFilters,
    params::Tap,
    profile,
    runtime::Runtime,
};
use nalgebra::Matrix3;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::{
    colorspace::chromatic_adaptation_matrix_cat16_f64,
    image::ImageBuf,
    precision::{from_f64, to_f64},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
mod color;
mod report;
mod stimulus;
use color::{Color, color};
pub use report::write_report;
use stimulus::Pcg;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaResult {
    pub name: String,
    pub summary: BTreeMap<String, Value>,
    pub reference_values: BTreeMap<String, String>,
    pub passed: Option<bool>,
    pub units: String,
    pub interpretation: String,
    pub figure_path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrintReport {
    pub print_index: usize,
    pub print_name: String,
    pub folder: String,
    pub results: Vec<QaResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaReport {
    pub bundle_name: String,
    pub reference_commit: String,
    pub backend: String,
    pub precision: String,
    pub input_color_space: String,
    pub output_color_space: String,
    pub topology: String,
    pub resolution: usize,
    pub rng_seed: u64,
    pub offgrid_samples: usize,
    pub passed: bool,
    pub prints: Vec<PrintReport>,
}
fn result(
    name: &str,
    summary: Value,
    passed: Option<bool>,
    units: &str,
    interpretation: &str,
) -> QaResult {
    QaResult {
        name: name.into(),
        summary: serde_json::from_value(summary).expect("metric object"),
        reference_values: BTreeMap::new(),
        passed,
        units: units.into(),
        interpretation: interpretation.into(),
        figure_path: None,
    }
}
fn limits(r: &mut QaResult, entries: &[(&str, &str)]) {
    for (k, v) in entries {
        r.reference_values.insert((*k).into(), (*v).into());
    }
}
fn failed_scenario(name: &str, error: String) -> QaResult {
    result(
        name,
        json!({"error": error}),
        Some(false),
        "",
        "Scenario execution failed; subsequent scenarios continue.",
    )
}
fn with_figure(mut scenario: QaResult, figure: Result<String, String>) -> QaResult {
    match figure {
        Ok(path) => scenario.figure_path = Some(path),
        Err(error) => return failed_scenario(&scenario.name, error),
    }
    scenario
}

/// Run every pinned scenario for selected print(s), writing offline JSON, Markdown,
/// HTML and PNG assets. `qa_print_index=None` evaluates every recorded print.
pub fn run(
    bundle: &Bundle,
    data_dir: &Path,
    backend: &dyn ComputeBackend,
    output_dir: &Path,
) -> Result<QaReport, String> {
    bundle.spec.validate()?;
    if bundle.meta.stocks.prints != bundle.spec.print_profiles
        || bundle.meta.topology != bundle.spec.topology
    {
        return Err("QA bundle metadata disagrees with source spec".into());
    }
    let input = lut_transport::resolve(&bundle.spec.input_color_space)?;
    let output = lut_transport::resolve(&bundle.spec.output_color_space)?;
    for (role, entry) in [("input", input), ("output", output)] {
        let recorded = bundle
            .meta
            .color_spaces
            .get(role)
            .ok_or_else(|| format!("missing QA {role} color-space metadata"))?;
        if recorded.name != entry.name || recorded.cctf != entry.cctf.is_some() {
            return Err(format!(
                "QA {role} color-space metadata disagrees with transport"
            ));
        }
    }
    let stops = bundle
        .spec
        .stops_above_midgray
        .ok_or("QA spec stops_above_midgray was not normalized")?;
    let expected_gain = if bundle.spec.uses_native_input_gain() {
        input.native_input_gain()
    } else {
        input.input_gain_for_stops(stops)
    };
    if bundle.meta.input_exposure.as_ref().is_some_and(|m| {
        !m.gain.is_finite()
            || m.stops_above_midgray != stops
            || (m.gain - expected_gain).abs() > expected_gain.abs().max(1e-12) * 1e-12
    }) {
        return Err("QA input-exposure metadata disagrees with stops_above_midgray".into());
    }
    let indices: Vec<usize> = bundle.spec.qa_print_index.map_or_else(
        || (0..bundle.meta.stocks.prints.len()).collect(),
        |i| vec![i],
    );
    let mut report = QaReport {
        bundle_name: bundle.meta.name.clone(),
        reference_commit: REFERENCE_COMMIT.into(),
        backend: backend.name().into(),
        precision: if cfg!(feature = "precision-f64") {
            "f64"
        } else {
            "f32"
        }
        .into(),
        input_color_space: input.name.into(),
        output_color_space: output.name.into(),
        topology: bundle.meta.topology.name().into(),
        resolution: bundle.meta.resolution,
        rng_seed: 20260515,
        offgrid_samples: 50_000,
        passed: true,
        prints: Vec::new(),
    };
    for index in indices {
        let print = bundle
            .meta
            .stocks
            .prints
            .get(index)
            .ok_or("QA print index out of range")?;
        let lut = effective_lut(bundle, print)?;
        if lut.resolution < 3 {
            return Err("QA Jacobian and characteristic metrics require resolution >= 3".into());
        }
        let pipeline = make_pipeline(bundle, print, input, output, data_dir, false)?;
        let unbounded = make_pipeline(bundle, print, input, output, data_dir, true)?;
        let folder = format!(
            "{}_{}",
            crate::lut_baker::normalize_stock(&bundle.spec.film_profile),
            crate::lut_baker::normalize_stock(print)
        );
        let root = output_dir.join(&folder);
        std::fs::create_dir_all(root.join("figures")).map_err(|e| e.to_string())?;
        let mut results = run_print(
            bundle, &lut, input, output, &pipeline, &unbounded, backend, data_dir, &root,
        )?;
        // A required scenario with missing/nonfinite metrics is a failure, including
        // otherwise informational diagnostics. Do not let null metrics imply PASS.
        for r in &mut results {
            if r.summary.values().any(|v| v.is_null()) {
                r.passed = Some(false);
                r.interpretation
                    .push_str(" Nonfinite metric: required measurement failed.");
            }
        }
        report.passed &= results.iter().all(|r| r.passed != Some(false));
        report.prints.push(PrintReport {
            print_index: index,
            print_name: print.clone(),
            folder,
            results,
        });
    }
    write_report(&report, output_dir)?;
    Ok(report)
}

fn effective_lut(bundle: &Bundle, print: &str) -> Result<Lut, String> {
    let roles: &[(&str, Option<&str>)] = match bundle.meta.topology {
        Topology::One => &[("combined", Some(print))],
        Topology::Two => &[("film", None), ("print", Some(print))],
        Topology::Three => &[
            ("filming_expose", None),
            ("filming_develop", None),
            ("printing_combined", Some(print)),
        ],
        Topology::Four => &[
            ("filming_expose", None),
            ("filming_develop", None),
            ("printing_expose", Some(print)),
            ("printing_develop_scan", Some(print)),
        ],
    };
    let mut chain = Vec::new();
    for &(role, stock) in roles {
        let candidates: Vec<_> = bundle
            .meta
            .luts
            .iter()
            .filter(|m| m.role == role && m.print_profile.as_deref() == stock)
            .collect();
        if candidates.len() != 1 {
            return Err(format!("expected one canonical LUT {role} for {stock:?}"));
        }
        let m = candidates[0];
        let lut = bundle
            .luts
            .iter()
            .find(|(path, _)| *path == m.path)
            .map(|(_, l)| l)
            .ok_or_else(|| format!("missing QA LUT {}", m.path))?;
        if lut.resolution < 2
            || lut.table.len() != lut.resolution.checked_pow(3).ok_or("LUT size overflow")?
            || lut.table.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(format!("malformed QA LUT {}", m.path));
        }
        let taps = bundle.meta.topology.taps();
        let stage = chain.len();
        let domain = if stage == 0 {
            "input_rgb"
        } else {
            taps[stage].name()
        };
        let range = if stage + 1 == roles.len() {
            "output_rgb"
        } else {
            taps[stage + 1].name()
        };
        if m.domain != domain || m.range != range {
            return Err(format!(
                "malformed QA wire {}: expected {domain} -> {range}",
                m.path
            ));
        }
        if stage > 0 {
            let tap = taps[stage];
            let a = bundle.meta.wires.decode(tap, [0.; 3])?;
            let b = bundle.meta.wires.decode(tap, [1.; 3])?;
            if a.iter()
                .zip(b)
                .any(|(a, b)| !a.is_finite() || !b.is_finite() || b <= *a)
            {
                return Err(format!("invalid QA wire {}", tap.name()));
            }
        }
        chain.push(lut);
    }
    if chain.len() == 1 {
        return Ok(chain[0].clone());
    }
    let n = bundle.meta.resolution;
    let table = grid(n)
        .into_iter()
        .map(|mut p| {
            for l in &chain {
                p = l.sample(p);
            }
            p
        })
        .collect();
    Ok(Lut {
        resolution: n,
        table,
        title: format!("{} canonical chain", print),
    })
}
fn grid(n: usize) -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(n.pow(3));
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                out.push([r, g, b].map(|v| v as f64 / (n - 1) as f64));
            }
        }
    }
    out
}
fn tetra(lut: &Lut, p: [f64; 3]) -> [f64; 3] {
    let n = lut.resolution;
    let p = p.map(|v| v.clamp(0., 1.) * (n - 1) as f64);
    let lo = p.map(|v| (v.floor() as usize).min(n - 2));
    let f: [f64; 3] = std::array::from_fn(|i| p[i] - lo[i] as f64);
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| f[b].total_cmp(&f[a]));
    let mut vertex = lo;
    let mut out = lut.at(vertex[0], vertex[1], vertex[2]);
    for &axis in &order {
        let prev = lut.at(vertex[0], vertex[1], vertex[2]);
        vertex[axis] += 1;
        let next = lut.at(vertex[0], vertex[1], vertex[2]);
        for c in 0..3 {
            out[c] += f[axis] * (next[c] - prev[c]);
        }
    }
    out
}
fn make_pipeline(
    bundle: &Bundle,
    print: &str,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    data: &Path,
    unbounded: bool,
) -> Result<Runtime, String> {
    let film = profile::load_profile_by_name(data, &bundle.spec.film_profile)
        .map_err(|e| e.to_string())?;
    let paper = profile::load_profile_by_name(data, print).map_err(|e| e.to_string())?;
    let mut params = if let Some(params) = bundle.baked_params.get(print) {
        params.clone()
    } else if let Some(snapshot) = bundle
        .meta
        .params_snapshot
        .get(print)
        .filter(|v| v.get("camera").is_some())
    {
        let mut runtime = snapshot.clone();
        if let Some(fields) = runtime.as_object_mut() {
            for key in [
                "film",
                "print",
                "digest_changes",
                "stops_above_midgray",
                "exposure_ev",
                "input_gain",
            ] {
                fields.remove(key);
            }
        }
        serde_json::from_value(runtime)
            .map_err(|e| format!("invalid baked params snapshot: {e}"))?
    } else {
        let neutral = NeutralFilters::load(data)?;
        crate::lut_baker::bake_params(
            &bundle.spec,
            input,
            output,
            &film,
            &paper,
            &neutral,
            None,
            true,
        )?
    };
    if unbounded {
        params.io.output_gamut_compress.algorithm = "off".into();
    }
    params.validate()?;
    Runtime::new(film, paper, params, data)
}
fn process_linear(
    pipe: &Runtime,
    samples: &[[f64; 3]],
    backend: &dyn ComputeBackend,
) -> Result<Vec<[f64; 3]>, String> {
    let data = samples
        .iter()
        .flatten()
        .map(|&v| from_f64(v as f32 as f64))
        .collect();
    let out = pipe.process_with_taps(
        ImageBuf::from_data(samples.len() as u32, 1, data),
        backend,
        Some(Tap::RgbIn),
        Some(Tap::RgbOut),
    )?;
    let rows: Vec<_> = out
        .data
        .chunks_exact(3)
        .map(|p| [to_f64(p[0]), to_f64(p[1]), to_f64(p[2])])
        .collect();
    if rows.iter().flatten().any(|x| !x.is_finite()) {
        return Err("QA scientific reference produced nonfinite output".into());
    }
    Ok(rows)
}
fn reference(
    pipe: &Runtime,
    samples: &[[f64; 3]],
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    gain: f64,
    backend: &dyn ComputeBackend,
) -> Result<Vec<[f64; 3]>, String> {
    let linear: Vec<_> = samples
        .iter()
        .map(|&p| input.decode_rgb(p).map(|v| v * gain))
        .collect();
    Ok(process_linear(pipe, &linear, backend)?
        .into_iter()
        .map(|p| {
            output
                .encode_rgb(p.map(|v| v * output.output_gain()))
                .map(|v| v.clamp(0., 1.))
        })
        .collect())
}

fn mul(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|j| m[i][j] * v[j]).sum())
}
fn xyz(cs: &ColorSpaceEntry, rgb: [f64; 3]) -> [f64; 3] {
    mul(
        color(cs).m,
        cs.decode_rgb(rgb).map(|v| v / cs.output_gain()),
    )
}
fn oklab(v: [f64; 3]) -> [f64; 3] {
    mul(
        [
            [0.2104542553, 0.7936177850, -0.0040720468],
            [1.9779984951, -2.4285922050, 0.4505937099],
            [0.0259040371, 0.7827717662, -0.8086757660],
        ],
        mul(
            [
                [0.8189330101, 0.3618667424, -0.1288597137],
                [0.0329845436, 0.9293118715, 0.0361456387],
                [0.0482003018, 0.2643662691, 0.6338517070],
            ],
            v,
        )
        .map(f64::cbrt),
    )
}
fn okxyz(v: [f64; 3]) -> [f64; 3] {
    mul(
        [
            [1.2270138511035211, -0.5577999806518222, 0.2812561489664678],
            [-0.0405801784232806, 1.1122568696168302, -0.0716766786656012],
            [-0.0763812845057069, -0.4214819784180127, 1.586163220440795],
        ],
        mul(
            [
                [0.9999999984505196, 0.3963377921737678, 0.21580375806075877],
                [
                    1.0000000088817607,
                    -0.10556134232365633,
                    -0.0638541747717059,
                ],
                [
                    1.0000000546724108,
                    -0.08948418209496574,
                    -1.2914855378640917,
                ],
            ],
            v,
        )
        .map(|v| v * v * v),
    )
}
fn lab(xyz: [f64; 3]) -> [f64; 3] {
    let white = [0.3127 / 0.329, 1., (1. - 0.3127 - 0.329) / 0.329];
    let f: [f64; 3] = std::array::from_fn(|i| {
        let t = xyz[i] / white[i];
        if t > (6f64 / 29.).powi(3) {
            t.cbrt()
        } else {
            t / (3. * (6f64 / 29.).powi(2)) + 4. / 29.
        }
    });
    [
        116. * f[1] - 16.,
        500. * (f[0] - f[1]),
        200. * (f[1] - f[2]),
    ]
}
/// CIE 142:2001 CIEDE2000, unit weights.
pub fn delta_e_2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    let [l1, a1, b1] = a;
    let [l2, a2, b2] = b;
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.;
    let g = 0.5 * (1. - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let ap1 = (1. + g) * a1;
    let ap2 = (1. + g) * a2;
    let cp1 = ap1.hypot(b1);
    let cp2 = ap2.hypot(b2);
    let h1 = b1.atan2(ap1).to_degrees().rem_euclid(360.);
    let h2 = b2.atan2(ap2).to_degrees().rem_euclid(360.);
    let dl = l2 - l1;
    let dc = cp2 - cp1;
    let mut dh = h2 - h1;
    if cp1 * cp2 == 0. {
        dh = 0.;
    } else if dh > 180. {
        dh -= 360.;
    } else if dh < -180. {
        dh += 360.;
    }
    let d_h = 2. * (cp1 * cp2).sqrt() * (dh / 2.).to_radians().sin();
    let lm = (l1 + l2) / 2.;
    let cm = (cp1 + cp2) / 2.;
    let hm = if cp1 * cp2 == 0. {
        h1 + h2
    } else if (h1 - h2).abs() <= 180. {
        (h1 + h2) / 2.
    } else if h1 + h2 < 360. {
        (h1 + h2 + 360.) / 2.
    } else {
        (h1 + h2 - 360.) / 2.
    };
    let t = 1. - 0.17 * (hm - 30.).to_radians().cos()
        + 0.24 * (2. * hm).to_radians().cos()
        + 0.32 * (3. * hm + 6.).to_radians().cos()
        - 0.20 * (4. * hm - 63.).to_radians().cos();
    let sl = 1. + 0.015 * (lm - 50.).powi(2) / (20. + (lm - 50.).powi(2)).sqrt();
    let sc = 1. + 0.045 * cm;
    let sh = 1. + 0.015 * cm * t;
    let rt = -2.
        * (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt()
        * (60. * (-((hm - 275.) / 25.).powi(2)).exp())
            .to_radians()
            .sin();
    let x = dl / sl;
    let y = dc / sc;
    let z = d_h / sh;
    (x * x + y * y + z * z + rt * y * z).max(0.).sqrt()
}
fn ictcp(xyz: [f64; 3]) -> [f64; 3] {
    let rgb = mul(
        [
            [1.716651187971268, -0.355670783776392, -0.253366281373660],
            [-0.666684351832489, 1.616481236634939, 0.015768545813911],
            [0.017639857445311, -0.042770613257809, 0.942103121235474],
        ],
        xyz,
    );
    let lms = mul(
        [
            [1688. / 4096., 2146. / 4096., 262. / 4096.],
            [683. / 4096., 2951. / 4096., 462. / 4096.],
            [99. / 4096., 309. / 4096., 3688. / 4096.],
        ],
        rgb,
    )
    .map(|v| {
        let y = (v.abs() / 10000.).powf(2610. / 16384.);
        ((3424. / 4096. + 2413. / 128. * y) / (1. + 2392. / 128. * y)).powf(2523. / 32.)
    });
    mul(
        [
            [0.5, 0.5, 0.],
            [6610. / 4096., -13613. / 4096., 7003. / 4096.],
            [17933. / 4096., -17390. / 4096., -543. / 4096.],
        ],
        lms,
    )
}
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}
fn percentile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut a = v.to_vec();
    a.sort_by(f64::total_cmp);
    let x = (a.len() - 1) as f64 * q;
    let i = x.floor() as usize;
    a[i] + (a[x.ceil() as usize] - a[i]) * (x - i as f64)
}
fn max(v: &[f64]) -> f64 {
    v.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}
fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}
fn singular(j: [[f64; 3]; 3]) -> [f64; 3] {
    let m = Matrix3::from_fn(|r, c| j[r][c]);
    let mut s: [f64; 3] = m.svd(false, false).singular_values.into();
    s.sort_by(|a, b| b.total_cmp(a));
    s
}
fn hue_delta(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((b[2].atan2(b[1]) - a[2].atan2(a[1])).to_degrees() + 180.)
        .rem_euclid(360.)
        .sub(180.)
        .abs()
}
use std::ops::Sub;
fn xy(v: [f64; 3]) -> [f64; 2] {
    let s = v.iter().sum::<f64>();
    if s == 0. {
        [0., 0.]
    } else {
        [v[0] / s, v[1] / s]
    }
}
fn adapt(rgb: [f64; 3], from: Color, to: Color) -> [f64; 3] {
    let w = |v: [f64; 2]| [v[0] / v[1], 1., (1. - v[0] - v[1]) / v[1]];
    mul(
        to.inv,
        mul(
            chromatic_adaptation_matrix_cat16_f64(w(from.white), w(to.white)),
            mul(from.m, rgb),
        ),
    )
}
fn display(rgb: [f64; 3], cs: &ColorSpaceEntry) -> [f64; 3] {
    let s = lut_transport::resolve("sRGB").expect("sRGB");
    s.encode_rgb(
        adapt(
            cs.decode_rgb(rgb).map(|v| v / cs.output_gain()),
            color(cs),
            color(s),
        )
        .map(|v| v.clamp(0., 1.)),
    )
    .map(|v| v.clamp(0., 1.))
}
fn png(
    root: &Path,
    name: &str,
    width: usize,
    height: usize,
    pixels: &[[f64; 3]],
) -> Result<String, String> {
    if pixels.len() != width * height {
        return Err("QA figure dimensions mismatch".into());
    }
    let mut img = image::RgbImage::new(width as u32, height as u32);
    for (i, p) in pixels.iter().enumerate() {
        img.put_pixel(
            (i % width) as u32,
            (i / width) as u32,
            image::Rgb(p.map(|v| (v.clamp(0., 1.) * 255.).round() as u8)),
        );
    }
    let path = format!("figures/{name}.png");
    img.save(root.join(&path)).map_err(|e| e.to_string())?;
    Ok(path)
}
fn plot(
    root: &Path,
    name: &str,
    points: &[[f64; 2]],
    colors: &[[f64; 3]],
) -> Result<String, String> {
    let n = 640;
    let mut pixels = vec![[0.04; 3]; n * n];
    for (p, c) in points.iter().zip(colors) {
        let x = (p[0] * (n - 1) as f64).round() as isize;
        let y = ((1. - p[1]) * (n - 1) as f64).round() as isize;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let xx = x + dx;
                let yy = y + dy;
                if xx >= 0 && yy >= 0 && xx < n as isize && yy < n as isize {
                    pixels[yy as usize * n + xx as usize] = *c;
                }
            }
        }
    }
    png(root, name, n, n, &pixels)
}

fn hull_indices(pop: usize) -> Vec<usize> {
    let size = pop.min(8000);
    let mut rng = Pcg::seeded(0);
    let mut ids;
    if pop > 10000 && size > pop / 50 {
        ids = (0..pop).collect::<Vec<_>>();
        for i in (std::cmp::max(pop - size, 1)..pop).rev() {
            let j = rng.bounded(i);
            ids.swap(i, j);
        }
        ids = ids.split_off(pop - size);
    } else {
        ids = Vec::with_capacity(size);
        let mut set = BTreeSet::new();
        for j in pop - size..pop {
            let val = rng.bounded(j);
            let value = if set.contains(&val) { j } else { val };
            set.insert(value);
            ids.push(value);
        }
        for i in (1..size).rev() {
            let j = rng.bounded(i);
            ids.swap(i, j);
        }
    }
    ids
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn diff(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn volume(points: &[[f64; 3]]) -> Result<f64, String> {
    if points.len() < 4 {
        return Err("gamut hull needs four noncoplanar points".into());
    }
    let a = 0;
    let b = (1..points.len())
        .max_by(|&i, &j| distance(points[i], points[a]).total_cmp(&distance(points[j], points[a])))
        .unwrap();
    let c = (0..points.len())
        .max_by(|&i, &j| {
            dot(
                cross(diff(points[b], points[a]), diff(points[i], points[a])),
                cross(diff(points[b], points[a]), diff(points[i], points[a])),
            )
            .total_cmp(&dot(
                cross(diff(points[b], points[a]), diff(points[j], points[a])),
                cross(diff(points[b], points[a]), diff(points[j], points[a])),
            ))
        })
        .unwrap();
    let normal = cross(diff(points[b], points[a]), diff(points[c], points[a]));
    let d = (0..points.len())
        .max_by(|&i, &j| {
            dot(normal, diff(points[i], points[a]))
                .abs()
                .total_cmp(&dot(normal, diff(points[j], points[a])).abs())
        })
        .unwrap();
    if dot(normal, diff(points[d], points[a])).abs() < 1e-14 {
        return Err("degenerate gamut hull (collapsed output)".into());
    }
    let center =
        std::array::from_fn(|i| (points[a][i] + points[b][i] + points[c][i] + points[d][i]) / 4.);
    let orient = |mut f: [usize; 3]| {
        if dot(
            cross(
                diff(points[f[1]], points[f[0]]),
                diff(points[f[2]], points[f[0]]),
            ),
            diff(center, points[f[0]]),
        ) > 0.
        {
            f.swap(1, 2);
        }
        f
    };
    let mut faces = vec![
        orient([a, b, c]),
        orient([a, d, b]),
        orient([a, c, d]),
        orient([b, d, c]),
    ];
    for (i, p) in points.iter().enumerate() {
        if [a, b, c, d].contains(&i) {
            continue;
        }
        let mut edges: BTreeMap<(usize, usize), (usize, usize)> = BTreeMap::new();
        faces.retain(|f| {
            let n = cross(
                diff(points[f[1]], points[f[0]]),
                diff(points[f[2]], points[f[0]]),
            );
            let visible = dot(n, diff(*p, points[f[0]])) > 1e-12 * dot(n, n).sqrt();
            if visible {
                for (u, v) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                    let k = (u.min(v), u.max(v));
                    if edges.remove(&k).is_none() {
                        edges.insert(k, (u, v));
                    }
                }
            }
            !visible
        });
        for (_, (u, v)) in edges {
            faces.push(orient([u, v, i]));
        }
    }
    Ok(faces
        .iter()
        .map(|f| {
            dot(
                diff(points[f[0]], center),
                cross(diff(points[f[1]], center), diff(points[f[2]], center)),
            ) / 6.
        })
        .sum::<f64>()
        .abs())
}
fn locus() -> Vec<[f64; 2]> {
    crate::input_gamut::spectral_locus_xy()
}
fn inside(p: [f64; 2], polygon: &[[f64; 2]]) -> bool {
    let mut yes = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[j];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            yes = !yes;
        }
        j = i;
    }
    yes
}

fn bundle_input_gain(bundle: &Bundle, input: &ColorSpaceEntry) -> Result<f64, String> {
    let stops = bundle
        .spec
        .stops_above_midgray
        .ok_or("QA spec stops_above_midgray was not normalized")?;
    Ok(if bundle.spec.uses_native_input_gain() {
        input.native_input_gain()
    } else {
        input.input_gain_for_stops(stops)
    })
}
#[allow(clippy::too_many_arguments)]
fn run_print(
    bundle: &Bundle,
    lut: &Lut,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    pipeline: &Runtime,
    unbounded: &Runtime,
    backend: &dyn ComputeBackend,
    data: &Path,
    root: &Path,
) -> Result<Vec<QaResult>, String> {
    let n = lut.resolution;
    let gain = bundle_input_gain(bundle, input)?;
    let inputs = grid(n);
    let mut results = Vec::new();
    let mut rng = Pcg::seeded(20260515);
    let samples: Vec<_> = (0..50_000)
        .map(|_| std::array::from_fn(|_| rng.uniform() as f32 as f64))
        .collect();
    match reference(pipeline, &samples, input, output, gain, backend) {
        Ok(truth) => {
            let mut tri = Vec::with_capacity(samples.len());
            let mut tet = Vec::with_capacity(samples.len());
            let mut itp = Vec::with_capacity(samples.len());
            for (&p, &t) in samples.iter().zip(&truth) {
                let a = xyz(output, lut.sample(p));
                let b = xyz(output, t);
                tri.push(delta_e_2000(lab(a), lab(b)));
                tet.push(delta_e_2000(lab(xyz(output, tetra(lut, p))), lab(b)));
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
            let points: Vec<_> = samples.iter().map(|p| [p[0], p[1]]).collect();
            let colors: Vec<_> = tri
                .iter()
                .map(|v| [(v / 2.).clamp(0., 1.), 0.2, 1. - (v / 2.).clamp(0., 1.)])
                .collect();
            let figure = plot(root, &r.name, &points, &colors);
            results.push(with_figure(r, figure));
        }
        Err(error) => results.push(failed_scenario("off_grid_identity", error)),
    }
    let mut violations = 0;
    let mut worst = 0f64;
    let mut tv = [0.; 3];
    let mut counts = [0usize; 3];
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let p = [r, g, b];
                for a in 0..3 {
                    if p[a] + 1 < n {
                        let mut q = p;
                        q[a] += 1;
                        let x = lut.at(r, g, b);
                        let y = lut.at(q[0], q[1], q[2]);
                        let d = y[a] - x[a];
                        if d < 0. {
                            violations += 1;
                            worst = worst.min(d);
                        }
                        for c in 0..3 {
                            tv[a] += (y[c] - x[c]).abs();
                            counts[a] += 1;
                        }
                    }
                }
            }
        }
    }
    let pin = input.encode(0.18 / gain);
    let mut curves = Vec::new();
    let mut center_violations = 0;
    for axis in 0..3 {
        let mut previous = None;
        for i in 0..65 {
            let t = i as f64 / 64.;
            let mut p = [pin; 3];
            p[axis] = t;
            let y = lut.sample(p);
            if previous.is_some_and(|v| y[axis] < v) {
                center_violations += 1;
            }
            previous = Some(y[axis]);
            curves.push([t, 1. - y[axis]]);
        }
    }
    let mut r = result(
        "monotonicity",
        json!({"violations":violations,"worst_negative_diff":worst,"centerline_pin_encoded":pin,"centerline_violations":center_violations}),
        Some(violations == 0),
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
    let figure = plot(root, &r.name, &curves, &vec![[0.7, 0.8, 0.9]; curves.len()]);
    results.push(with_figure(r, figure));
    let mut cond = Vec::new();
    for r in 1..n - 1 {
        for g in 1..n - 1 {
            for b in 1..n - 1 {
                let derivatives = [
                    diff(lut.at(r + 1, g, b), lut.at(r - 1, g, b)),
                    diff(lut.at(r, g + 1, b), lut.at(r, g - 1, b)),
                    diff(lut.at(r, g, b + 1), lut.at(r, g, b - 1)),
                ];
                let s = singular(std::array::from_fn(|i| {
                    std::array::from_fn(|j| derivatives[j][i])
                }));
                cond.push((s[0] / s[2].max(1e-12)).max(1.).log10());
            }
        }
    }
    let mut r = result(
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
    let figure = png(root, &r.name, (n - 2) * (n - 2), n - 2, &pix);
    results.push(with_figure(r, figure));
    for a in 0..3 {
        tv[a] /= counts[a] as f64;
    }
    let fft = fft_ratios(lut);
    let mut r = result(
        "total_variation",
        json!({"tv":tv.iter().sum::<f64>(),"tv_r":tv[0],"tv_g":tv[1],"tv_b":tv[2],"axial_highband_ratio_r":fft[2],"axial_highband_ratio_g":fft[1],"axial_highband_ratio_b":fft[0],"axial_highband_ratio_mean":mean(&fft)}),
        None,
        "",
        "Mean absolute cube finite differences and upper-half axial FFT magnitude energy (Python axis-label convention).",
    );
    let figure = png(
        root,
        &r.name,
        n * n,
        n,
        &lut.table
            .iter()
            .map(|&p| display(p, output))
            .collect::<Vec<_>>(),
    );
    results.push(with_figure(r, figure));
    let gamut_result = (|| -> Result<QaResult, String> {
        let flips = face_flips(lut);
        let indices: Vec<_> = hull_indices(inputs.len())
            .into_iter()
            .map(|i| {
                let b = i / (n * n);
                let g = i / n % n;
                let r = i % n;
                (r * n + g) * n + b
            })
            .collect();
        let projected_in: Vec<_> = indices
            .iter()
            .map(|&i| oklab(xyz(output, inputs[i])))
            .collect();
        let projected_out: Vec<_> = indices
            .iter()
            .map(|&i| oklab(xyz(output, lut.table[i])))
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
        let linear: Vec<_> = rim.iter().map(|&p| input.decode_rgb(p)).collect();
        let before = process_linear(unbounded, &linear, backend)?;
        let compressor =
            OutputGamutCompress::build(&bundle.spec.output_gamut_compress, output.primaries)?;
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
                    displacement.push(distance(a, b));
                }
            }
        }
        let mut r = result(
            "output_gamut_compression",
            json!({"fold_triangles":flips.0,"fold_fraction":flips.0 as f64/flips.1 as f64,"input_hull_volume":in_volume,"output_hull_volume":out_volume.as_ref().ok(),"compression_ratio":ratio,"compression_algorithm":bundle.spec.output_gamut_compress.algorithm,"rim_oog_fraction":oog as f64/bright.max(1) as f64,"rim_oog_samples":oog,"rim_max_displacement":if displacement.is_empty(){0.}else{max(&displacement)},"rim_mean_displacement":if displacement.is_empty(){0.}else{mean(&displacement)}}),
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
            .map(|&p| xy(mul(color(output).m, p)))
            .collect();
        let colors = vec![[1., 0.4, 0.2]; before.len()]
            .into_iter()
            .chain(vec![[0.2, 0.7, 1.]; after.len()])
            .collect::<Vec<_>>();
        r.figure_path = Some(plot(root, &r.name, &pts, &colors)?);
        Ok(r)
    })();
    results.push(
        gamut_result.unwrap_or_else(|error| failed_scenario("output_gamut_compression", error)),
    );
    let neutral: Vec<_> = (0..n).map(|i| lut.at(i, i, i)).collect();
    let density: Vec<_> = neutral
        .iter()
        .map(|p| p.map(|v| -v.clamp(1e-4, 1.).log10()))
        .collect();
    let spread = density
        .iter()
        .map(|p| max(p) - p.iter().copied().fold(f64::INFINITY, f64::min))
        .fold(0., f64::max);
    let mid = n / 2;
    let axis = |i: usize| 1e-6 + (1. - 1e-6) * i as f64 / (n - 1) as f64;
    let gamma = (mean(&neutral[mid + 1]).clamp(1e-4, 1.).log10()
        - mean(&neutral[mid - 1]).clamp(1e-4, 1.).log10())
        / (axis(mid + 1).log10() - axis(mid - 1).log10());
    let mut r = result(
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
            (t, -mean(&lut.sample([t; 3])).clamp(1e-4, 1.).log10())
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
                let y = lut.sample(p);
                for c in 0..3 {
                    pts.push([x, 1. + y[c].clamp(1e-4, 1.).log10() / 4.]);
                    colors.push(match c {
                        0 => [1., 0.2, 0.2],
                        1 => [0.2, 1., 0.2],
                        _ => [0.2, 0.2, 1.],
                    });
                }
            }
        }
    }
    let figure = plot(root, &r.name, &pts, &colors);
    results.push(with_figure(r, figure));
    match dynamic_range(lut, input, output, root) {
        Ok(r) => results.push(r),
        Err(error) => results.push(failed_scenario("dynamic_range_usage", error)),
    };
    let mut planck = Vec::new();
    for i in 0..16 {
        let t = 2700. + 7300. * i as f64 / 15.;
        let (x, y) = cct_xy(t);
        let rgb = mul(color(input).inv, [x / y, 1., (1. - x - y) / y]);
        let peak = max(&rgb).max(1e-6);
        planck.push(
            input
                .encode_rgb(rgb.map(|v| (v / peak).clamp(0., 1.) / gain))
                .map(|v| v as f32 as f64),
        );
    }
    let outxy: Vec<_> = planck
        .iter()
        .map(|&p| xy(xyz(output, lut.sample(p))))
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
    let figure = plot(root, &r.name, &outxy, &vec![[1., 0.8, 0.3]; 16]);
    results.push(with_figure(r, figure));
    let mut input_lab = Vec::new();
    let mut output_lab = Vec::new();
    let mut filtered = 0;
    for (&a, &b) in inputs.iter().zip(&lut.table) {
        let a = oklab(xyz(input, a));
        if a[1].hypot(a[2]) <= 0.6 {
            input_lab.push(a);
            output_lab.push(oklab(xyz(output, b)));
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
    let mut r = result(
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
    let figure = plot(root, &r.name, &pts, &vec![[0.4, 0.8, 1.]; pts.len()]);
    results.push(with_figure(r, figure));
    let locus = locus();
    let mut valid_xy = Vec::new();
    let mut colors = Vec::new();
    let mut near = 0;
    let mut in_gamut = 0;
    for &p in &lut.table {
        let v = xyz(output, p);
        let q = xy(v);
        if v[1] > 1e-4 && q.iter().all(|v| v.is_finite()) {
            if inside(q, &color(output).pri) {
                in_gamut += 1;
            }
            if locus
                .iter()
                .map(|a| (q[0] - a[0]).hypot(q[1] - a[1]))
                .fold(f64::INFINITY, f64::min)
                < 0.02
            {
                near += 1;
            }
            valid_xy.push(q);
            colors.push(display(p, output));
        }
    }
    let valid = valid_xy.len();
    let mut r = result(
        "spectral_locus_envelope",
        json!({"cube_resolution":n,"cube_cells":n.pow(3),"valid_cells":valid,"inside_output_gamut_fraction":in_gamut as f64/valid.max(1) as f64,"near_locus_fraction":near as f64/valid.max(1) as f64}),
        None,
        "",
        "Full cube chromaticity footprint with near-black cells excluded.",
    );
    let figure = plot(root, &r.name, &valid_xy, &colors);
    results.push(with_figure(r, figure));
    results.extend(input_compression(
        bundle, &inputs, input, data, root, &locus,
    )?);
    results.extend(picture(
        bundle, lut, input, output, pipeline, backend, root,
    )?);
    Ok(results)
}
fn fft_ratios(lut: &Lut) -> [f64; 3] {
    use rustfft::{FftPlanner, num_complex::Complex};
    let n = lut.resolution;
    let fft = FftPlanner::<f64>::new().plan_fft_forward(n);
    let mut line = vec![Complex::new(0., 0.); n];
    let mut ratios = [0.; 3];
    for a in 0..3 {
        let mut total = 0f64;
        let mut high = 0f64;
        for u in 0..n {
            for v in 0..n {
                for c in 0..3 {
                    for i in 0..n {
                        let mut p = [0; 3];
                        p[a] = i;
                        p[(a + 1) % 3] = u;
                        p[(a + 2) % 3] = v;
                        line[i] = Complex::new(lut.at(p[0], p[1], p[2])[c], 0.);
                    }
                    fft.process(&mut line);
                    for (i, x) in line[..n / 2 + 1].iter().enumerate() {
                        total += x.norm();
                        if i >= std::cmp::max(1, n / 4) {
                            high += x.norm();
                        }
                    }
                }
            }
        }
        ratios[a] = high / total.max(1e-12);
    }
    ratios
}
fn face_flips(lut: &Lut) -> (usize, usize) {
    let n = lut.resolution;
    let mut flips = 0;
    let mut total = 0;
    for fixed in 0..3 {
        for edge in [0, n - 1] {
            let mut neg = 0;
            let mut pos = 0;
            let free: Vec<_> = (0..3).filter(|&a| a != fixed).collect();
            let keep: Vec<_> = (0..3).filter(|&c| c != 2 - fixed).collect();
            for u in 0..n - 1 {
                for v in 0..n - 1 {
                    let mut p = [0; 3];
                    p[fixed] = edge;
                    p[free[0]] = u;
                    p[free[1]] = v;
                    let x = lut.at(p[0], p[1], p[2]);
                    p[free[0]] += 1;
                    let du = diff(lut.at(p[0], p[1], p[2]), x);
                    p[free[0]] -= 1;
                    p[free[1]] += 1;
                    let dv = diff(lut.at(p[0], p[1], p[2]), x);
                    let det = du[keep[0]] * dv[keep[1]] - du[keep[1]] * dv[keep[0]];
                    if det < 0. {
                        neg += 1;
                    } else if det > 0. {
                        pos += 1;
                    }
                    total += 1;
                }
            }
            flips += neg.min(pos);
        }
    }
    (flips, total)
}
fn interp_pairs(p: &[(f64, f64)], x: f64) -> f64 {
    if x <= p[0].0 {
        return p[0].1;
    }
    for v in p.windows(2) {
        if x < v[1].0 {
            return v[0].1 + (v[1].1 - v[0].1) * (x - v[0].0) / (v[1].0 - v[0].0);
        }
    }
    p[p.len() - 1].1
}
fn dynamic_range(
    lut: &Lut,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    root: &Path,
) -> Result<QaResult, String> {
    let ceiling = (0..3201)
        .map(|i| -12. + 0.01 * i as f64)
        .filter(|&s| input.encode(input.midgray_linear * s.exp2()) <= 1.)
        .last()
        .ok_or("input encoding clips every dynamic-range probe")?;
    let hi = ceiling + 0.5;
    let step = (hi + 8.) / 256.;
    let stops: Vec<_> = (0..257).map(|i| -8. + i as f64 * step).collect();
    let mut y = Vec::new();
    let mut ok = Vec::new();
    for &s in &stops {
        let e = input.encode(input.midgray_linear * s.exp2());
        ok.push((0.0..=1.).contains(&e));
        let e = e.clamp(0., 1.) as f32 as f64;
        y.push(xyz(output, lut.sample([e; 3]))[1].max(1e-6));
    }
    let lo = ok.iter().position(|&v| v).unwrap_or(0);
    let hi = ok.iter().rposition(|&v| v).unwrap_or(0);
    let active: Vec<_> = y
        .windows(2)
        .map(|v| (v[1].max(1e-4).log10() - v[0].max(1e-4).log10()).abs() / step > 0.10)
        .collect();
    let count = active[lo..hi].iter().filter(|&&v| v).count();
    let toe = active[lo..hi].iter().take_while(|&&v| !v).count();
    let shoulder = active[lo..hi].iter().rev().take_while(|&&v| !v).count();
    let mid = interp_pairs(
        &stops
            .iter()
            .copied()
            .zip(y.iter().copied())
            .collect::<Vec<_>>(),
        0.,
    );
    let mut r = result(
        "dynamic_range_usage",
        json!({"encoded_range_stops":stops[hi]-stops[lo],"active_range_stops":count as f64*step,"toe_collapsed_stops":toe as f64*step,"shoulder_collapsed_stops":shoulder as f64*step,"midgray_output_y":mid,"midgray_offset_stops":(mid/0.18).log2()}),
        None,
        "stops",
        "Native camera middle gray anchors the ramp; active segments exceed 0.10 density per stop.",
    );
    let pts: Vec<_> = stops
        .iter()
        .zip(y)
        .map(|(&s, y)| [(s + 8.) / (ceiling + 8.5), 1. + y.max(1e-4).log10() / 4.])
        .collect();
    r.figure_path = Some(plot(
        root,
        &r.name,
        &pts,
        &vec![[0.9, 0.8, 0.4]; pts.len()],
    )?);
    Ok(r)
}
fn cct_xy(t: f64) -> (f64, f64) {
    if t >= 4000. {
        let x = if t <= 7000. {
            -4.607e9 / t.powi(3) + 2.9678e6 / t.powi(2) + 99.11 / t + 0.244063
        } else {
            -2.0064e9 / t.powi(3) + 1.9018e6 / t.powi(2) + 247.48 / t + 0.23704
        };
        (x, -3. * x * x + 2.87 * x - 0.275)
    } else {
        let x = -0.2661239e9 / t.powi(3) - 0.2343580e6 / t.powi(2) + 877.6956 / t + 0.179910;
        let y = if t <= 2222. {
            -1.1063814 * x.powi(3) - 1.34811020 * x * x + 2.18555832 * x - 0.20219683
        } else {
            -0.9549476 * x.powi(3) - 1.37418593 * x * x + 2.09137015 * x - 0.16748867
        };
        (x, y)
    }
}
fn input_compression(
    bundle: &Bundle,
    inputs: &[[f64; 3]],
    input: &ColorSpaceEntry,
    data: &Path,
    root: &Path,
    locus: &[[f64; 2]],
) -> Result<Vec<QaResult>, String> {
    let film = profile::load_profile_by_name(data, &bundle.spec.film_profile)
        .map_err(|e| e.to_string())?;
    let illuminant = film.info.reference_illuminant;
    let white = match illuminant.as_str() {
        "D50" => [0.3457, 0.3585],
        "D55" => [0.33243, 0.34744],
        "D65" => [0.3127, 0.329],
        "D75" => [0.29902, 0.31485],
        "A" => [0.44758, 0.40745],
        other => return Err(format!("QA cannot resolve film illuminant {other}")),
    };
    let source = color(input);
    let w = |p: [f64; 2]| [p[0] / p[1], 1., (1. - p[0] - p[1]) / p[1]];
    let cat = chromatic_adaptation_matrix_cat16_f64(w(source.white), w(white));
    let spec = &bundle.spec.input_gamut_compress;
    let compressor = match crate::input_gamut::InputGamutCompress::build(spec) {
        Ok(compressor) => compressor,
        Err(error) => {
            return Ok(vec![
                failed_scenario("input_gamut_compression_preview", error.clone()),
                failed_scenario("input_gamut_compression_smoothness", error),
            ]);
        }
    };
    let active = compressor.is_active();
    let mut oog = 0;
    let mut bright = 0;
    let mut valid = 0;
    let mut pts = Vec::new();
    let mut colors = Vec::new();
    for &p in inputs {
        let v = mul(cat, mul(source.m, input.decode_rgb(p)));
        let b = v.iter().sum::<f64>();
        let q = if b > 1e-12 {
            [v[0] / b, v[1] / b]
        } else {
            [v[0], v[1]]
        };
        let in_locus = inside(q, locus);
        if b > 1e-4 {
            valid += 1;
            if !in_locus {
                oog += 1;
                if b > 1e-2 {
                    bright += 1;
                }
            }
            pts.push(q);
            colors.push(if in_locus {
                [0.4, 0.8, 0.6]
            } else {
                [1., 0.4, 0.4]
            });
            if active && b > 1e-2 {
                let compressed = compressor.compress_xy(q, white);
                if !in_locus || (compressed[0] - q[0]).hypot(compressed[1] - q[1]) > 1e-3 {
                    pts.push(compressed);
                    colors.push([0.4, 0.8, 1.]);
                }
            }
        }
    }
    let mut r = result(
        "input_gamut_compression_preview",
        json!({"active":active,"algorithm":spec.algorithm,"knee_threshold":spec.knee[0],"knee_limit":spec.knee[1],"knee_power":spec.knee[2],"oog_fraction":oog as f64/valid.max(1) as f64,"n_oog_samples":oog,"n_oog_bright":bright,"reference_illuminant":illuminant}),
        None,
        "",
        "Input cube projected with CAT16 into the film reference-illuminant frame; red outside the locus, cyan compressed.",
    );
    let figure = plot(root, &r.name, &pts, &colors);
    let r = with_figure(r, figure);
    let ring: Vec<_> = (0..720)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 720.;
            compressor.compress_xy(
                [white[0] + 0.30 * a.cos(), white[1] + 0.30 * a.sin()],
                white,
            )
        })
        .collect();
    let lengths: Vec<_> = ring
        .windows(2)
        .map(|p| (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]))
        .collect();
    let med = percentile(&lengths, 0.5);
    let mut smooth = result(
        "input_gamut_compression_smoothness",
        json!({"active":active,"algorithm":spec.algorithm,"knee_threshold":spec.knee[0],"knee_limit":spec.knee[1],"knee_power":spec.knee[2],"probe_radius":0.30,"probe_samples":720,"worst_step":max(&lengths),"median_step":med,"worst_over_median_step":max(&lengths)/med.max(1e-9),"reference_illuminant":illuminant}),
        None,
        "",
        "Circumferential 720-point radius-0.30 probe; worst/median adjacent step detects compression discontinuities.",
    );
    let figure = plot(root, &smooth.name, &ring, &vec![[0.4, 0.8, 1.]; 720]);
    let smooth = with_figure(smooth, figure);
    Ok(vec![r, smooth])
}
fn gamut_chroma(cs: &ColorSpaceEntry, l: f64, h: f64) -> f64 {
    let ok = |c: f64| {
        let rgb = mul(color(cs).inv, okxyz([l, c * h.cos(), c * h.sin()]));
        rgb.iter().all(|v| (0.0..=1.).contains(v))
    };
    let mut lo = 0.;
    let mut hi = 0.45;
    for _ in 0..3 {
        if ok(hi) {
            hi *= 1.5;
        } else {
            break;
        }
    }
    for _ in 0..18 {
        let c = (lo + hi) / 2.;
        if ok(c) {
            lo = c;
        } else {
            hi = c;
        }
    }
    lo
}
#[allow(clippy::too_many_arguments)]
fn picture(
    bundle: &Bundle,
    lut: &Lut,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    pipeline: &Runtime,
    backend: &dyn ComputeBackend,
    root: &Path,
) -> Result<Vec<QaResult>, String> {
    let gain = bundle_input_gain(bundle, input)?;
    let l = 0.5646225971435698;
    let mut samples = Vec::new();
    for (c, h) in std::iter::once((0., 0.)).chain(
        [0.07, 0.14, 0.21]
            .into_iter()
            .flat_map(|c| (0..16).map(move |i| (c, std::f64::consts::TAU * i as f64 / 16.))),
    ) {
        let rgb = mul(color(input).inv, okxyz([l, c * h.cos(), c * h.sin()]));
        if rgb.iter().all(|v| (0.0..=1.).contains(v)) {
            let enc = input.encode_rgb(rgb.map(|v| v / gain));
            if enc.iter().all(|v| *v > 0.006 && *v < 0.994) {
                samples.push(enc);
            }
        }
    }
    let sensitivity = (|| -> Result<QaResult, String> {
        if samples.is_empty() {
            return Err("required noise-sensitivity stimulus has no in-gamut samples".into());
        }
        let mut sigma = Vec::new();
        let mut anisotropy = Vec::new();
        let mut worst_hue = 0.;
        let mut rotation = 0.;
        for &p in &samples {
            let mut j = [[0.; 3]; 3];
            for a in 0..3 {
                let mut plus = p;
                let mut minus = p;
                plus[a] += 0.005;
                minus[a] -= 0.005;
                let d = diff(
                    oklab(xyz(output, lut.sample(plus))),
                    oklab(xyz(output, lut.sample(minus))),
                );
                for c in 0..3 {
                    j[c][a] = d[c] / 0.01;
                }
            }
            let s = singular(j);
            sigma.push(s[0]);
            anisotropy.push(s[0] / s[2].max(1e-9));
            let a = oklab(xyz(input, p));
            let b = oklab(xyz(output, lut.sample(p)));
            if a[1].hypot(a[2]) > 1e-3 {
                let d = hue_delta(a, b);
                if d > rotation {
                    rotation = d;
                    worst_hue = a[2].atan2(a[1]).to_degrees();
                }
            }
        }
        let mut r = result(
            "noise_sensitivity",
            json!({"L_slice":l,"sigma_in_encoded":0.005,"n_input_samples":samples.len(),"max_sigma1":max(&sigma),"p99_sigma1":percentile(&sigma,0.99),"p50_sigma1":percentile(&sigma,0.5),"max_anisotropy":max(&anisotropy),"p99_anisotropy":percentile(&anisotropy,0.99),"max_hue_rotation_deg":rotation,"worst_hue_deg":worst_hue,"ellipse_display_scale":1.5}),
            None,
            "OkLab per encoded-RGB",
            "Encoded-input central-difference Jacobian singular values quantify isotropic noise amplification at polar OkLab stimuli.",
        );
        r.figure_path = Some(noise_figure(root, lut, input, output, &samples, gain, l)?);
        Ok(r)
    })();
    let mut results =
        vec![sensitivity.unwrap_or_else(|error| failed_scenario("noise_sensitivity", error))];
    let side = 512;
    let mut clean = Vec::with_capacity(side * side);
    let mut in_gamut = 0;
    for row in 0..side {
        let v = row as f64 / (side - 1) as f64;
        let light = if v <= 0.5 {
            v / 0.5 * l
        } else {
            l + (v - 0.5) / 0.5 * (1. - l)
        };
        let weight = 1. - (2. * v - 1.).abs();
        for col in 0..side {
            let h = std::f64::consts::TAU * col as f64 / side as f64;
            let c = 0.96 * weight * gamut_chroma(input, light, h);
            let rgb = mul(color(input).inv, okxyz([light, c * h.cos(), c * h.sin()]));
            if rgb.iter().all(|v| (0.0..=1.).contains(v)) {
                in_gamut += 1;
            }
            clean.push(input.encode_rgb(rgb.map(|v| v.clamp(0., 1.) / gain)));
        }
    }
    let mut rng = Pcg::seeded(0);
    let mut differences = Vec::with_capacity(clean.len() * 3);
    let mut panels = vec![[0.; 3]; side * side * 2];
    for (i, &p) in clean.iter().enumerate() {
        let noisy = p.map(|v| (v + 0.02 * rng.normal()).clamp(0., 1.));
        let a = lut.sample(p);
        let b = lut.sample(noisy);
        differences.extend((0..3).map(|c| (a[c] - b[c]).abs()));
        let row = i / side;
        let col = i % side;
        panels[row * side * 2 + col] = display(a, output);
        panels[row * side * 2 + side + col] = display(b, output);
    }
    let mut r = result(
        "noise_gradient",
        json!({"image_side":side,"sigma_in_encoded":0.02,"mid_L":l,"rim_scale":0.96,"input_in_gamut_fraction":in_gamut as f64/clean.len() as f64,"mean_abs_output_delta":mean(&differences),"p99_abs_output_delta":percentile(&differences,0.99)}),
        None,
        "encoded RGB",
        "Continuous 512² OkLab hue gradient, clean versus NumPy PCG64 seed-0 ziggurat encoded-input noise.",
    );
    let figure = png(root, &r.name, side * 2, side, &panels);
    results.push(with_figure(r, figure));
    let stress = (|| -> Result<QaResult, String> {
        let width = 768;
        let height = 256;
        let mut panels = vec![[0.; 3]; width * height * 3];
        let mut summary = BTreeMap::<String, Value>::new();
        let srgb = lut_transport::resolve("sRGB")?;
        for (panel, target) in ["Rec.709", "DCI-P3", "Rec.2020"].iter().enumerate() {
            let target_cs = lut_transport::resolve(target)?;
            let mut linear = Vec::with_capacity(width * height);
            let mut oog = 0;
            let mut band_count = 0;
            for row in 0..height {
                let v = row as f64 / (height - 1) as f64;
                let white = (1. - 2. * v).max(0.);
                let sat = 1. - (2. * v - 1.).abs();
                for col in 0..width {
                    let t = 6. * col as f64 / width as f64;
                    let f = t.fract();
                    let hue = match t as usize {
                        0 => [1., f, 0.],
                        1 => [1. - f, 1., 0.],
                        2 => [0., 1., f],
                        3 => [0., 1. - f, 1.],
                        4 => [f, 0., 1.],
                        _ => [1., 0., 1. - f],
                    };
                    let enc = hue.map(|c| white + sat * c);
                    let rgb = adapt(target_cs.decode_rgb(enc), color(target_cs), color(input));
                    if row >= height / 2 - height / 16 && row < height / 2 + height / 16 {
                        band_count += 1;
                        if rgb.iter().any(|v| *v < 0. || *v > 1.) {
                            oog += 1;
                        }
                    }
                    linear.push(rgb);
                }
            }
            let out = process_linear(pipeline, &linear, backend)?;
            for (i, p) in out.into_iter().enumerate() {
                let rgb = adapt(p, color(output), color(srgb));
                panels[(i / width) * width * 3 + panel * width + i % width] = srgb
                    .encode_rgb(rgb.map(|v| v.clamp(0., 1.)))
                    .map(|v| v.clamp(0., 1.));
            }
            summary.insert(
                format!("{target}_oog_fraction_saturated_row"),
                json!(oog as f64 / band_count as f64),
            );
        }
        let mut r = result(
            "output_gamut_edge_stress",
            serde_json::to_value(summary).unwrap(),
            None,
            "",
            "Rec.709, DCI-P3 and Rec.2020 encoded tent gradients CAT16-adapted without clipping and evaluated through the actual runtime.",
        );
        let figure = png(root, &r.name, width * 3, height, &panels);
        Ok(with_figure(r, figure))
    })();
    results.push(stress.unwrap_or_else(|error| failed_scenario("output_gamut_edge_stress", error)));
    let count = lut.resolution.min(9);
    let n = lut.resolution;
    let mut pixels = vec![[0.; 3]; n * n * count];
    for panel in 0..count {
        let b = (panel as f64 * (n - 1) as f64 / (count - 1).max(1) as f64).round() as usize;
        for row in 0..n {
            for col in 0..n {
                pixels[row * n * count + panel * n + col] =
                    display(lut.at(col, n - 1 - row, b), output);
            }
        }
    }
    let mut r = result(
        "rg_plane_slices",
        json!({"n_slices":count,"cube_resolution":n}),
        None,
        "",
        "Evenly spaced B-input slices; R increases left-to-right and G bottom-to-top, converted to sRGB for display.",
    );
    let figure = png(root, &r.name, n * count, n, &pixels);
    results.push(with_figure(r, figure));
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn perceptual_difference_matches_cie_reference() {
        assert!(
            (delta_e_2000([50., 2.6772, -79.7751], [50., 0., -82.7485]) - 2.0424596801565764).abs()
                < 1e-12
        );
        assert!(
            (delta_e_2000([50., 3.1571, -77.2803], [50., 0., -82.7485]) - 2.861510174747494).abs()
                < 1e-12
        );
    }
    #[test]
    fn early_figure_failure_preserves_later_measurements_and_reports() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let spec=serde_json::from_value(json!({"name":"scenario_isolation","film_profile":"kodak_portra_400","print_profiles":["kodak_portra_endura"],"input_color_space":"sRGB","output_color_space":"sRGB","resolution":3})).unwrap();
        let bundle = crate::lut_baker::BundleBuilder::new(spec)
            .build(&data, &backend)
            .unwrap();
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-qa-isolation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let folder = format!(
            "{}_{}",
            crate::lut_baker::normalize_stock(&bundle.spec.film_profile),
            crate::lut_baker::normalize_stock(&bundle.spec.print_profiles[0])
        );
        let figures = root.join(folder).join("figures");
        std::fs::create_dir_all(figures.join("off_grid_identity.png")).unwrap();
        let report = run(&bundle, &data, &backend, &root).unwrap();
        let rows = &report.prints[0].results;
        assert_eq!(rows[0].name, "off_grid_identity");
        assert_eq!(rows[0].passed, Some(false));
        assert!(rows[0].summary.contains_key("error"));
        let later = rows
            .iter()
            .find(|r| r.name == "dynamic_range_usage")
            .unwrap();
        assert!(later.summary["encoded_range_stops"].as_f64().unwrap() > 0.);
        assert!(
            root.join(&report.prints[0].folder)
                .join(later.figure_path.as_ref().unwrap())
                .is_file()
        );
        let delivered: QaReport =
            serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
        assert_eq!(delivered.prints[0].results.len(), 16);
        assert!(!delivered.passed);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hull_rejects_collapsed_gamut() {
        assert!(volume(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]]).is_err());
        assert!(
            (volume(&[
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., 0., 1.],
                [1., 1., 1.]
            ])
            .unwrap()
                - 0.5)
                .abs()
                < 1e-12
        );
    }
}

fn noise_figure(
    root: &Path,
    lut: &Lut,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    samples: &[[f64; 3]],
    gain: f64,
    l: f64,
) -> Result<String, String> {
    let mut extent = 0.30f64;
    for cs in [input, output] {
        for i in 0..181 {
            let h = std::f64::consts::TAU * i as f64 / 181.;
            let c = gamut_chroma(cs, l, h);
            extent = extent
                .max((c * h.cos()).abs() * 1.15)
                .max((c * h.sin()).abs() * 1.15);
        }
    }
    let size = 480;
    let width = size * 3;
    let mut pixels = vec![[0.04; 3]; width * size];
    let jacobian = |p: [f64; 3]| {
        let mut j = [[0.; 3]; 3];
        for a in 0..3 {
            let mut plus = p;
            let mut minus = p;
            plus[a] += 0.005;
            minus[a] -= 0.005;
            let d = diff(
                oklab(xyz(output, lut.sample(plus))),
                oklab(xyz(output, lut.sample(minus))),
            );
            for c in 0..3 {
                j[c][a] = d[c] / 0.01;
            }
        }
        j
    };
    let put = |pixels: &mut Vec<[f64; 3]>, panel: usize, a: f64, b: f64, c: [f64; 3]| {
        let x = ((a + extent) / (2. * extent) * (size - 1) as f64).round() as isize;
        let y = ((extent - b) / (2. * extent) * (size - 1) as f64).round() as isize;
        if x >= 0 && y >= 0 && x < size as isize && y < size as isize {
            pixels[y as usize * width + panel * size + x as usize] = c;
        }
    };
    for &p in samples {
        let out = lut.sample(p);
        let lab = oklab(xyz(output, out));
        let j = jacobian(p);
        let cov = |r: usize, c: usize| {
            (0..3)
                .map(|k| j[r][k] * j[c][k] * 0.005f64.powi(2))
                .sum::<f64>()
        };
        let a = cov(1, 1);
        let b = cov(1, 2);
        let d = cov(2, 2);
        let theta = 0.5 * (2. * b).atan2(a - d);
        let disc = ((a - d).powi(2) + 4. * b * b).sqrt();
        let major = ((a + d + disc) / 2.).max(0.).sqrt() * 3.;
        let minor = ((a + d - disc) / 2.).max(0.).sqrt() * 3.;
        for i in 0..360 {
            let angle = std::f64::consts::TAU * i as f64 / 360.;
            let u = major * angle.cos();
            let v = minor * angle.sin();
            put(
                &mut pixels,
                0,
                lab[1] + u * theta.cos() - v * theta.sin(),
                lab[2] + u * theta.sin() + v * theta.cos(),
                display(out, output),
            );
        }
    }
    let grid = 96;
    let mut heat = Vec::with_capacity(grid * grid);
    let mut maximum = 0f64;
    for row in 0..grid {
        for col in 0..grid {
            let a = -extent + 2. * extent * col as f64 / (grid - 1) as f64;
            let b = -extent + 2. * extent * row as f64 / (grid - 1) as f64;
            let rgb = mul(color(input).inv, okxyz([l, a, b]));
            if rgb.iter().all(|v| (0.0..=1.).contains(v)) {
                let p = input
                    .encode_rgb(rgb.map(|v| v / gain))
                    .map(|v| v.clamp(0.006, 0.994));
                let s = singular(jacobian(p))[0];
                maximum = maximum.max(s);
                heat.push(Some(s));
            } else {
                heat.push(None);
            }
        }
    }
    for row in 0..size {
        for col in 0..size {
            if let Some(s) = heat[(grid - 1 - row * grid / size) * grid + col * grid / size] {
                let t = (s / maximum.max(1e-12)).clamp(0., 1.);
                pixels[row * width + size + col] = [t, 0.2, 1. - t];
            }
        }
    }
    // The middle chroma ring's marginal luminance/chroma standard deviations,
    // in polar coordinates, show directional noise response around hue.
    let mut rosette = Vec::new();
    let mut scale = 0f64;
    for i in 0..16 {
        let h = std::f64::consts::TAU * i as f64 / 16.;
        let rgb = mul(color(input).inv, okxyz([l, 0.14 * h.cos(), 0.14 * h.sin()]));
        if rgb.iter().all(|v| (0.0..=1.).contains(v)) {
            let p = input.encode_rgb(rgb.map(|v| v / gain));
            if p.iter().all(|v| *v > 0.006 && *v < 0.994) {
                let j = jacobian(p);
                let sl = (j[0].iter().map(|v| v * v).sum::<f64>()).sqrt() * 0.005;
                let sab = (j[1].iter().chain(&j[2]).map(|v| v * v).sum::<f64>()).sqrt() * 0.005;
                scale = scale.max(sl).max(sab);
                rosette.push((h, sl, sab));
            }
        }
    }
    for (h, sl, sab) in rosette {
        for (s, c) in [(sl, [1., 0.8, 0.2]), (sab, [0.3, 0.8, 1.])] {
            let r = s / scale.max(1e-12) * extent * 0.9;
            for i in 0..300 {
                let t = i as f64 / 299.;
                put(&mut pixels, 2, r * t * h.cos(), r * t * h.sin(), c);
            }
        }
    }
    png(root, "noise_sensitivity", width, size, &pixels)
}
