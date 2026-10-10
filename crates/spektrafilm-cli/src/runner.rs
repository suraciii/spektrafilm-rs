use std::path::{Path, PathBuf};
use std::{fs, io::Write};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::image_io::{self, BitDepth, SaveOptions};
use spektrafilm_core::params_builder::resize_for_preview;
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, RuntimePhotoParams};

use crate::contract;

pub(super) fn cmd_render(
    input: &Path,
    recipe_path: &Path,
    output: &Path,
    data_dir: &Path,
) -> Result<()> {
    if output.exists() {
        bail!("refusing to replace existing output {}", output.display());
    }
    if !input.is_file() {
        bail!("input is not a regular file: {}", input.display());
    }
    let recipe = contract::read_recipe(recipe_path)?;
    let facts = render_recipe(input, &recipe, output, data_dir)?;
    println!("{}", serde_json::to_string_pretty(&facts)?);
    Ok(())
}

fn render_recipe(
    input: &Path,
    recipe: &contract::RenderRecipe,
    output: &Path,
    data_dir: &Path,
) -> Result<Value> {
    contract::validate_output(&recipe.output)?;
    contract::validate_output_path(output, &recipe.output)?;
    contract::validate_input_path(input)?;
    if let Some(seed) = recipe.seed {
        contract::validate_seed(seed)?;
    }
    let mut params = contract::normalize_parameters(recipe.parameters.clone())?;
    params.random_seed = recipe.seed.unwrap_or(0);
    if recipe.output.format == "png" {
        params.settings.preview_mode = true;
        params.settings.preview_max_size = recipe.output.max_edge.unwrap_or(640);
    }
    params.io.input_color_space = "ProPhoto RGB".into();
    params.io.input_cctf_decoding = false;
    params.io.output_color_space = "sRGB".into();
    params.io.output_cctf_encoding = true;
    params.io.scan_film = false;
    params.validate_color().map_err(anyhow::Error::msg)?;
    let film = profile::load_profile_by_name(data_dir, &recipe.film_profile)
        .with_context(|| format!("loading film profile {}", recipe.film_profile))?;
    let print_name = recipe
        .print_profile
        .as_deref()
        .or(film.info.target_print.as_deref())
        .ok_or_else(|| anyhow::anyhow!("recipe does not select a print profile"))?;
    let print = profile::load_profile_by_name(data_dir, print_name)
        .with_context(|| format!("loading print profile {print_name}"))?;
    let photo = RuntimePhotoParams {
        film,
        print,
        params,
        data_dir: data_dir.to_owned(),
    };
    let runtime = photo
        .into_runtime(DigestMode::ApplyStockSpecifics)
        .map_err(anyhow::Error::msg)
        .context("building spektrafilm-rs spectral runtime")?;
    let params = runtime.params();
    let loaded = image_io::load(input)
        .with_context(|| format!("loading staged input {}", input.display()))?;
    contract::validate_input_dimensions(loaded.image.width, loaded.image.height)?;
    let input_size = [loaded.image.width, loaded.image.height];
    let image = if recipe.output.format == "png" {
        resize_for_preview(&loaded.image, params.settings.preview_max_size)
    } else {
        loaded.image
    };
    let backend = spektrafilm_gpu::select_backend();
    let result = runtime
        .process(image, backend.as_ref())
        .map_err(anyhow::Error::msg)
        .context("running spektrafilm-rs runtime")?;
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let extension = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tmp");
    let temporary = parent.join(format!(
        ".spektrafilm-{}-{}.{}",
        std::process::id(),
        format!(
            "{:x}",
            Sha256::digest(output.as_os_str().as_encoded_bytes())
        ),
        extension
    ));
    if temporary.exists() {
        bail!("temporary output already exists: {}", temporary.display());
    }
    let jpeg_output = recipe.output.format == "jpeg";
    if let Err(error) = image_io::save(
        &temporary,
        &result,
        SaveOptions {
            depth: BitDepth::Eight,
            color_space: "sRGB",
            cctf_encoding: true,
            jpeg_quality: jpeg_output.then_some(contract::FINISHED_JPEG_QUALITY),
            jpeg_subsampling: jpeg_output
                .then_some(spektrafilm_core::image_io::JpegSubsampling::Yuv444),
            compression: None,
        },
        loaded.metadata.as_ref(),
    ) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("writing spektrafilm-rs output");
    }
    if let Err(error) = fs::rename(&temporary, output) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("committing spektrafilm-rs output");
    }
    let bytes = fs::read(output)?;
    Ok(json!({
        "implementation": contract::IMPLEMENTATION,
        "adapterVersion": contract::ADAPTER_VERSION,
        "parameterSchemaVersion": contract::PARAMETER_SCHEMA_VERSION,
        "parameterDigest": contract::parameter_digest(&recipe.parameters),
        "input": {"path": input, "width": input_size[0], "height": input_size[1]},
        "output": {
            "path": output,
            "format": recipe.output.format,
            "width": result.width,
            "height": result.height,
            "size": bytes.len(),
            "sha256": format!("{:x}", Sha256::digest(&bytes)),
        },
        "seed": recipe.seed,
        "resource": {"backend": backend.name()},
        "terminalOutcome": "succeeded",
    }))
}

