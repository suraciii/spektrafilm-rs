use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::ScannerProvenance;
use spektrafilm_core::image_io::ImageFormat;
use spektrafilm_core::params::RuntimeParams;
use std::fs;
use std::path::Path;

pub const IMPLEMENTATION: &str = "spektrafilm-rs";
pub const ADAPTER_VERSION: &str = "spektrafilm-rs-adapter-1";
pub const PARAMETER_SCHEMA_VERSION: &str = "spektrafilm-rs-params-6";
const LEGACY_PARAMETER_SCHEMA_VERSIONS: &[&str] = &[
    "spektrafilm-rs-params-1",
    "spektrafilm-rs-params-2",
    "spektrafilm-rs-params-3",
    "spektrafilm-rs-params-4",
    "spektrafilm-rs-params-5",
];
pub const FORK_REFERENCE: &str = "suraciii/spektrafilm-rs";
pub const FINISHED_JPEG_QUALITY: u8 = 85;
pub const MAX_EDGE: u32 = 9568;
pub const MAX_DECODED_BYTES: u64 = 2_147_483_648;
pub const MAX_SEED: u64 = u32::MAX as u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputContract {
    pub format: String,
    pub precision_bits: u8,
    pub color_space: String,
    pub transfer_function: String,
    pub geometry: String,
    pub encoding: String,
    #[serde(default)]
    pub max_edge: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenderRecipe {
    pub schema_version: String,
    pub film_profile: String,
    #[serde(default)]
    pub print_profile: Option<String>,
    pub parameters: Value,
    pub output: OutputContract,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Built-in derived scanner provenance. Optional for legacy recipes.
    #[serde(default)]
    pub scanner_provenance: Option<Value>,
}

pub fn default_output(format: &str) -> OutputContract {
    let preview = format == "png";
    OutputContract {
        format: format.to_owned(),
        precision_bits: 8,
        color_space: "sRGB".to_owned(),
        transfer_function: "srgb".to_owned(),
        geometry: if preview {
            "bounded"
        } else {
            "input-preserving"
        }
        .to_owned(),
        encoding: if preview {
            "bounded-preview".to_owned()
        } else {
            format!("quality-{FINISHED_JPEG_QUALITY}-baseline")
        },
        max_edge: None,
    }
}

pub fn validate_seed(seed: u64) -> Result<()> {
    if seed > MAX_SEED {
        bail!("seed must be <= {MAX_SEED} for the admitted runtime contract");
    }
    Ok(())
}

pub fn read_recipe(path: &Path) -> Result<RenderRecipe> {
    let file =
        fs::File::open(path).with_context(|| format!("opening recipe {}", path.display()))?;
    let mut recipe: RenderRecipe = serde_json::from_reader(file)
        .with_context(|| format!("parsing recipe {}", path.display()))?;
    if let Some(seed) = recipe.seed {
        validate_seed(seed)?;
    }
    if !matches!(recipe.schema_version.as_str(), PARAMETER_SCHEMA_VERSION)
        && !LEGACY_PARAMETER_SCHEMA_VERSIONS.contains(&recipe.schema_version.as_str())
    {
        bail!("unsupported recipe schema {}", recipe.schema_version);
    }
    if recipe.film_profile.trim().is_empty() {
        bail!("filmProfile must not be empty");
    }
    validate_output(&recipe.output)?;
    let mut parameters = canonical_parameters(recipe.parameters.clone())?;
    if LEGACY_PARAMETER_SCHEMA_VERSIONS.contains(&recipe.schema_version.as_str()) {
        // Params 5 already rejected the older retired-field spellings.
        if recipe.schema_version != "spektrafilm-rs-params-5" {
            spektrafilm_core::params::sources::normalize_legacy_complete_params(&mut parameters)
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(provenance) = recipe.scanner_provenance.as_ref() {
            recipe.scanner_provenance = Some(
                ScannerProvenance::migrate_legacy(provenance, parameters.pointer("/scanner/lamp"))
                    .map_err(anyhow::Error::msg)?,
            );
        }
        spektrafilm_core::params::sources::normalize_legacy_scan_readout(&mut parameters, true)
            .map_err(anyhow::Error::msg)?;
    }
    let params = normalize_parameters(parameters.clone())?;
    recipe.parameters = serde_json::to_value(&params)?;
    recipe.schema_version = PARAMETER_SCHEMA_VERSION.into();
    if let Some(provenance) = &recipe.scanner_provenance {
        let expected = serde_json::to_value(ScannerProvenance::from_params(&params))?;
        if provenance != &expected {
            bail!("scannerProvenance does not match the typed scanner parameters");
        }
    }
    Ok(recipe)
}

fn canonical_parameters(value: Value) -> Result<Value> {
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("parameters must be a JSON object"))?;
    if object.contains_key("filmRender") && object.contains_key("film_render") {
        bail!("parameters contain both filmRender and film_render");
    }
    if object.contains_key("printRender") && object.contains_key("print_render") {
        bail!("parameters contain both printRender and print_render");
    }
    if let Some(value) = object.remove("filmRender") {
        object.insert("film_render".into(), value);
    }
    if let Some(value) = object.remove("printRender") {
        object.insert("print_render".into(), value);
    }
    object.remove("output");
    Ok(Value::Object(object))
}

pub fn normalize_parameters(value: Value) -> Result<RuntimeParams> {
    if value["io"]["output_gamut_compress"]
        .get("lightness_compression")
        .is_some()
    {
        bail!(
            "io.output_gamut_compress.lightness_compression is retired; explicitly remove this setting from the document"
        );
    }
    let params: RuntimeParams = serde_json::from_value(canonical_parameters(value)?)
        .context("parameters do not match the spektrafilm-rs RuntimeParams schema")?;
    params.validate().map_err(anyhow::Error::msg)?;
    let route = params.workflow.route_enum();
    if route.artifact_input().is_some() || route.artifact_output().is_some() {
        bail!("RenderRecipe supports image routes only; material artifact routes require process");
    }
    Ok(params)
}

pub fn validate_input_path(path: &Path) -> Result<()> {
    if ImageFormat::detect(path)? != ImageFormat::Tiff {
        bail!("render input must be a TIFF");
    }
    let file =
        fs::File::open(path).with_context(|| format!("opening render input {}", path.display()))?;
    let mut decoder = tiff::decoder::Decoder::new(file)
        .with_context(|| format!("reading TIFF header {}", path.display()))?;
    let (width, height) = decoder.dimensions()?;
    validate_input_dimensions(width, height)?;
    let bits = match decoder.colortype()? {
        tiff::ColorType::RGB(bits)
        | tiff::ColorType::RGBA(bits)
        | tiff::ColorType::Gray(bits)
        | tiff::ColorType::GrayA(bits) => bits,
        other => bail!("render input has unsupported TIFF color type {other:?}"),
    };
    if bits != 32 {
        bail!("render input must be a 32-bit TIFF");
    }
    Ok(())
}

pub fn validate_input_dimensions(width: u32, height: u32) -> Result<()> {
    if width.max(height) > MAX_EDGE {
        bail!("render input exceeds maxEdge {MAX_EDGE}: {width}x{height}");
    }
    let decoded_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(3))
        .and_then(|samples| samples.checked_mul(std::mem::size_of::<f64>() as u64))
        .ok_or_else(|| anyhow::anyhow!("render input decoded size overflows"))?;
    if decoded_bytes > MAX_DECODED_BYTES {
        bail!("render input exceeds maxDecodedBytes {MAX_DECODED_BYTES}: {decoded_bytes}");
    }
    Ok(())
}

