use crate::{
    common::{Error, Result, create_destination, dump, sha256_file},
    documents, image_data,
};
use image::{Rgb, RgbImage};
use serde_json::{Value, json};
use std::{fs, path::Path};

const NATURAL_SOURCE: &str = include_str!("../assets/natural-source.json");
fn configuration(grain: bool, spatial: bool) -> Value {
    let mut edits = vec![
        "camera.auto_exposure=false".to_owned(),
        "io.output_color_space=sRGB".into(),
        "io.output_cctf_encoding=true".into(),
        format!("debug.deactivate_spatial_effects={}", !spatial),
        format!("debug.deactivate_stochastic_effects={}", !grain),
        format!("film_render.grain.active={grain}"),
    ];
    if grain {
        edits.extend(
            [
                "film_render.grain.engine=v2",
                "film_render.grain.v2_profile=custom",
                "film_render.grain.v2_mode=noise",
                "film_render.grain.v2_amount=25",
            ]
            .map(str::to_owned),
        );
    }
    json!({"film":"kodak_gold_200","paper":"kodak_portra_endura","route":"input > film > print > scan","presets":[],"set_files":[],"set":edits})
}
fn region(id: &str, rect: [usize; 4], families: &[&str], role: &str) -> Value {
    json!({"id":id,"label":human_label(id),"role":role,"geometry":rect,"measurements":families})
}
fn human_label(id: &str) -> String {
    id.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or(String::new(), |c| {
                c.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn make_case(
    id: &str,
    path: &Path,
    families: &[&str],
    regions: Vec<Value>,
    grain: bool,
    spatial: bool,
    probe: Value,
) -> Result<Value> {
    let mut value = json!({"id":id,"kind":"probe","input":{"kind":"raster","path":path.file_name().and_then(|s|s.to_str()),"sha256":sha256_file(path)?,"color_space":"sRGB","cctf_decoding":true},"configuration":configuration(grain,spatial),"seeds":if grain {vec![0,42,99]} else {vec![0]},"regions":regions,"measurements":families});
    if !probe.is_null() {
        value["probe"] = probe;
    }
    Ok(value)
}
fn save_probe(
    path: &Path,
    width: u32,
    height: u32,
    sample: impl Fn(u32, u32) -> [f64; 3],
) -> Result<()> {
    let mut image = RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgb(sample(x, y).map(|v| (v * 255.0).round_ties_even().clamp(0.0, 255.0) as u8));
    }
    image
        .save(path)
        .map_err(|e| Error::new("image_write", e.to_string()).at(path.display().to_string()))
}
/// Generate controlled authored suites. Natural RAW acquisition occurs only on explicit request.
pub fn create_fixtures(
    renderer: &Path,
    data_dir: &Path,
    out: &Path,
    natural_input: Option<&Path>,
    download_natural: bool,
) -> Result<Value> {
    create_destination(out)?;
    let colors = [
        [0.45, 0.45, 0.45],
        [0.65, 0.42, 0.32],
        [0.2, 0.55, 0.3],
        [0.1, 0.3, 0.65],
        [0.7, 0.15, 0.12],
        [0.15, 0.65, 0.65],
        [0.65, 0.15, 0.65],
        [0.7, 0.65, 0.1],
    ];
    let path = out.join("chart.png");
    save_probe(&path, 128, 128, |x, y| {
        if y < 32 {
            [x as f64 / 127.0; 3]
        } else {
            colors[x as usize / 16]
        }
    })?;
    let chart = make_case(
        "color_chart",
        &path,
        &["numeric", "color", "tone", "flip"],
        vec![
            region("skin", [16, 32, 16, 64], &["color", "tone"], "skin"),
            region("gray", [0, 32, 16, 64], &["color", "tone"], "neutral"),
        ],
        false,
        false,
        Value::Null,
    )?;
    let path = out.join("exposure.png");
    save_probe(&path, 128, 64, |x, _| {
        [image_data::srgb_encode((0.18 * 2_f64.powi(x as i32 / 16 - 5)).min(1.0)); 3]
    })?;
    let response: Vec<_> = (0..8)
        .map(|i| json!({"exposure_stops":(i as f64)-5.0,"geometry":[i*16+3,3,10,58]}))
        .collect();
    let tone = make_case(
        "exposure_response",
        &path,
        &["numeric", "color", "tone", "flip"],
        vec![],
        false,
        false,
        json!({"tone_response":response}),
    )?;
    let path = out.join("flat_fields.png");
    save_probe(&path, 128, 128, |x, _| {
        [[0.1, 0.3, 0.5, 0.8][x as usize / 32]; 3]
    })?;
    let grain = make_case(
        "grain_fields",
        &path,
        &["numeric", "color", "flip"],
        (0..4)
            .map(|i| {
                region(
                    &format!("level_{i}"),
                    [i * 32 + 4, 4, 24, 120],
                    &["grain", "color"],
                    "flat_field",
                )
            })
            .collect(),
        true,
        false,
        Value::Null,
    )?;
    let path = out.join("bright_point.png");
    save_probe(&path, 128, 128, |x, y| {
        [if (x as f64 - 63.5).powi(2) + (y as f64 - 63.5).powi(2) <= 9.0 {
            1.0
        } else {
            0.04
        }; 3]
    })?;
    let point = make_case(
        "halation_point",
        &path,
        &["numeric", "color", "flip", "halation"],
        vec![],
        false,
        true,
        json!({"halation":{"center_px":[63.5,63.5],"core_radius_px":3.0,"background_inner_radius_px":45.0,"background_outer_radius_px":58.0,"extent_px":40.0,"bin_width_px":1.0,"domain":"linear_srgb"}}),
    )?;
    let path = out.join("edge.png");
    save_probe(&path, 128, 128, |x, _| [if x < 64 { 0.1 } else { 0.8 }; 3])?;
    let contrast = image_data::srgb_decode(204.0 / 255.0) - image_data::srgb_decode(26.0 / 255.0);
    let edge = make_case(
        "edge_response",
        &path,
        &["numeric", "color", "flip", "edge"],
        vec![],
        false,
        true,
        json!({"edge":{"axis":"x","plateau_fraction":0.2,"domain":"linear_srgb","input_contrast":contrast,"input_exposure_stops":0.0}}),
    )?;
    let protocol = json!({"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0,4.0,16.0],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":4.0,"border_px":1},"spectrum":{"window":"hann"}},"hotspots":{"count":6,"min_area_px":1024}});
    let mut quick =
        json!({"schema_version":1,"id":"quick-probes","protocol":protocol,"cases":[chart,tone]});
    let mut complete = json!({"schema_version":1,"id":"complete-probes","protocol":protocol,"cases":[chart,tone,grain,point,edge]});
    let mut provenance = json!({"schema_version":1,"controlled_images":"Deterministic RGB probes generated by this package; not measured spectral or physical film references.","natural_scene":null});
    let mut suites = vec!["quick.json", "complete.json"];
    if natural_input.is_some() || download_natural {
        let source: Value = serde_json::from_str(NATURAL_SOURCE)?;
        let destination = out.join("natural.KDC");
        if let Some(input) = natural_input {
            fs::copy(input, &destination)?;
        } else {
            let mut response =
                ureq::get(source["source_url"].as_str().ok_or_else(|| {
                    Error::new("invalid_document", "Natural provenance URL missing")
                })?)
                .config()
                .timeout_global(Some(std::time::Duration::from_secs(120)))
                .build()
                .call()
                .map_err(|e| Error::new("dependency_download", e.to_string()))?;
            let mut reader = response.body_mut().as_reader();
            let mut file = fs::File::create(&destination)?;
            std::io::copy(&mut reader, &mut file)?;
        }
        if sha256_file(&destination)? != source["sha256"].as_str().unwrap_or("") {
            return Err(Error::new(
                "dependency_hash",
                "Natural RAW does not match pinned provenance",
            )
            .at(destination.display().to_string()));
        }
        let mut config = configuration(false, false);
        config["raw"] = json!({"white_balance":"as-shot","lens_correction":false});
        if fs::metadata(&destination)?.len()
            != source["bytes"].as_u64().ok_or_else(|| {
                Error::new("invalid_document", "Natural provenance byte count missing")
            })?
        {
            return Err(Error::new(
                "dependency_hash",
                "Natural RAW byte count does not match pinned provenance",
            )
            .at(destination.display().to_string()));
        }
        let natural = json!({"id":"kodak_dc50_scene","kind":"scene","input":{"kind":"raw","adapter":"spektrafilm-raw","path":"natural.KDC","sha256":source["sha256"]},"configuration":config,"seeds":[0],"regions":[region("shadow",[64,64,160,128],&["color","tone"],"shadow"),region("texture",[320,192,160,128],&["color","tone"],"texture")],"measurements":["numeric","color","tone","flip"]});
        dump(
            &out.join("natural.json"),
            &json!({"schema_version":1,"id":"natural-scene","protocol":protocol,"cases":[natural]}),
        )?;
        suites.push("natural.json");
        provenance["natural_scene"] = source;
        quick["id"] = json!("quick-probes-and-scene");
        complete["id"] = json!("complete-probes-and-scene");
        quick["cases"].as_array_mut().unwrap().push(natural.clone());
        complete["cases"].as_array_mut().unwrap().push(natural);
    }
    dump(&out.join("quick.json"), &quick)?;
    dump(&out.join("complete.json"), &complete)?;
    let renderer_path = renderer.canonicalize()?;
    let data_path = data_dir.canonicalize()?;
    let hash = sha256_file(renderer)?;
    for (name, edits) in [
        ("baseline", vec![]),
        ("candidate", vec!["camera.exposure_compensation_ev=0.25"]),
        ("grain-candidate", vec!["film_render.grain.v2_amount=35"]),
    ] {
        dump(
            &out.join(format!("{name}.json")),
            &json!({"schema_version":1,"id":name,"renderer":{"path":renderer_path,"sha256":hash},"data_dir":data_path,"backend":"cpu","changes":{"set_files":[],"set":edits},"environment":{"RAYON_NUM_THREADS":"8","OPENBLAS_NUM_THREADS":"1","OMP_NUM_THREADS":"1"}}),
        )?;
        documents::load_target(&out.join(format!("{name}.json")))?;
    }
    let policy = json!({"schema_version":1,"id":"exploratory-color","protocol_digest":documents::protocol_digest(&complete)?,"calibration":{"status":"exploratory","evidence":[]},"rules":[{"id":"skin-color","purpose":"protect","selector":{"case_id":"color_chart","region_id":"skin"},"metric":"delta_e00.p95","max":2.0,"required":true}]});
    dump(&out.join("exploratory-policy.json"), &policy)?;
    dump(&out.join("provenance.json"), &provenance)?;
    for suite in &suites {
        documents::load_suite(&out.join(suite))?;
    }
    documents::load_policy(&out.join("exploratory-policy.json"))?;
    Ok(json!({"schema_version":1,"execution_status":"completed","artifact":out,"suites":suites}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probes_use_numpy_ties_even_quantization() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("probe.png");
        save_probe(&path, 2, 1, |x, _| [if x == 0 { 0.1 } else { 0.5 }; 3]).unwrap();
        let image = image::open(path).unwrap().to_rgb8();
        assert_eq!(image.get_pixel(0, 0).0, [26; 3]);
        assert_eq!(image.get_pixel(1, 0).0, [128; 3]);
    }
    #[test]
    fn grain_settings_and_seeds_preserve_controlled_probe_contract() {
        let config = configuration(true, false);
        let edits = config["set"].as_array().unwrap();
        assert!(edits.contains(&json!("debug.deactivate_stochastic_effects=false")));
        assert!(edits.contains(&json!("film_render.grain.v2_amount=25")));
        let provenance: Value = serde_json::from_str(NATURAL_SOURCE).unwrap();
        assert_eq!(
            provenance["sha256"],
            "37e290dbd0053f00e508d02a6b3a2a990432dad1eb74c40a52ca899f0f225ecc"
        );
    }
    #[test]
    fn generated_suites_targets_policy_and_provenance_are_complete() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("fixtures");
        let renderer = std::env::current_exe().unwrap();
        let result = create_fixtures(&renderer, temp.path(), &out, None, false).unwrap();
        assert_eq!(result["suites"], json!(["quick.json", "complete.json"]));
        let quick = documents::load_suite(&out.join("quick.json")).unwrap();
        let complete = documents::load_suite(&out.join("complete.json")).unwrap();
        assert_eq!(quick["cases"].as_array().unwrap().len(), 2);
        assert_eq!(complete["cases"].as_array().unwrap().len(), 5);
        assert_eq!(complete["cases"][2]["seeds"], json!([0, 42, 99]));
        assert_eq!(
            complete["cases"][1]["probe"]["tone_response"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        assert_eq!(
            complete["cases"][4]["probe"]["edge"]["input_contrast"],
            json!(image_data::srgb_decode(204.0 / 255.0) - image_data::srgb_decode(26.0 / 255.0))
        );
        let policy = documents::load_policy(&out.join("exploratory-policy.json")).unwrap();
        assert_eq!(
            policy["protocol_digest"],
            documents::protocol_digest(&complete).unwrap()
        );
        assert_eq!(policy["calibration"]["status"], "exploratory");
        let candidate = documents::load_target(&out.join("candidate.json")).unwrap();
        assert_eq!(
            candidate["changes"]["set"],
            json!(["camera.exposure_compensation_ev=0.25"])
        );
        assert_eq!(
            candidate["renderer"]["sha256"],
            sha256_file(&renderer).unwrap()
        );
        assert!(!out.join("natural.KDC").exists());
        assert!(
            documents::load_json_document(&out.join("provenance.json")).unwrap()["natural_scene"]
                .is_null()
        );
        assert_eq!(
            create_fixtures(&renderer, temp.path(), &out, None, false)
                .unwrap_err()
                .code,
            "destination_exists"
        );
    }
    #[test]
    fn supplied_natural_input_must_match_pinned_hash() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("wrong.KDC");
        fs::write(&input, b"not the pinned RAW").unwrap();
        let error = create_fixtures(
            &std::env::current_exe().unwrap(),
            temp.path(),
            &temp.path().join("fixtures"),
            Some(&input),
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "dependency_hash");
    }
}