pub(super) fn cmd_parity(corpus: &Path, report: &Path, data_dir: &Path) -> Result<()> {
    if report.exists() {
        bail!("refusing to replace existing report {}", report.display());
    }
    let base = if corpus.is_dir() {
        corpus
    } else {
        corpus.parent().unwrap_or_else(|| Path::new("."))
    };
    let documents = if corpus.is_file() {
        vec![(
            corpus.to_path_buf(),
            serde_json::from_slice::<Value>(&fs::read(corpus)?)?,
        )]
    } else if corpus.is_dir() {
        let mut paths = fs::read_dir(corpus)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                Ok((
                    path.clone(),
                    serde_json::from_slice::<Value>(&fs::read(&path)?)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        bail!(
            "corpus is neither a file nor a directory: {}",
            corpus.display()
        );
    };
    let mut cases = Vec::new();
    for (source, document) in documents {
        let entries = document
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![document]);
        for (index, entry) in entries.into_iter().enumerate() {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("case");
            let result = run_parity_case(base, &entry, data_dir, report, cases.len() + index);
            let case = match result {
                Ok(case) => case,
                Err(error) => json!({
                    "name": name,
                    "source": source,
                    "status": "failed",
                    "passed": false,
                    "failure": error.to_string(),
                }),
            };
            cases.push(case);
        }
    }
    let passed = !cases.is_empty()
        && cases
            .iter()
            .all(|case| case["passed"].as_bool() == Some(true));
    let document = json!({
        "implementation": contract::IMPLEMENTATION,
        "adapterVersion": contract::ADAPTER_VERSION,
        "parameterSchemaVersion": contract::PARAMETER_SCHEMA_VERSION,
        "corpus": corpus,
        "passed": passed,
        "status": if passed { "passed" } else { "failed" },
        "cases": cases,
        "dataDir": data_dir,
    });
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(report)?;
    file.write_all(serde_json::to_string_pretty(&document)?.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn run_parity_case(
    base: &Path,
    entry: &Value,
    data_dir: &Path,
    report: &Path,
    index: usize,
) -> Result<Value> {
    let input = resolve_case_path(base, entry.get("input"))?;
    let recipe = resolve_case_path(base, entry.get("recipe"))?;
    let input_digest = format!("{:x}", Sha256::digest(fs::read(&input)?));
    let recipe_bytes = fs::read(&recipe)?;
    let recipe_digest = format!("{:x}", Sha256::digest(&recipe_bytes));
    let recipe_value: contract::RenderRecipe = serde_json::from_slice(&recipe_bytes)?;
    let extension = if recipe_value.output.format == "jpeg" {
        "jpg"
    } else {
        "png"
    };
    let output = report.with_file_name(format!(
        ".spektrafilm-parity-{}-{index}.{extension}",
        std::process::id()
    ));
    if output.exists() {
        bail!("parity output already exists: {}", output.display());
    }
    let render = render_recipe(&input, &recipe_value, &output, data_dir);
    let result = match render {
        Ok(facts) => {
            let reference_path = entry
                .get("reference")
                .map(|reference| resolve_case_path(base, Some(reference)))
                .transpose()?;
            let reference_digest = if let Some(path) = &reference_path {
                Some(format!("{:x}", Sha256::digest(fs::read(path)?)))
            } else {
                entry
                    .get("referenceSha256")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            };
            let image_diff = if let Some(path) = &reference_path {
                Some(compare_images(
                    path,
                    &output,
                    entry
                        .get("pixelTolerance")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.02),
                )?)
            } else {
                None
            };
            let passed = if let Some(diff) = &image_diff {
                diff["dimensionsMatch"].as_bool() == Some(true)
                    && diff["maxAbs"].as_f64().is_some_and(|value| {
                        value
                            <= entry
                                .get("pixelTolerance")
                                .and_then(Value::as_f64)
                                .unwrap_or(0.02)
                    })
            } else {
                reference_digest == facts["output"]["sha256"].as_str().map(ToOwned::to_owned)
            };
            json!({
                "name": entry.get("name").and_then(Value::as_str).unwrap_or("case"),
                "status": if passed { "passed" } else if reference_digest.is_some() { "mismatch" } else { "unqualified" },
                "passed": passed,
                "inputSha256": input_digest,
                "recipeSha256": recipe_digest,
                "referenceSha256": reference_digest,
                "imageDiff": image_diff,
                "output": facts["output"],
                "resource": facts["resource"],
            })
        }
        Err(error) => json!({
            "name": entry.get("name").and_then(Value::as_str).unwrap_or("case"),
            "status": "failed",
            "passed": false,
            "inputSha256": input_digest,
            "recipeSha256": recipe_digest,
            "failure": error.to_string(),
        }),
    };
    let _ = fs::remove_file(&output);
    Ok(result)
}

fn resolve_case_path(base: &Path, value: Option<&Value>) -> Result<PathBuf> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("parity case requires a string path"))?;
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

fn compare_images(reference: &Path, output: &Path, tolerance: f64) -> Result<Value> {
    let reference_image = image_io::load(reference)
        .with_context(|| format!("loading parity reference {}", reference.display()))?;
    let output_image = image_io::load(output)
        .with_context(|| format!("loading parity output {}", output.display()))?;
    if reference_image.image.data.len() != output_image.image.data.len() {
        return Ok(json!({
            "dimensionsMatch": false,
            "maxAbs": null,
            "rmse": null,
            "tolerance": tolerance,
        }));
    }
    let mut max_abs: f64 = 0.0;
    let mut sum_squared: f64 = 0.0;
    for (reference, output) in reference_image
        .image
        .data
        .iter()
        .zip(&output_image.image.data)
    {
        let delta = *reference as f64 - *output as f64;
        max_abs = max_abs.max(delta.abs());
        sum_squared += delta * delta;
    }
    let count = reference_image.image.data.len() as f64;
    Ok(json!({
        "dimensionsMatch": reference_image.image.width == output_image.image.width
            && reference_image.image.height == output_image.image.height,
        "maxAbs": max_abs,
        "rmse": (sum_squared / count).sqrt(),
        "tolerance": tolerance,
    }))
}