pub fn validate_output_path(path: &Path, output: &OutputContract) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let matches = match output.format.as_str() {
        "jpeg" => extension == "jpg" || extension == "jpeg",
        "png" => extension == "png",
        _ => false,
    };
    if !matches {
        bail!(
            "output path extension does not match declared {} format",
            output.format
        );
    }
    Ok(())
}

pub fn validate_output(output: &OutputContract) -> Result<()> {
    let preview = output.format == "png";
    let expected = default_output(&output.format);
    if output.format != "jpeg" && !preview {
        bail!(
            "unsupported output format {}; use jpeg or png",
            output.format
        );
    }
    if output.precision_bits != expected.precision_bits
        || output.color_space != expected.color_space
        || output.transfer_function != expected.transfer_function
        || output.geometry != expected.geometry
        || output.encoding != expected.encoding
    {
        bail!("output contract is not an admitted spektrafilm-rs contract");
    }
    if preview && output.max_edge.is_none_or(|edge| edge == 0) {
        bail!("bounded png output requires a positive maxEdge");
    }
    if preview && output.max_edge.is_some_and(|edge| edge > MAX_EDGE) {
        bail!("bounded png output exceeds maxEdge {MAX_EDGE}");
    }
    if !preview && output.max_edge.is_some() {
        bail!("full-size jpeg output must not carry maxEdge");
    }
    Ok(())
}

