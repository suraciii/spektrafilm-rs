//! Calibrated deterministic LUT bundles matching spektrafilm 0.3.4.
//! Tables have canonical `[r][g][b][channel]` order (blue varies fastest).
//! Transport encodings belong to this module; runtime stages receive linear RGB
//! or physical tap values. Format writers alone reorder lattice vertices.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::lut_transport::{self, ColorSpaceEntry};
use crate::neutral_filters::NeutralFilters;
use crate::params::{RuntimeParams, Tap};
use crate::profile;
use crate::runtime::{DigestMode, Runtime, digest_params_with_neutral};

mod bundle;
use bundle::effective_input_gain;
pub use bundle::{
    BoundaryWires, Bundle, BundleMeta, BundleSpec, ColorSpaceMeta, DensityWire, InputExposureMeta,
    LogEWire, Lut, LutFileMeta, StocksMeta, Topology, normalize_stock,
};

pub const REFERENCE_COMMIT: &str = "28bf883e1672e884307edc75852549376e13644e";

pub struct BundleBuilder {
    pub spec: BundleSpec,
    base_params: Option<RuntimeParams>,
}
impl BundleBuilder {
    pub fn new(spec: BundleSpec) -> Self {
        Self {
            spec,
            base_params: None,
        }
    }
    pub fn with_params(spec: BundleSpec, params: RuntimeParams) -> Self {
        Self {
            spec,
            base_params: Some(params),
        }
    }
    /// Construct once per print and evaluate each lattice as one ImageBuf.
    /// No spatial effects or stochastic grain; non-spatial DIR remains active.
    pub fn build(&self, data_dir: &Path, backend: &dyn ComputeBackend) -> Result<Bundle, String> {
        let mut spec = self.spec.clone();
        spec.normalize()?;
        let input = lut_transport::resolve(&spec.input_color_space)?;
        let output = lut_transport::resolve(&spec.output_color_space)?;
        let neutral = NeutralFilters::load(data_dir)?;
        let first = make_pipeline(
            &spec,
            &spec.print_profiles[0],
            input,
            output,
            data_dir,
            &neutral,
            self.base_params.as_ref(),
        )?;
        let wires = measure_wires(&first, &spec, input, backend)?;
        let mut luts = Vec::new();
        let mut baked_params = BTreeMap::new();
        let mut snapshots = BTreeMap::new();
        let mut digests = BTreeMap::new();
        let mut metas = Vec::new();
        let recipes = recipes(spec.topology, spec.include_combinations);
        for recipe in recipes.iter().filter(|r| r.shared) {
            let (path, lut, meta) =
                bake_recipe(recipe, None, &first, &spec, &wires, input, output, backend)?;
            luts.push((path, lut));
            metas.push(meta);
        }
        for (i, print) in spec.print_profiles.iter().enumerate() {
            let extra;
            let pipeline = if i == 0 {
                &first
            } else {
                extra = make_pipeline(
                    &spec,
                    print,
                    input,
                    output,
                    data_dir,
                    &neutral,
                    self.base_params.as_ref(),
                )?;
                &extra
            };
            let snapshot = bake_snapshot(
                &spec,
                print,
                pipeline,
                input,
                output,
                data_dir,
                &neutral,
                self.base_params.as_ref(),
            )?;
            let digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?)
            );
            snapshots.insert(print.clone(), snapshot);
            digests.insert(print.clone(), digest);
            baked_params.insert(print.clone(), pipeline.params().clone());
            for recipe in recipes.iter().filter(|r| !r.shared) {
                let (path, lut, meta) = bake_recipe(
                    recipe,
                    Some(print),
                    pipeline,
                    &spec,
                    &wires,
                    input,
                    output,
                    backend,
                )?;
                luts.push((path, lut));
                metas.push(meta);
            }
        }
        let meta = BundleMeta {
            schema_version: 3,
            name: spec.name.clone(),
            topology: spec.topology,
            resolution: spec.resolution,
            target: spec.target.clone(),
            provenance: BTreeMap::from([
                ("spektrafilm_version".into(), "0.3.4".into()),
                ("lut_creator_version".into(), "0.3.4".into()),
                ("reference_commit".into(), REFERENCE_COMMIT.into()),
                (
                    "project_url".into(),
                    "https://github.com/andreavolpato/spektrafilm".into(),
                ),
                (
                    "license".into(),
                    "spektrafilm LUT by Andrea Volpato, licensed under CC BY-SA 4.0.".into(),
                ),
                (
                    "citation".into(),
                    "Please cite the spektrafilm project and CITATION.cff.".into(),
                ),
            ]),
            stocks: StocksMeta {
                film: spec.film_profile.clone(),
                prints: spec.print_profiles.clone(),
            },
            color_spaces: BTreeMap::from([
                (
                    "input".into(),
                    ColorSpaceMeta {
                        name: input.name.into(),
                        cctf: input.cctf.is_some(),
                    },
                ),
                (
                    "output".into(),
                    ColorSpaceMeta {
                        name: output.name.into(),
                        cctf: output.cctf.is_some(),
                    },
                ),
            ]),
            wires,
            luts: metas,
            input_exposure: Some(InputExposureMeta {
                stops_above_midgray: spec.stops_above_midgray.unwrap(),
                exposure_ev: spec.exposure_ev,
                gain: effective_input_gain(&spec, input),
            }),
            params_snapshot: snapshots,
            params_digest: digests,
            artifacts: Vec::new(),
        };
        Ok(Bundle {
            spec,
            luts,
            meta,
            baked_params,
        })
    }
}

