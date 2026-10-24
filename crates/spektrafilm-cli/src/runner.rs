use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::ScannerProvenance;
use spektrafilm_core::image_io::{self, BitDepth, SaveOptions};
use spektrafilm_core::params_builder::resize_for_preview;
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, RuntimePhotoParams};

use crate::contract;
use crate::telemetry;
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory, OperationKind};

pub fn cmd_render(
    input: &Path,
    recipe_path: &Path,
    output: &Path,
    data_dir: &Path,
    diagnostics: Option<&Path>,
    gpu_timings: bool,
    matches: &clap::ArgMatches,
) -> Result<()> {
    let facts = telemetry::run(
        OperationKind::Render,
        diagnostics,
        gpu_timings,
        &[input, recipe_path, output],
        telemetry::requested_backend(None),
        |invocation| {
            if output.exists() {
                bail!("refusing to replace existing output {}", output.display());
            }
            invocation.boundary(IssueCategory::Input, IssueBoundary::InputLoad);
            if !input.is_file() {
                bail!("input is not a regular file: {}", input.display());
            }
            invocation.boundary(IssueCategory::Configuration, IssueBoundary::Operation);
            let data = crate::select_data_dir(data_dir.to_owned(), matches)?;
            let recipe = contract::read_recipe(recipe_path)?;
            let facts = render_recipe(input, &recipe, output, &data.path, invocation)?;
            invocation.phase(
                "result_prepare",
                IssueCategory::Publication,
                IssueBoundary::ResultPrepare,
                &invocation.operation.context(),
                |_| Ok(serde_json::to_string_pretty(&facts)?),
            )
        },
    )?;
    println!("{facts}");
    Ok(())
}

fn render_recipe(
    input: &Path,
    recipe: &contract::RenderRecipe,
    output: &Path,
    data_dir: &Path,
    invocation: &mut telemetry::Invocation,
) -> Result<Value> {
    let context = invocation.operation.context();
    invocation.boundary(IssueCategory::Configuration, IssueBoundary::Operation);
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
    // The selected backend is initialized here so the render and the
    // published resource report read the same backend selection.
    let backend = invocation.phase(
        "backend_init",
        IssueCategory::Backend,
        IssueBoundary::BackendInit,
        &context,
        |backend_context| {
            let backend = spektrafilm_gpu::select_backend().map_err(|e| anyhow::anyhow!("{e}"))?;
            telemetry::observe_backend(backend.as_ref(), backend_context);
            Ok(backend)
        },
    )?;
    invocation.boundary(IssueCategory::Configuration, IssueBoundary::RuntimePrepare);
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
    let runtime = invocation.phase(
        "runtime_prepare",
        IssueCategory::Configuration,
        IssueBoundary::RuntimePrepare,
        &context,
        |prepare_context| {
            photo
                .into_runtime_observed_with_backend(
                    DigestMode::ApplyStockSpecifics,
                    backend.as_ref(),
                    prepare_context,
                )
                .map_err(anyhow::Error::msg)
                .context("building spektrafilm-rs spectral runtime")
        },
    )?;
    if context.enabled() {
        invocation
            .operation
            .set_render_configuration(runtime.telemetry_configuration());
    }
    let loaded = invocation.phase(
        "input_load",
        IssueCategory::Input,
        IssueBoundary::InputLoad,
        &context,
        |_| {
            let loaded = image_io::load(input)
                .with_context(|| format!("loading staged input {}", input.display()))?;
            contract::validate_input_dimensions(loaded.image.width, loaded.image.height)?;
            Ok(loaded)
        },
    )?;
    invocation
        .operation
        .set_input_dimensions(loaded.image.width, loaded.image.height);
    let input_size = [loaded.image.width, loaded.image.height];
    let params = runtime.params();
    let image = invocation.phase(
        "input_prepare",
        IssueCategory::Input,
        IssueBoundary::InputPrepare,
        &context,
        |_| {
            let image = if recipe.output.format == "png" && !params.scanner.requires_full_frame() {
                resize_for_preview(&loaded.image, params.settings.preview_max_size)
            } else {
                loaded.image
            };
            Ok(image)
        },
    )?;
    let (attempt_id, result) = telemetry::simulation_attempt(invocation, 1, |simulation_context| {
        runtime
            .process_observed(image, backend.as_ref(), simulation_context)
            .map_err(anyhow::Error::msg)
            .context("running spektrafilm-rs runtime")
    });
    let result = result?;
    let result = if recipe.output.format == "png"
        && params.scanner.requires_full_frame()
        && params
            .taps
            .collect
            .as_deref()
            .is_none_or(|tap| tap == "rgb_out")
    {
        resize_for_preview(&result, params.settings.preview_max_size)
    } else {
        result
    };
    invocation.operation.set_saved_attempt(attempt_id);
    invocation.boundary(IssueCategory::Writing, IssueBoundary::FileWrite);
    let parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("output has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let jpeg_output = recipe.output.format == "jpeg";
    let options = SaveOptions {
        depth: BitDepth::Eight,
        color_space: "sRGB",
        cctf_encoding: true,
        jpeg_quality: jpeg_output.then_some(contract::FINISHED_JPEG_QUALITY),
        jpeg_subsampling: jpeg_output
            .then_some(spektrafilm_core::image_io::JpegSubsampling::Yuv444),
        compression: None,
    };
    let format = image_io::ImageFormat::detect(output)?;
    invocation.operation.set_saving_configuration(
        spektrafilm_core::telemetry::SavingConfiguration::from_save_options(format, &options),
    );
    let write_result = invocation.phase(
        "file_write",
        IssueCategory::Writing,
        IssueBoundary::FileWrite,
        &context,
        |write_context| {
            image_io::prepare_save_observed(
                output,
                &result,
                options,
                loaded.metadata.as_ref(),
                write_context,
            )
            .map_err(anyhow::Error::from)
        },
    );
    let prepared = write_result.context("writing spektrafilm-rs output")?;
    if let Some(warning) = &prepared.report().metadata_warning {
        eprintln!("Warning: {warning}");
    }
    invocation.phase(
        "publication",
        IssueCategory::Publication,
        IssueBoundary::Publication,
        &context,
        |_| {
            prepared
                .publish(false)
                .context("committing spektrafilm-rs output")?;
            Ok(())
        },
    )?;
    invocation
        .operation
        .set_published_dimensions(result.width, result.height);
    invocation.phase(
        "result_prepare",
        IssueCategory::Publication,
        IssueBoundary::ResultPrepare,
        &context,
        |_| {
            let bytes = fs::read(output)?;
            Ok(json!({
                "implementation": contract::IMPLEMENTATION,
                "adapterVersion": contract::ADAPTER_VERSION,
                "parameterSchemaVersion": contract::PARAMETER_SCHEMA_VERSION,
                "modelVersion": spektrafilm_core::runtime::MODEL_VERSION,
                "parameterDigest": contract::parameter_digest(&recipe.parameters),
                "scannerProvenance": ScannerProvenance::from_params(params),
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
        },
    )
}
