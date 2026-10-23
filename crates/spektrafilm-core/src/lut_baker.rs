//! Calibrated deterministic LUT bundles matching spektrafilm 0.3.4.
//! Tables have canonical `[r][g][b][channel]` order (blue varies fastest).
//! Transport encodings belong to this module; runtime stages receive linear RGB
//! or physical tap values. Format writers alone reorder lattice vertices.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::WorkflowRoute;
use crate::lut_transport::{self, ColorSpaceEntry};
use crate::neutral_filters::NeutralFilters;
use crate::params::{RuntimeParams, Tap};
use crate::profile;
use crate::runtime::{DigestMode, Runtime, digest_params_with_neutral};

mod bundle;
use bundle::Stage;
pub(crate) use bundle::effective_input_gain;
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
        if let Some(params) = self.base_params.as_ref() {
            if params.output_mix.amount < 1.0 && spec.topology != Topology::One {
                return Err(
                    "output_mix.amount below 1 requires 1lut: segmented LUTs have no prepared source branch"
                        .into(),
                );
            }
            validate_route_topology(spec.topology, params.workflow.route.as_str())?;
        }

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
        for recipe in recipes.iter().filter(|r| r.stage.shared) {
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
            for recipe in recipes.iter().filter(|r| !r.stage.shared) {
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
        let cmyk_ink_active = first.params().cmyk.active && first.params().cmyk.strength > 0.0;
        let mut provenance = BTreeMap::from([
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
        ]);
        if cmyk_ink_active {
            provenance.insert(
                "cmyk_ink_v1".into(),
                "included in the baked RGB final stage".into(),
            );
            provenance.insert("cmyk_ink_v1.style".into(), "ink".into());
            provenance.insert("cmyk_ink_v1.model".into(), "cmyk_ink_v1".into());
            provenance.insert(
                "cmyk_ink_v1.strength".into(),
                first.params().cmyk.strength.to_string(),
            );
            provenance.insert(
                "cmyk_ink_v1.output_color_space".into(),
                output.primaries.to_string(),
            );
            provenance.insert("cmyk_ink_v1.transfer_state".into(), "baked".into());
            provenance.insert("cmyk_ink_v1.included".into(), "true".into());
        }
        provenance.insert(
            "output_mix.amount".into(),
            first.params().output_mix.amount.to_string(),
        );
        provenance.insert("model_version".into(), crate::runtime::MODEL_VERSION.into());
        let meta = BundleMeta {
            schema_version: 4,
            name: spec.name.clone(),
            topology: spec.topology,
            resolution: spec.resolution,
            target: spec.target.clone(),
            workflow_route: first.params().workflow.route.clone(),
            provenance,
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
fn validate_route_topology(topology: Topology, route: &str) -> Result<(), String> {
    if topology == Topology::Four && WorkflowRoute::parse(route) == WorkflowRoute::FilmScan {
        return Err(
            "4lut is not supported with direct film-scan routes: the topology has no print taps"
                .into(),
        );
    }
    if topology != Topology::One && WorkflowRoute::parse(route).is_convert_film() {
        return Err(
            "convert-film routes require 1lut because split LUT topologies do not expose conversion taps"
                .into(),
        );
    }
    Ok(())
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
    let (film, print) = resolve_profiles(&spec.film_profile, print_stock, data_dir)?;
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

/// Resolve film and print profiles from stock names. Shared by
/// `make_pipeline` (lut_baker) and `make_pipeline` (lut_qa).
pub(crate) fn resolve_profiles(
    film_profile: &str,
    print_stock: &str,
    data_dir: &Path,
) -> Result<(profile::Profile, profile::Profile), String> {
    let film = profile::load_profile_by_name(data_dir, film_profile).map_err(|e| e.to_string())?;
    let print = profile::load_profile_by_name(data_dir, print_stock).map_err(|e| e.to_string())?;
    Ok((film, print))
}

// The bake context keeps the existing parameter contract.
#[allow(clippy::too_many_arguments)]
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
    params.io.scan_film = params.workflow.route_enum() == WorkflowRoute::FilmScan;
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

// The snapshot uses the same complete bake context.
#[allow(clippy::too_many_arguments)]
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
            for p in raw.data.as_chunks::<3>().0 {
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
    stage: Stage,
    combination: bool,
}
fn recipes(topology: Topology, combinations: bool) -> Vec<Recipe> {
    let taps = topology.taps();
    let mut result: Vec<_> = topology
        .stages()
        .iter()
        .map(|stage| Recipe {
            stage: stage.clone(),
            combination: false,
        })
        .collect();
    if combinations {
        // Increasing span length matches pinned ordering: l12,l23,l34,l123,...
        for span in 2..taps.len() {
            for i in 0..taps.len() - span {
                let ids = (i + 1..=i + span)
                    .map(|id| id.to_string())
                    .collect::<String>();
                result.push(Recipe {
                    stage: Stage {
                        inject: taps[i],
                        collect: taps[i + span],
                        role: format!("subchain_{ids}").into(),
                        suffix: Some(format!("l{ids}").into()),
                        shared: topology.stages()[i + span - 1].shared,
                    },
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
    let cmyk_ink_active = pipeline.params().cmyk.active && pipeline.params().cmyk.strength > 0.0;
    let stage = &recipe.stage;
    let role = stage.effective_role(cmyk_ink_active);
    let image = lattice_image(spec.resolution, stage.inject, spec, wires, input)?;
    let raw =
        pipeline.process_with_taps(image, backend, Some(stage.inject), Some(stage.collect))?;
    let mut table = Vec::with_capacity(spec.resolution.pow(3));
    for pixel in raw.data.as_chunks::<3>().0 {
        let physical = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
        let code = if stage.collect == Tap::RgbOut {
            output.encode_rgb(physical.map(|v| v * output.output_gain()))
        } else {
            wires.encode(stage.collect, physical)?
        };
        if code.iter().any(|v| !v.is_finite()) {
            return Err(format!("nonfinite {role} LUT sample"));
        }
        table.push(code.map(|v| v.clamp(0.0, 1.0)));
    }
    let mut title = format!("v034_{}", normalize_stock(&spec.film_profile));
    if let Some(print) = print {
        title.push('_');
        title.push_str(&normalize_stock(print));
    }
    if let Some(suffix) = &stage.suffix {
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
    let meta = LutFileMeta {
        role: role.into_owned(),
        path: path.clone(),
        domain: stage.domain().into(),
        range: stage.range().into(),
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
    #[cfg(feature = "precision-f64")]
    use crate::test_support::data_dir;

    fn spec(input: &str, output: &str) -> BundleSpec {
        serde_json::from_value(serde_json::json!({
            "film_profile":"kodak_portra_400", "print_profiles":["kodak_portra_endura"],
            "input_color_space":input, "output_color_space":output
        }))
        .unwrap()
    }

    #[test]
    fn lut_topology_rejects_routes_without_matching_taps() {
        let error = validate_route_topology(Topology::Four, "input > film > scan").unwrap_err();
        assert!(error.contains("no print taps"));

        let error = validate_route_topology(Topology::Three, "input > convert-film > print > scan")
            .unwrap_err();
        assert!(error.contains("require 1lut"));

        assert!(validate_route_topology(Topology::Three, "input > film > scan").is_ok());
    }

    #[test]
    fn mixed_segmented_bake_rejects_before_profile_loading() {
        let mut params = RuntimeParams::default();
        params.output_mix.amount = 0.5;
        for topology in [Topology::Two, Topology::Three, Topology::Four] {
            let mut spec = spec("srgb", "srgb");
            spec.topology = topology;
            let error = BundleBuilder::with_params(spec, params.clone())
                .build(
                    Path::new("missing-profile-directory"),
                    &spektrafilm_backend::cpu_backend::CpuBackend,
                )
                .expect_err("segmented mixing must fail");
            assert!(error.contains("no prepared source branch"), "{error}");
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
        assert_eq!(baked.workflow.route, "input");
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
    fn cmyk_params_use_scan_final_route_and_remain_in_snapshot_input() {
        let film: profile::Profile =
            serde_json::from_str(r#"{"metadata":{},"info":{},"data":{}}"#).unwrap();
        let print = film.clone();
        let neutral = NeutralFilters::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let s = spec("srgb", "srgb");
        let input = lut_transport::resolve("srgb").unwrap();
        let mut base = RuntimeParams::default();
        base.cmyk.active = true;
        base.cmyk.strength = 0.75;
        base.workflow.route = "input > film > scan".into();
        let baked =
            bake_params(&s, input, input, &film, &print, &neutral, Some(&base), true).unwrap();
        assert_eq!(baked.workflow.route, "input > film > scan");
        assert!(baked.io.scan_film);
        assert_eq!(baked.cmyk.strength, 0.75);
        let json = serde_json::to_value(&baked).unwrap();
        assert_eq!(json["cmyk"]["active"], true);
        assert_eq!(json["cmyk"]["strength"], 0.75);
    }

    #[test]
    #[cfg(feature = "precision-f64")]
    fn cmyk_lut_provenance_names_the_model_and_preserves_the_input_route() {
        let data = data_dir();
        let backend = &spektrafilm_backend::cpu_backend::CpuBackend;
        // A small lattice keeps the bake cheap while still exercising the
        // whole `input` base route with the appearance enabled.
        let mut spec = spec("srgb", "srgb");
        spec.resolution = 4;
        let mut params = RuntimeParams::default();
        params.workflow.route = "input".into();
        params.io.scan_film = true;
        params.cmyk.active = true;
        params.cmyk.strength = 0.75;
        let bundle = BundleBuilder::with_params(spec, params)
            .build(&data, backend)
            .unwrap();

        // Route preservation: baking must not silently turn the `input`
        // base route into a film/print/scan route.
        assert_eq!(bundle.meta.workflow_route, "input");
        assert_eq!(
            bundle.baked_params["kodak_portra_endura"].workflow.route,
            "input"
        );
        // The final RGB stage carries the canonical `cmyk_` role name.
        let final_stage = bundle
            .meta
            .luts
            .iter()
            .find(|lut| lut.range == "output_rgb")
            .expect("a final RGB stage");
        assert!(
            final_stage.role.starts_with("cmyk_"),
            "final RGB stage role must use canonical cmyk naming: {}",
            final_stage.role
        );

        // Provenance: the style identifier, model revision, strength,
        // output space, transfer state and inclusion flag are recorded
        // independently of the base route.
        let provenance = &bundle.meta.provenance;
        assert_eq!(provenance["cmyk_ink_v1.style"], "ink");
        assert_eq!(provenance["cmyk_ink_v1.model"], "cmyk_ink_v1");
        assert_eq!(provenance["cmyk_ink_v1.strength"], "0.75");
        assert_eq!(provenance["cmyk_ink_v1.output_color_space"], "sRGB");
        assert_eq!(provenance["cmyk_ink_v1.transfer_state"], "baked");
        assert_eq!(provenance["cmyk_ink_v1.included"], "true");
        assert!(
            provenance.contains_key("cmyk_ink_v1"),
            "the final RGB stage must declare that it includes the appearance"
        );
    }

    #[test]
    #[cfg(feature = "precision-f64")]
    fn cmyk_lut_applies_the_appearance_exactly_once() {
        use crate::cmyk::CmykTransform;
        let data = data_dir();
        let backend = &spektrafilm_backend::cpu_backend::CpuBackend;
        let mut enabled_spec = spec("srgb", "srgb");
        enabled_spec.resolution = 5;
        enabled_spec.stops_above_midgray = Some(0.0);
        let mut params = RuntimeParams::default();
        params.workflow.route = "input".into();
        params.io.scan_film = true;
        params.cmyk.active = true;
        params.cmyk.strength = 1.0;
        let bundle = BundleBuilder::with_params(enabled_spec, params)
            .build(&data, backend)
            .unwrap();
        let (_, lut) = &bundle.luts[0];
        let output_space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();

        // A collapsed complete transform must apply the appearance exactly
        // once: evaluating the baked lattice must equal one reference
        // application over the same route with the appearance disabled.
        let mut disabled = RuntimeParams::default();
        disabled.workflow.route = "input".into();
        disabled.io.scan_film = true;
        disabled.cmyk.active = false;
        let mut base_spec = spec("srgb", "srgb");
        base_spec.resolution = 5;
        base_spec.stops_above_midgray = Some(0.0);
        let base = BundleBuilder::with_params(base_spec, disabled)
            .build(&data, backend)
            .unwrap();
        let (_, base_lut) = &base.luts[0];

        let transform = CmykTransform::new(output_space);
        let applied_once = crate::cmyk::appearance;
        let cctf = output_space.cctf;
        let n = lut.resolution;
        let mut worst_sample = ([0usize; 3], [0.0; 3], [0.0; 3]);
        let mut worst = 0.0f64;
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    let baked = lut.at(r, g, b);
                    // The baked lattice is display-encoded, so the reference
                    // application runs in the same encoded domain: decode,
                    // apply the appearance once, re-encode.
                    let baseline = base_lut.at(r, g, b);
                    let linear =
                        baseline.map(|v| spektrafilm_math::colorspace::cctf_decode(v, cctf));
                    let mapped = transform.to_rgb(applied_once(transform.to_lab(linear)));
                    let encoded = mapped.map(|v| {
                        spektrafilm_math::colorspace::cctf_encode(v, cctf).clamp(0.0, 1.0)
                    });
                    for c in 0..3 {
                        let error = (baked[c] - encoded[c]).abs();
                        if error > worst {
                            worst = error;
                            worst_sample = ([r, g, b], baked, encoded);
                        }
                    }
                }
            }
        }
        // The baked lattice is a 5^3 trilinear approximation of a smooth
        // point transform, so it must agree with a single reference
        // application inside the lattice's own interpolation error — never
        // twice, which would roughly double the chromatic deviation.
        assert!(
            worst < 5e-2,
            "baked LUT must apply the appearance exactly once (worst {worst}, sample {:?})",
            worst_sample
        );
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
