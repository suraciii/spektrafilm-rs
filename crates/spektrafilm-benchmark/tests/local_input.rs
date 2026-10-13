use serde_json::{Value, json};
use spektrafilm_benchmark::{common, documents, local_input::LocalInput};
use std::fs;
use std::path::Path;

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}
fn adapter(root: &Path) -> LocalInput {
    fs::write(
        root.join("input.png"),
        b"hashable input: decoding belongs to renderer",
    )
    .unwrap();
    write_json(
        &root.join("configuration.json"),
        &json!({"route":"input","set":[]}),
    );
    LocalInput {
        input: root.join("input.png"),
        configuration: root.join("configuration.json"),
        input_kind: "raster".into(),
        input_color_space: Some("sRGB".into()),
        input_cctf_decoding: Some(true),
        seeds: vec![],
        regions: None,
        reference: None,
        reference_info: None,
        preparation: None,
    }
}

#[test]
fn generated_suite_retains_explicit_protocol_and_each_document_path_owner() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut input = adapter(root);
    let config = root.join("config");
    let regions = root.join("regions");
    let prep = root.join("preparation");
    fs::create_dir(&config).unwrap();
    fs::create_dir(&regions).unwrap();
    fs::create_dir(&prep).unwrap();
    fs::write(config.join("params.json"), b"{}").unwrap();
    write_json(
        &config.join("configuration.json"),
        &json!({"route":"input","params":"params.json"}),
    );
    write_json(
        &regions.join("regions.json"),
        &json!([{ "id":"detail","label":"Detail","role":"other","geometry":[0,0,2,2],"measurements":["numeric"] }]),
    );
    fs::write(prep.join("original.bin"), b"original").unwrap();
    write_json(
        &prep.join("preparation.json"),
        &json!({"original":{"path":"original.bin","sha256":common::sha256_file(&prep.join("original.bin")).unwrap()},"operations":["crop [8, 4, 16, 16]"]}),
    );
    input.configuration = config.join("configuration.json");
    input.regions = Some(regions.join("regions.json"));
    input.preparation = Some(prep.join("preparation.json"));
    input.seeds = vec![42, u32::MAX];
    let suite = input.suite().unwrap();
    assert_eq!(suite["id"], "local-image");
    assert_eq!(
        suite["protocol"]["region_sampling_scales"],
        json!([1.0, 0.4, 0.16])
    );
    assert_eq!(suite["protocol"]["hotspots"]["min_area_px"], 151);
    assert_eq!(
        suite["cases"][0]["configuration"]["params"],
        json!(config.join("params.json"))
    );
    assert_eq!(
        suite["cases"][0]["preparation"]["original"]["path"],
        json!(prep.join("original.bin"))
    );
    assert_eq!(suite["cases"][0]["seeds"], json!([42, u32::MAX]));
}

#[test]
fn single_source_declarations_and_population_fail_before_destination_creation() {
    let directory = tempfile::tempdir().unwrap();
    let mut input = adapter(directory.path());
    input.input_color_space = None;
    assert_eq!(input.suite().unwrap_err().code, "source_declaration");
    input.input_kind = "raw".into();
    input.input_cctf_decoding = None;
    assert_eq!(
        input.suite().unwrap()["cases"][0]["input"]["adapter"],
        "spektrafilm-raw"
    );
    input.seeds = vec![7, 7];
    assert!(input.suite().is_err());
    input.seeds.clear();
    input.reference = Some(directory.path().join("reference.png"));
    assert_eq!(input.suite().unwrap_err().code, "reference_declaration");
}

#[test]
fn existing_shapes_reject_unknown_fields_and_bad_preparation_hash() {
    let directory = tempfile::tempdir().unwrap();
    let input = adapter(directory.path());
    write_json(
        &input.configuration,
        &json!({"route":"input","invented":true}),
    );
    assert_eq!(input.suite().unwrap_err().code, "unknown_field");
    let preparation = directory.path().join("preparation.json");
    write_json(
        &preparation,
        &json!({"original":{"path":"input.png","sha256":"0".repeat(64)},"operations":["crop"]}),
    );
    assert!(spektrafilm_benchmark::local_input::load("preparation", &preparation).is_err());
    for kind in ["configuration", "reference", "regions", "preparation"] {
        assert_eq!(
            documents::schema_for(kind).unwrap()["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
    }
}