fn make_pipeline(
    spec: &BundleSpec,
    print_stock: &str,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    data_dir: &Path,
    neutral: &NeutralFilters,
    base_params: Option<&RuntimeParams>,
) -> Result<Runtime, String> {
    let film =
        profile::load_profile_by_name(data_dir, &spec.film_profile).map_err(|e| e.to_string())?;
    let print = profile::load_profile_by_name(data_dir, print_stock).map_err(|e| e.to_string())?;
    let params = bake_params(
        spec,
        input,
        output,
        &film,
        &print,
        neutral,
        base_params,
        true,
    )?;
    Runtime::new(film, print, params, data_dir)
}

pub(crate) fn bake_params(
    spec: &BundleSpec,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    film: &profile::Profile,
    print: &profile::Profile,
    neutral: &NeutralFilters,
    base_params: Option<&RuntimeParams>,
    lut_mode: bool,
) -> Result<RuntimeParams, String> {
    let mut params = base_params.cloned().unwrap_or_default();
    if base_params.is_none() {
        params.io.input_gamut_compress = spec.input_gamut_compress.clone();
        params.io.output_gamut_compress = spec.output_gamut_compress.clone();
    }
    params.debug.lut_mode = lut_mode;
    params.io.input_color_space = input.primaries.into();
    params.io.output_color_space = output.primaries.into();
    params.io.input_cctf_decoding = false;
    params.io.output_cctf_encoding = false;
    params.io.scan_film = false;
    params.workflow.route = "input > film > print > scan".into();
    params.taps.inject = None;
    params.taps.collect = None;
    params.settings.preview_mode = false;
    if params.io.input_gamut_compress.algorithm == "off" {
        params.io.input_gamut_compress.active = false;
    }
    params.validate()?;
    Ok(digest_params_with_neutral(
        params,
        film,
        print,
        neutral,
        DigestMode::ApplyStockSpecifics,
    ))
}

fn bake_snapshot(
    spec: &BundleSpec,
    print_stock: &str,
    pipeline: &Runtime,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    data_dir: &Path,
    neutral: &NeutralFilters,
    base_params: Option<&RuntimeParams>,
) -> Result<serde_json::Value, String> {
    let film =
        profile::load_profile_by_name(data_dir, &spec.film_profile).map_err(|e| e.to_string())?;
    let print = profile::load_profile_by_name(data_dir, print_stock).map_err(|e| e.to_string())?;
    let reference = bake_params(
        spec,
        input,
        output,
        &film,
        &print,
        neutral,
        base_params,
        false,
    )?;
    let before = serde_json::to_value(reference).map_err(|e| e.to_string())?;
    let mut snapshot = serde_json::to_value(pipeline.params()).map_err(|e| e.to_string())?;
    let mut changes = BTreeMap::new();
    collect_digest_changes("", &before, &snapshot, &mut changes);
    snapshot["film"] = serde_json::json!({"stock":film.info.stock,"version":film.metadata.version});
    snapshot["print"] =
        serde_json::json!({"stock":print.info.stock,"version":print.metadata.version});
    snapshot["digest_changes"] = serde_json::to_value(changes).map_err(|e| e.to_string())?;
    snapshot["stops_above_midgray"] = serde_json::json!(spec.stops_above_midgray);
    snapshot["exposure_ev"] = serde_json::json!(spec.exposure_ev);
    snapshot["input_gain"] = serde_json::json!(effective_input_gain(spec, input));
    Ok(snapshot)
}

