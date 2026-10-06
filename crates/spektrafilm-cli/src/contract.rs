use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::image_io::ImageFormat;
use spektrafilm_core::params::RuntimeParams;
use std::fs;
use std::path::Path;

pub const IMPLEMENTATION: &str = "spektrafilm-rs";
pub const ADAPTER_VERSION: &str = "spektrafilm-rs-adapter-1";
pub const PARAMETER_SCHEMA_VERSION: &str = "spektrafilm-rs-params-1";
pub const FORK_REFERENCE: &str = "suraciii/spektrafilm-rs";
pub const FINISHED_JPEG_QUALITY: u8 = 85;
pub const MAX_EDGE: u32 = 9568;
pub const MAX_DECODED_BYTES: u64 = 2_147_483_648;

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
}

pub fn default_output(format: &str) -> OutputContract {
    let preview = format == "png";
    OutputContract {
        format: format.to_owned(),
        precision_bits: 8,
        color_space: "sRGB".to_owned(),
        transfer_function: "srgb".to_owned(),
        geometry: if preview { "bounded" } else { "input-preserving" }.to_owned(),
        encoding: if preview {
            "bounded-preview".to_owned()
        } else {
            format!("quality-{FINISHED_JPEG_QUALITY}-baseline")
        },
        max_edge: None,
    }
}

pub fn read_recipe(path: &Path) -> Result<RenderRecipe> {
    let file = fs::File::open(path).with_context(|| format!("opening recipe {}", path.display()))?;
    let recipe: RenderRecipe = serde_json::from_reader(file)
        .with_context(|| format!("parsing recipe {}", path.display()))?;
    if recipe.schema_version != PARAMETER_SCHEMA_VERSION {
        bail!("unsupported recipe schema {}", recipe.schema_version);
    }
    if recipe.film_profile.trim().is_empty() {
        bail!("filmProfile must not be empty");
    }
    validate_output(&recipe.output)?;
    Ok(recipe)
}

pub fn normalize_parameters(value: Value) -> Result<RuntimeParams> {
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
    let params: RuntimeParams = serde_json::from_value(Value::Object(object))
        .context("parameters do not match the spektrafilm-rs RuntimeParams schema")?;
    params.validate().map_err(anyhow::Error::msg)?;
    Ok(params)
}

pub fn validate_input_path(path: &Path) -> Result<()> {
    if ImageFormat::detect(path)? != ImageFormat::Tiff {
        bail!("render input must be a TIFF");
    }
    let file = fs::File::open(path)
        .with_context(|| format!("opening render input {}", path.display()))?;
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
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    let matches = match output.format.as_str() {
        "jpeg" => extension == "jpg" || extension == "jpeg",
        "png" => extension == "png",
        _ => false,
    };
    if !matches {
        bail!("output path extension does not match declared {} format", output.format);
    }
    Ok(())
}

pub fn validate_output(output: &OutputContract) -> Result<()> {
    let preview = output.format == "png";
    let expected = default_output(&output.format);
    if output.format != "jpeg" && !preview {
        bail!("unsupported output format {}; use jpeg or png", output.format);
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
    let defaults = serde_json::to_value(RuntimeParams::default()).expect("runtime defaults serialize");
    let controls = [
        "camera", "enlarger", "scanner", "film_render", "print_render", "io", "settings", "debug", "taps",
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
            "seedPolicy": "explicit-or-deterministic-default",
        },
        "errors": [
            "invalid-recipe", "unsupported-control", "incompatible-input", "resource-unavailable",
            "cancelled", "deadline", "engine-failure", "output-invalid", "outcome-unknown"
        ],
    })
}

#[cfg(test)]
mod tests {
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
        assert!(validate_output(&OutputContract {
            max_edge: Some(MAX_EDGE + 1),
            ..output
        }).is_err());
    }

    #[test]
    fn render_input_limits_reject_oversized_decodes() {
        assert!(validate_input_dimensions(MAX_EDGE, 1).is_ok());
        assert!(validate_input_dimensions(1024, 1024).is_ok());
        assert!(validate_input_dimensions(MAX_EDGE + 1, 1).is_err());
        assert!(validate_input_dimensions(20_000, 20_000).is_err());
    }
}
