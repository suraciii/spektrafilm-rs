use crate::{Error, Result, common, documents};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Command adapter for the existing case configuration and suite language.
pub struct LocalInput {
    pub input: PathBuf,
    pub configuration: PathBuf,
    pub input_kind: String,
    pub input_color_space: Option<String>,
    pub input_cctf_decoding: Option<bool>,
    pub seeds: Vec<u32>,
    pub regions: Option<PathBuf>,
    pub reference: Option<PathBuf>,
    pub reference_info: Option<PathBuf>,
    pub preparation: Option<PathBuf>,
}

fn resolve_paths(value: &mut Value, document: &Path) -> Result<()> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == "path" || key == "params" {
                    if let Some(path) = child.as_str() {
                        *child = json!(authored_path(path, document)?);
                    }
                } else if key == "presets" || key == "set_files" {
                    for path in child.as_array_mut().into_iter().flatten() {
                        let text = path.as_str().ok_or_else(|| {
                            Error::new("invalid_path", "File path must be a string")
                        })?;
                        *path = json!(authored_path(text, document)?);
                    }
                } else {
                    resolve_paths(child, document)?;
                }
            }
        }
        Value::Array(array) => {
            for child in array {
                resolve_paths(child, document)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn authored_path(path: &str, document: &Path) -> Result<PathBuf> {
    Ok(documents::normalize_path(document)?
        .parent()
        .unwrap_or(Path::new("/"))
        .join(path))
}

pub fn load(kind: &str, document: &Path) -> Result<Value> {
    let mut value = documents::load_json_value(document)?;
    documents::validate_shape(kind, &value)?;
    resolve_paths(&mut value, document)?;
    // Validate dependencies through the same suite loader, with a real input
    // supplied only by the run adapter. Standalone records retain their shape.
    documents::validate_local_dependencies(kind, &value, document)?;
    Ok(value)
}

impl LocalInput {
    pub fn suite(&self) -> Result<Value> {
        if self.input_kind != "raster" && self.input_kind != "raw" {
            return Err(Error::new(
                "source_declaration",
                "Input kind must be raster or raw",
            ));
        }
        let input_path = documents::normalize_path(&self.input)?;
        let mut input = json!({"kind":self.input_kind,"path":input_path,"sha256":common::sha256_file(&input_path)?});
        if self.input_kind == "raster" {
            input["color_space"] =
                json!(self.input_color_space.as_ref().ok_or_else(|| Error::new(
                    "source_declaration",
                    "Raster input requires --input-color-space"
                ))?);
            input["cctf_decoding"] = json!(self.input_cctf_decoding.ok_or_else(|| Error::new(
                "source_declaration",
                "Raster input requires --input-cctf-decoding"
            ))?);
        } else {
            if self.input_color_space.is_some() || self.input_cctf_decoding.is_some() {
                return Err(Error::new(
                    "source_declaration",
                    "RAW input must not have raster color declarations",
                ));
            }
            input["adapter"] = json!("spektrafilm-raw");
        }
        let seeds = if self.seeds.is_empty() {
            vec![0]
        } else {
            self.seeds.clone()
        };
        let configuration = load("configuration", &self.configuration)?;
        // `--regions` declares a regions document; the case embeds its
        // regions array, so one region shape serves both the standalone
        // document and the case element.
        let regions = match self.regions.as_ref() {
            Some(path) => load("regions", path)?
                .get("regions")
                .cloned()
                .unwrap_or_else(|| json!([])),
            None => json!([]),
        };
        let mut case = json!({"id":"image","kind":"scene","input":input,"configuration":configuration,"seeds":seeds,"regions":regions,"measurements":["numeric","color","tone","flip"]});
        match (&self.reference, &self.reference_info) {
            (Some(image), Some(info)) => {
                let mut reference = load("reference", info)?;
                let path = documents::normalize_path(image)?;
                reference["path"] = json!(path);
                reference["sha256"] = json!(common::sha256_file(&path)?);
                case["reference"] = reference;
            }
            (None, None) => {}
            _ => {
                return Err(Error::new(
                    "reference_declaration",
                    "--reference and --reference-info require each other",
                ));
            }
        }
        if let Some(path) = &self.preparation {
            case["preparation"] = load("preparation", path)?;
        }
        let suite = json!({"schema_version":1,"id":"local-image","protocol":{"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0,0.4,0.16],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":4.0,"border_px":1},"spectrum":{"window":"hann"}},"hotspots":{"count":6,"min_area_px":151}},"cases":[case]});
        documents::validate_suite(&suite, &documents::normalize_path(&self.configuration)?)?;
        Ok(suite)
    }
}