fn collect_digest_changes(
    path: &str,
    before: &serde_json::Value,
    after: &serde_json::Value,
    changes: &mut BTreeMap<String, serde_json::Value>,
) {
    if let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
        for (key, value) in before {
            if let Some(next) = after.get(key) {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                collect_digest_changes(&child, value, next, changes);
            }
        }
    } else if before != after {
        changes.insert(path.into(), serde_json::json!({"from":before,"to":after}));
    }
}

/// Uniform code cube packed `[r][g][b]` into a (n*n)-wide, n-high image.
fn lattice_image(
    n: usize,
    inject: Tap,
    spec: &BundleSpec,
    wires: &BoundaryWires,
    input: &ColorSpaceEntry,
) -> Result<ImageBuf, String> {
    let mut data = Vec::with_capacity(n * n * n * 3);
    let gain = effective_input_gain(spec, input);
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let code = [r, g, b].map(|i| i as f64 / (n - 1) as f64);
                let physical = if inject == Tap::RgbIn {
                    input.decode_rgb(code).map(|v| v * gain)
                } else {
                    wires.decode(inject, code)?
                };
                // Python explicitly casts every injected lattice to float32, including
                // when the scientific stages subsequently evaluate in float64.
                data.extend(physical.map(|v| from_f64(v as f32 as f64)));
            }
        }
    }
    Ok(ImageBuf::from_data((n * n) as u32, n as u32, data))
}
fn measure_wires(
    pipeline: &Runtime,
    spec: &BundleSpec,
    input: &ColorSpaceEntry,
    backend: &dyn ComputeBackend,
) -> Result<BoundaryWires, String> {
    let mut wires = BoundaryWires::default();
    for &tap in spec
        .topology
        .taps()
        .iter()
        .skip(1)
        .filter(|&&t| t != Tap::RgbOut)
    {
        let image = lattice_image(9, Tap::RgbIn, spec, &wires, input)?;
        let raw = pipeline.process_with_taps(image, backend, Some(Tap::RgbIn), Some(tap))?;
        if raw.data.iter().any(|v| !to_f64(*v).is_finite()) {
            return Err(format!("nonfinite {} probe", tap.name()));
        }
        if tap == Tap::CmyFilm {
            let mut max = [f64::NEG_INFINITY; 3];
            for p in raw.data.chunks_exact(3) {
                for c in 0..3 {
                    max[c] = max[c].max(to_f64(p[c]));
                }
            }
            wires.cmy_film = Some(DensityWire {
                d_min: [-0.2; 3],
                d_max: max.map(|v| (v * 1.05 * 10000.0).ceil() / 10000.0),
            });
        } else {
            let lo = raw
                .data
                .iter()
                .map(|v| to_f64(*v))
                .fold(f64::INFINITY, f64::min);
            let hi = raw
                .data
                .iter()
                .map(|v| to_f64(*v))
                .fold(f64::NEG_INFINITY, f64::max);
            let wire = Some(LogEWire {
                min: ((lo - 0.1) * 10000.0).floor() / 10000.0,
                max: ((hi + 0.1) * 10000.0).ceil() / 10000.0,
            });
            if tap == Tap::LogEFilm {
                wires.log_e_film = wire;
            } else {
                wires.log_e_print = wire;
            }
        }
    }
    Ok(wires)
}
struct Recipe {
    inject: Tap,
    collect: Tap,
    role: String,
    suffix: Option<String>,
    shared: bool,
    combination: bool,
}
fn recipes(topology: Topology, combinations: bool) -> Vec<Recipe> {
    let taps = topology.taps();
    let mut result = Vec::new();
    for i in 0..taps.len() - 1 {
        let (role, suffix) = match topology {
            Topology::One => ("combined", None),
            Topology::Two => {
                if i == 0 {
                    ("film", Some("film".into()))
                } else {
                    ("print", Some("print".into()))
                }
            }
            Topology::Three | Topology::Four => {
                let role = match i {
                    0 => "filming_expose",
                    1 => "filming_develop",
                    2 if topology == Topology::Three => "printing_combined",
                    2 => "printing_expose",
                    _ => "printing_develop_scan",
                };
                (role, Some(format!("l{}", i + 1)))
            }
        };
        result.push(Recipe {
            inject: taps[i],
            collect: taps[i + 1],
            role: role.into(),
            suffix,
            shared: matches!(taps[i + 1], Tap::LogEFilm | Tap::CmyFilm),
            combination: false,
        });
    }
    if combinations {
        // Increasing span length matches pinned ordering: l12,l23,l34,l123,...
        for span in 2..taps.len() {
            for i in 0..taps.len() - span {
                let ids = (i + 1..=i + span)
                    .map(|id| id.to_string())
                    .collect::<String>();
                result.push(Recipe {
                    inject: taps[i],
                    collect: taps[i + span],
                    role: format!("subchain_{ids}"),
                    suffix: Some(format!("l{ids}")),
                    shared: matches!(taps[i + span], Tap::LogEFilm | Tap::CmyFilm),
                    combination: true,
                });
            }
        }
    }
    result
}
#[allow(clippy::too_many_arguments)]
fn bake_recipe(
    recipe: &Recipe,
    print: Option<&str>,
    pipeline: &Runtime,
    spec: &BundleSpec,
    wires: &BoundaryWires,
    input: &ColorSpaceEntry,
    output: &ColorSpaceEntry,
    backend: &dyn ComputeBackend,
) -> Result<(String, Lut, LutFileMeta), String> {
    let image = lattice_image(spec.resolution, recipe.inject, spec, wires, input)?;
    let raw =
        pipeline.process_with_taps(image, backend, Some(recipe.inject), Some(recipe.collect))?;
    let mut table = Vec::with_capacity(spec.resolution.pow(3));
    for pixel in raw.data.chunks_exact(3) {
        let physical = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
        let code = if recipe.collect == Tap::RgbOut {
            output.encode_rgb(physical.map(|v| v * output.output_gain()))
        } else {
            wires.encode(recipe.collect, physical)?
        };
        if code.iter().any(|v| !v.is_finite()) {
            return Err(format!("nonfinite {} LUT sample", recipe.role));
        }
        table.push(code.map(|v| v.clamp(0.0, 1.0)));
    }
    let mut title = format!("v034_{}", normalize_stock(&spec.film_profile));
    if let Some(print) = print {
        title.push('_');
        title.push_str(&normalize_stock(print));
    }
    if let Some(suffix) = &recipe.suffix {
        title.push('_');
        title.push_str(suffix);
    }
    let path = format!(
        "{}lut_{}.cube",
        if recipe.combination {
            "combinations/"
        } else {
            ""
        },
        title
    );
    let domain = if recipe.inject == Tap::RgbIn {
        "input_rgb"
    } else {
        recipe.inject.name()
    };
    let range = if recipe.collect == Tap::RgbOut {
        "output_rgb"
    } else {
        recipe.collect.name()
    };
    let meta = LutFileMeta {
        role: recipe.role.clone(),
        path: path.clone(),
        domain: domain.into(),
        range: range.into(),
        print_profile: print.map(str::to_owned),
    };
    Ok((
        path,
        Lut {
            resolution: spec.resolution,
            table,
            title,
        },
        meta,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(input: &str, output: &str) -> BundleSpec {
        serde_json::from_value(serde_json::json!({
            "film_profile":"kodak_portra_400", "print_profiles":["kodak_portra_endura"],
            "input_color_space":input, "output_color_space":output
        }))
        .unwrap()
    }

    #[test]
    #[cfg(feature = "precision-f64")]
    fn flog_bake_matches_independent_d65_compression_reference() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        assert_eq!(film.info.reference_illuminant, "D55");
        let mut s = spec("Fujifilm F-Log", "sRGB");
        s.resolution = 4;
        assert!(s.input_gamut_compress.active);
        let bundle = BundleBuilder::new(s)
            .build(&data, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        // Independent 28bf883 Python BundleBuilder, NumPy 2.5.3, float32
        // transport input; resolution 4, gain 1, default runtime gamut knees.
        // Saturated channels distinguish fixed-D65 compression from D55;
        // the previous D55 center missed these references by up to 0.09195.
        let reference = [
            (
                1,
                [0.25242497446445367, 0.0847493750709554, 0.19343996049115467],
            ),
            (
                4,
                [
                    0.0005628630644368622,
                    0.19641493988104003,
                    0.09760761143023325,
                ],
            ),
            (
                16,
                [0.014825877622419844, 0.078115288171892, 0.25334905318699014],
            ),
            (
                3,
                [0.8918221634211371, 0.4459024486440791, 0.7615401300303294],
            ),
            (
                12,
                [0.789960662997608, 0.8118796315037752, 0.18577316534685803],
            ),
            (
                48,
                [0.011540816177352134, 0.7547688090577326, 0.8396251588911985],
            ),
        ];
        for (index, expected) in reference {
            // Python table axes are [blue, green, red]; Rust is red-major.
            let rust_index = (index % 4) * 16 + ((index / 4) % 4) * 4 + index / 16;
            for (actual, expected) in bundle.luts[0].1.table[rust_index].into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() <= 2e-6,
                    "F-Log sample {index}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn stops_above_midgray_accepts_auto_native_and_explicit_values() {
        let mut auto = spec("srgb", "srgb");
        auto.normalize().unwrap();
        assert_eq!(auto.stops_above_midgray, Some(4.0));

        let mut native: BundleSpec = serde_json::from_value(serde_json::json!({
            "film_profile":"kodak_portra_400", "print_profiles":["kodak_portra_endura"],
            "input_color_space":"srgb", "output_color_space":"srgb",
            "stops_above_midgray":null
        }))
        .unwrap();
        native.normalize().unwrap();
        assert_eq!(native.stops_above_midgray, Some(0.0));
        assert!(native.uses_native_input_gain());

        let mut explicit: BundleSpec = serde_json::from_value(serde_json::json!({
            "film_profile":"kodak_portra_400", "print_profiles":["kodak_portra_endura"],
            "input_color_space":"srgb", "output_color_space":"srgb",
            "stops_above_midgray":2.5
        }))
        .unwrap();
        explicit.normalize().unwrap();
        assert_eq!(explicit.stops_above_midgray, Some(2.5));
        assert!(!explicit.uses_native_input_gain());
        let mut numeric_zero = spec("srgb", "srgb");
        numeric_zero.stops_above_midgray = Some(0.0);
        numeric_zero.normalize().unwrap();
        assert!(!numeric_zero.uses_native_input_gain());
        assert!(
            (effective_input_gain(&numeric_zero, lut_transport::resolve("srgb").unwrap()) - 0.18)
                .abs()
                < 1e-12
        );
        let input = lut_transport::resolve("srgb").unwrap();
        assert!((input.input_gain_for_stops(2.5) - 0.18 * 2.5f64.exp2()).abs() < 1e-12);
    }

    #[test]
    fn inactive_roles_fail_before_baking_and_slug_roles_match() {
        assert!(
            spec("ACEScg", "srgb")
                .normalize()
                .unwrap_err()
                .contains("input color space")
        );
        assert!(
            spec("srgb", "vlog")
                .normalize()
                .unwrap_err()
                .contains("output color space")
        );
        let mut s = spec("vlog", "rec2100pq");
        s.normalize().unwrap();
        assert_eq!(s.input_color_space, "Panasonic V-Log");
        assert_eq!(s.output_color_space, "Rec.2100 PQ");
        let input = lut_transport::resolve(&s.input_color_space).unwrap();
        assert!((input.input_gain(s.exposure_ev) - 1.0).abs() < 1e-12);
        assert_eq!(
            lut_transport::resolve("rec2100pq").unwrap().output_gain(),
            100.0 / 0.18
        );
        let mut sdr = spec("srgb", "srgb");
        sdr.normalize().unwrap();
        assert_eq!(sdr.exposure_ev, 0.0);
        assert_eq!(
            lut_transport::resolve("srgb")
                .unwrap()
                .input_gain(sdr.exposure_ev),
            1.0
        );
    }

    #[test]
    fn input_exposure_preserves_native_midgray_and_applies_deliberate_ev() {
        for name in ["srgb", "vlog", "acescct", "rec2100pq", "rec2100hlg"] {
            let input = lut_transport::resolve(name).unwrap();
            for ev in [-2.0_f64, 0.0, 1.0] {
                let got = input.midgray_linear * input.input_gain(ev);
                assert!(
                    (got - 0.18 * ev.exp2()).abs() < 1e-14,
                    "{name} at {ev} EV: {got}"
                );
            }
        }
    }

    #[test]
    fn params_first_bake_preserves_look_and_discloses_neutralized_trims() {
        let film: profile::Profile =
            serde_json::from_str(r#"{"metadata":{},"info":{},"data":{}}"#).unwrap();
        let print = film.clone();
        let neutral = NeutralFilters::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let s = spec("srgb", "srgb");
        let input = lut_transport::resolve("srgb").unwrap();
        let mut base = RuntimeParams::default();
        base.io.input_gamut_compress.algorithm = "off".into();
        base.io.output_gamut_compress.algorithm = "off".into();
        base.workflow.route = "input".into();
        base.io.scan_film = true;
        base.settings.preview_mode = true;
        base.taps.collect = Some("cmy_film".into());
        base.enlarger.y_filter_shift = 12.0;
        base.camera.exposure_compensation_ev = 1.5;
        base.io.crop = true;
        base.film_render.density_curve_gamma = 1.3;
        let baked =
            bake_params(&s, input, input, &film, &print, &neutral, Some(&base), true).unwrap();
        let reference = bake_params(
            &s,
            input,
            input,
            &film,
            &print,
            &neutral,
            Some(&base),
            false,
        )
        .unwrap();
        assert_eq!(baked.io.input_gamut_compress.algorithm, "off");
        assert_eq!(baked.io.output_gamut_compress.algorithm, "off");
        assert_eq!(baked.film_render.density_curve_gamma, 1.3);
        assert_eq!(baked.workflow.route, "input > film > print > scan");
        assert!(!baked.io.scan_film);
        assert!(!baked.settings.preview_mode);
        assert!(baked.taps.collect.is_none());
        let mut changes = BTreeMap::new();
        collect_digest_changes(
            "",
            &serde_json::to_value(reference).unwrap(),
            &serde_json::to_value(baked).unwrap(),
            &mut changes,
        );
        assert_eq!(
            changes["enlarger.y_filter_shift"],
            serde_json::json!({"from":12.0,"to":0.0})
        );
        assert_eq!(
            changes["camera.exposure_compensation_ev"],
            serde_json::json!({"from":1.5,"to":0.0})
        );
        assert_eq!(
            changes["io.crop"],
            serde_json::json!({"from":true,"to":false})
        );
        assert_eq!(base.enlarger.y_filter_shift, 12.0);
    }

    #[test]
    fn interpolation_preserves_channel_axis_and_clamps_domain_edges() {
        let mut table = Vec::new();
        for r in 0..3 {
            for g in 0..3 {
                for b in 0..3 {
                    table.push([r as f64 / 2.0, g as f64 / 2.0, b as f64 / 2.0]);
                }
            }
        }
        let lut = Lut {
            resolution: 3,
            table,
            title: "identity".into(),
        };
        let p = lut.sample([0.13, 0.71, 0.37]);
        for (actual, expected) in p.into_iter().zip([0.13, 0.71, 0.37]) {
            assert!((actual - expected).abs() < 1e-14);
        }
        assert_eq!(lut.sample([-1.0, 1.0, 2.0]), [0.0, 1.0, 1.0]);
    }

    #[test]
    fn density_wire_preserves_negative_fog_and_channel_specific_ranges() {
        let wires = BoundaryWires {
            cmy_film: Some(DensityWire {
                d_min: [-0.2; 3],
                d_max: [1.0, 2.0, 3.0],
            }),
            ..Default::default()
        };
        let physical = [-0.1, 1.1, 2.9];
        let code = wires.encode(Tap::CmyFilm, physical).unwrap();
        let decoded = wires.decode(Tap::CmyFilm, code).unwrap();
        for (actual, expected) in decoded.into_iter().zip(physical) {
            assert!((actual - expected).abs() < 1e-14);
        }
        assert_eq!(
            wires.encode(Tap::CmyFilm, [-1.0, 3.0, 4.0]).unwrap(),
            [0.0, 1.0, 1.0]
        );
    }
}