pub fn parameter_digest(parameters: &Value) -> String {
    let bytes = serde_json::to_vec(parameters).expect("JSON values are serializable");
    format!("{:x}", Sha256::digest(bytes))
}

pub fn describe() -> Value {
    let defaults =
        serde_json::to_value(RuntimeParams::default()).expect("runtime defaults serialize");
    let controls = [
        "camera",
        "enlarger",
        "scanner",
        "film_render",
        "print_render",
        "cmyk",
        "io",
        "settings",
        "debug",
        "taps",
    ]
    .into_iter()
    .map(|id| {
        json!({
            "id": id,
            "readable": true,
            "editable": true,
            "executable": true,
            "qualification": format!("{ADAPTER_VERSION}:{id}"),
        })
    })
    .collect::<Vec<_>>();
    json!({
        "implementation": IMPLEMENTATION,
        "forkReference": FORK_REFERENCE,
        "forkCommit": std::env::var("SPEKTRAFILM_FORK_COMMIT")
            .ok()
            .or_else(|| option_env!("SPEKTRAFILM_FORK_COMMIT").map(ToOwned::to_owned))
            .unwrap_or_else(|| "unknown".to_owned()),
        "adapterVersion": ADAPTER_VERSION,
        "parameterSchemaVersion": PARAMETER_SCHEMA_VERSION,
        "modelVersion": spektrafilm_core::runtime::MODEL_VERSION,
        "parameterSchema": {
            "type": "object",
            "moduleOwned": true,
            "default": defaults,
            "controls": controls,
        },
        "inputs": [{
            "format": "tiff",
            "precisionBits": 32,
            "colorSpace": "prophoto-rgb",
            "transferFunction": "linear",
            "geometry": "source-preserving",
        }],
        "outputs": [
            default_output("jpeg"),
            {"format":"tiff","state":"retained-intent","refusal":{"code":"not-qualified","reason":"film-tiff writer qualification is pending","recoveryAction":"use film-jpeg"}}
        ],
        "limits": {
            "deadlineMillis": 900000,
            "maxEdge": 9568,
            "maxDecodedBytes": 2147483648u64,
            "seedPolicy": "explicit-u32-or-deterministic-default",
        },
        "errors": [
            "invalid-recipe", "unsupported-control", "incompatible-input", "resource-unavailable",
            "cancelled", "deadline", "engine-failure", "output-invalid", "outcome-unknown"
        ],
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn recipe_inactive_chemistry_is_validated_without_rewriting_sources() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recipe.json");
        for version in ["spektrafilm-rs-params-1", PARAMETER_SCHEMA_VERSION] {
            for exhaustion in [0.0, 0.5, 1.0, -0.1, 2.0] {
                let parameters = json!({"print_render": {"density_curves_morph": {
                    "active": false, "developer_exhaustion": exhaustion
                }}});
                let source = serde_json::to_vec(&json!({
                    "schemaVersion": version, "filmProfile": "kodak_portra_400",
                    "parameters": parameters, "output": default_output("jpeg")
                }))
                .unwrap();
                std::fs::write(&path, &source).unwrap();
                let result = read_recipe(&path);
                if (0.0..=1.0).contains(&exhaustion) {
                    let recipe = result.unwrap();
                    assert_eq!(
                        recipe.parameters["print_render"]["density_curves_morph"]["active"],
                        false
                    );
                    assert_eq!(
                        recipe.parameters["print_render"]["density_curves_morph"]["developer_exhaustion"],
                        exhaustion
                    );
                } else {
                    assert!(
                        result
                            .unwrap_err()
                            .to_string()
                            .contains("print_render.density_curves_morph.developer_exhaustion")
                    );
                }
                assert_eq!(std::fs::read(&path).unwrap(), source);
            }
        }
    }

    #[test]
    fn recipe_versions_migrate_only_null_retired_values_and_preserve_source() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-recipe-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("recipe.json");
        let mut versions: Vec<&str> = LEGACY_PARAMETER_SCHEMA_VERSIONS.to_vec();
        versions.push(PARAMETER_SCHEMA_VERSION);
        for version in versions {
            for retired in [None, Some(Value::Null), Some(json!([0.95, 1.0, 1.6]))] {
                let mut parameters = serde_json::to_value(RuntimeParams::default()).unwrap();
                if let Some(value) = &retired {
                    parameters["io"]["output_gamut_compress"]["lightness_compression"] =
                        value.clone();
                }
                let source = serde_json::to_vec(&json!({"schemaVersion":version,"filmProfile":"kodak_gold_200","parameters":parameters,"output":default_output("jpeg")})).unwrap();
                std::fs::write(&path, &source).unwrap();
                let result = read_recipe(&path);
                if retired.is_none()
                    || (LEGACY_PARAMETER_SCHEMA_VERSIONS.contains(&version)
                        && version != "spektrafilm-rs-params-5"
                        && retired == Some(Value::Null))
                {
                    let accepted = result.unwrap();
                    assert_eq!(accepted.schema_version, PARAMETER_SCHEMA_VERSION);
                    assert_eq!(
                        normalize_parameters(accepted.parameters)
                            .unwrap()
                            .output_mix
                            .amount,
                        1.0
                    );
                } else {
                    assert!(
                        format!("{:#}", result.unwrap_err())
                            .contains("io.output_gamut_compress.lightness_compression")
                    );
                }
                assert_eq!(std::fs::read(&path).unwrap(), source);
            }
        }
        for invalid in [
            json!({"film_render":{"response":{}}}),
            json!({"print_render":{"response":{}}}),
            json!({"enlarger":{"c_filter_shift":0.0}}),
            json!({"output_mix":{"amount":1.01}}),
            json!({"output_mix":{"amount":-0.01}}),
        ] {
            let source = serde_json::to_vec(&json!({"schemaVersion":PARAMETER_SCHEMA_VERSION,"filmProfile":"kodak_gold_200","parameters":invalid,"output":default_output("jpeg")})).unwrap();
            std::fs::write(&path, &source).unwrap();
            assert!(read_recipe(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), source);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recipe_scan_cutover_preserves_capture_and_rejects_current_lamp_and_forged_provenance() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recipe.json");
        let mut metadata =
            serde_json::to_value(ScannerProvenance::from_params(&RuntimeParams::default()))
                .unwrap();
        metadata["model"] = "neutral_scan_v2".into();
        metadata["lamp"] = "T".into();
        let mut document = json!({"schemaVersion":"spektrafilm-rs-params-5","filmProfile":"kodak_portra_400",
            "parameters":{"scanner":{"lamp":"T","conversion_style":"neutral"}},
            "scannerProvenance":metadata,"output":default_output("jpeg")});
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let recipe = read_recipe(&path).unwrap();
        assert_eq!(recipe.schema_version, PARAMETER_SCHEMA_VERSION);
        assert_eq!(
            recipe.parameters["film_render"]["convert"]["scan_illuminant"],
            "T"
        );
        assert_eq!(recipe.parameters["scanner"]["conversion_style"], "standard");
        assert!(recipe.parameters["scanner"].get("lamp").is_none());
        assert_eq!(recipe.scanner_provenance.unwrap()["model"], "lab_scan_v1");
        document["scannerProvenance"]["sensor"] = "forged".into();
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(read_recipe(&path).is_err());
        document["schemaVersion"] = PARAMETER_SCHEMA_VERSION.into();
        document
            .as_object_mut()
            .unwrap()
            .remove("scannerProvenance");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(read_recipe(&path).is_err());
    }

    #[test]
    fn recipe_operator_roundtrip_defaults_and_rejections_use_the_real_loader() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("operator.json");
        let operator =
            json!({"exposure_ev":0.5000000000000001,"warmth_ev":-0.2,"tint_ev":0.1,"contrast":1.2});
        let mut document = json!({
            "schemaVersion":PARAMETER_SCHEMA_VERSION,"filmProfile":"kodak_portra_400",
            "parameters":{"workflow":{"route":"input > film > scan"},
                "scanner":{"scan_output":"negative_to_positive","operator":operator}},
            "output":default_output("png")
        });
        document["output"]["maxEdge"] = 128.into();
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let accepted = read_recipe(&path).unwrap();
        assert_eq!(accepted.parameters["scanner"]["operator"], operator);
        std::fs::write(&path, serde_json::to_vec(&accepted).unwrap()).unwrap();
        assert_eq!(read_recipe(&path).unwrap().parameters, accepted.parameters);
        document["parameters"]["scanner"]
            .as_object_mut()
            .unwrap()
            .remove("operator");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(
            normalize_parameters(read_recipe(&path).unwrap().parameters)
                .unwrap()
                .scanner
                .operator
                .is_default()
        );
        for scanner in [
            json!({"operator":{"exposure_ev":0.5}}),
            json!({"scan_output":"negative_to_positive","conversion_style":"fixed","operator":{"warmth_ev":0.2}}),
            json!({"scan_output":"negative_to_positive","operator":{"contrast":0}}),
            json!({"scan_output":"negative_to_positive","operator":{"unknown":1}}),
            json!({"conversion_exposure_ev":0}),
            json!({"auto_setup":"full"}),
            json!({"finish":"lab_scan"}),
        ] {
            document["parameters"]["scanner"] = scanner;
            let source = serde_json::to_vec(&document).unwrap();
            std::fs::write(&path, &source).unwrap();
            assert!(read_recipe(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), source);
        }
    }

    use super::*;

    #[test]
    fn admitted_output_requires_matching_path_and_bound() {
        let output = OutputContract {
            max_edge: Some(1024),
            ..default_output("png")
        };
        assert!(validate_output(&output).is_ok());
        assert!(validate_output_path(Path::new("result.png"), &output).is_ok());
        assert!(validate_output_path(Path::new("result.jpg"), &output).is_err());
        assert!(
            validate_output(&OutputContract {
                max_edge: Some(MAX_EDGE + 1),
                ..output
            })
            .is_err()
        );
    }

    #[test]
    fn seed_contract_rejects_gpu_truncation() {
        assert!(validate_seed(MAX_SEED).is_ok());
        assert!(validate_seed(MAX_SEED + 1).is_err());
    }

    #[test]
    fn render_input_limits_reject_oversized_decodes() {
        assert!(validate_input_dimensions(MAX_EDGE, 1).is_ok());
        assert!(validate_input_dimensions(1024, 1024).is_ok());
        assert!(validate_input_dimensions(MAX_EDGE + 1, 1).is_err());
        assert!(validate_input_dimensions(20_000, 20_000).is_err());
    }
}
