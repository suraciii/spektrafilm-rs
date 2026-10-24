//! Public stage routing; numerical operations remain in the shared core runtime.
use super::*;
use spektrafilm_core::stage_artifact::{MaterialKind, StageHeader};
use spektrafilm_core::{WorkflowRoute, process_params, stage_artifact, stage_execution};

pub(super) fn run(args: &ProcessArgs, data: &DataSelection) -> Result<()> {
    if args.iters == 0 {
        bail!("--iters must be at least 1");
    }
    if args.input.is_dir() {
        validate_read_only_input(args)?;
    }
    let profile_sources: Vec<PathBuf> = args
        .film
        .iter()
        .chain(args.paper.iter())
        .map(|name| data.path.join("profiles").join(format!("{name}.json")))
        .collect();
    let mut protected = vec![args.input.as_path()];
    protected.extend(args.params.as_deref());
    protected.extend(args.raw_out.as_deref());
    protected.extend(
        args.preset
            .iter()
            .filter(|p| p.ends_with(".json") || p.ends_with(".toml"))
            .map(Path::new),
    );
    protected.extend(
        args.sources
            .iter()
            .filter(|p| p.ends_with(".json") || p.ends_with(".toml"))
            .map(Path::new),
    );
    protected.extend(profile_sources.iter().map(PathBuf::as_path));
    let mut diagnostics_protected = protected.clone();
    diagnostics_protected.extend(args.output.as_deref());
    telemetry::run(
        spektrafilm_core::telemetry::OperationKind::Process,
        if args.dry_run {
            None
        } else {
            args.diagnostics.as_deref()
        },
        args.gpu_timings,
        &diagnostics_protected,
        telemetry::requested_backend(args.backend),
        |invocation| execute(args, data, invocation, &protected),
    )?;
    Ok(())
}

fn execute(
    args: &ProcessArgs,
    data: &DataSelection,
    invocation: &mut telemetry::Invocation,
    protected: &[&Path],
) -> Result<()> {
    let header = if args.input.is_dir() {
        Some(stage_artifact::read_header(&args.input)?)
    } else {
        None
    };
    let route = WorkflowRoute::parse(
        args.workflow
            .route
            .as_deref()
            .context("stage processing requires --route")?,
    );
    if route == WorkflowRoute::Custom {
        bail!("unsupported workflow route");
    }
    if route.artifact_input() != header.as_ref().map(|h| h.state) {
        bail!("--route input node does not match the supplied input state");
    }
    let artifact_output = route.artifact_output().is_some();
    if artifact_output {
        validate_artifact_writer(args)?;
        if let Some(output) = args.output.as_deref() {
            stage_artifact::validate_destination(output)?;
            validate_destination_aliases(output, protected, args.diagnostics.as_deref())?;
        }
    }
    if header.is_some() {
        if args.workflow.raw_white_balance != "as-shot"
            || args.workflow.raw_temperature.is_some()
            || args.workflow.raw_tint.is_some()
            || args.workflow.lens_correction
        {
            bail!("RAW loading controls are inapplicable to stage artifact input");
        }
        if args.workflow.film_channel_swap != "none" || args.workflow.print_channel_swap != "none" {
            bail!("channel swaps reinterpret frozen material; regenerate from the earlier input");
        }
    } else if !args.input.is_file() {
        bail!("input is not a regular file: {}", args.input.display());
    }
    let input_is_raw = header.is_none() && image_io::is_raw(&args.input);
    if header.is_none() && !input_is_raw {
        image_io::ImageFormat::detect(&args.input)?;
    }
    let raw = raw_options(&args.workflow);
    if input_is_raw {
        validate_raw_options(&raw)?;
    }
    let library = spektrafilm_core::presets::PresetLibrary::default();
    let presets = args
        .preset
        .iter()
        .map(|p| preset::select(&library, p).map(|s| s.preset))
        .collect::<Result<Vec<_>>>()?;
    let features = args
        .feature_profiles
        .iter()
        .enumerate()
        .map(|(i, s)| {
            spektrafilm_core::feature_profiles::parse_selection(s, i + 1)
                .map_err(anyhow::Error::msg)
        })
        .collect::<Result<Vec<_>>>()?;
    let request = process_params::ProcessParamsRequest {
        film: args.film.as_deref(),
        paper: args.paper.as_deref(),
        presets: &presets,
        params_file: args.params.as_deref(),
        sources: &args.sources,
        feature_profiles: &features,
        route: args.workflow.route.as_deref(),
        scan_film: args.scan_film,
        scan_output: args.workflow.scan_output.as_deref(),
        input_is_raw,
        input_color_space: args.input_color_space.as_deref(),
        input_cctf_decoding: args.input_cctf_decoding,
        require_input_declaration: true,
        digest_mode: DigestMode::ApplyStockSpecifics,
    };
    let (mut params, film, print, declaration, edits, feature_report, film_name, print_name) =
        if let Some(header) = &header {
            let resolved = process_params::resolve_continuation(request, header, &data.path)
                .map_err(anyhow::Error::msg)?;
            let print_name = if route.prints() || header.state == MaterialKind::Print {
                resolved
                    .print
                    .as_ref()
                    .and_then(|p| p.info.stock.clone())
                    .or_else(|| {
                        header
                            .print
                            .as_ref()
                            .or(header.original_print.as_ref())
                            .and_then(|p| p.profile.info.stock.clone())
                    })
            } else {
                None
            };
            (
                resolved.params,
                header.film.profile.clone(),
                resolved.print,
                None,
                resolved.edits,
                json!([]),
                header
                    .film
                    .profile
                    .info
                    .stock
                    .clone()
                    .unwrap_or_else(|| "embedded film".into()),
                print_name,
            )
        } else {
            let resolved =
                process_params::resolve(request, &data.path).map_err(anyhow::Error::msg)?;
            let mut film = resolved.film;
            let mut print = resolved.print;
            apply_channel_swap(&mut film, &args.workflow.film_channel_swap)?;
            apply_channel_swap(&mut print, &args.workflow.print_channel_swap)?;
            let print = route.prints().then_some(print);
            let print_name = route.prints().then_some(resolved.print_name);
            (
                resolved.params,
                film,
                print,
                Some(resolved.input_declaration),
                resolved.edits,
                serde_json::to_value(resolved.feature_profiles)?,
                resolved.film_name,
                print_name,
            )
        };
    params.random_seed = args
        .seed
        .map(u64::from)
        .unwrap_or_else(|| header.as_ref().map_or(0, |h| h.seed));
    let seed_source = if args.seed.is_some() {
        "argument"
    } else if header.is_some() {
        "artifact"
    } else {
        "image_default"
    };
    if params.taps.inject.is_some() || params.taps.collect.is_some() {
        bail!("stage artifact routes reject explicit taps.inject and taps.collect");
    }
    let retained_print_selection =
        if header.is_none() && route.artifact_output() == Some(MaterialKind::Film) {
            args.paper.clone().or_else(|| {
                presets
                    .iter()
                    .rev()
                    .find_map(|p| p.print_profile.as_ref().map(|p| p.stock.clone()))
            })
        } else {
            None
        };
    if header.is_none() {
        spektrafilm_core::stages::grain_v3::validate_supported(&film, &params)
            .map_err(anyhow::Error::msg)?;
        stage_execution::validate_stage_params(&film, &params).map_err(anyhow::Error::msg)?;
    }
    let plan = if artifact_output {
        None
    } else {
        let plan = match args.output.as_deref() {
            Some(path) => Some(OutputPlan::from_flags(
                path,
                args.format,
                args.bit_depth,
                args.jpeg_quality,
                args.jpeg_subsampling,
                args.compression,
            )?),
            None => args
                .format
                .map(|format| {
                    OutputPlan::from_format(
                        format,
                        args.bit_depth,
                        args.jpeg_quality,
                        args.jpeg_subsampling,
                        args.compression,
                    )
                })
                .transpose()?,
        };
        if plan.is_none() {
            OutputPlan::validate_unresolved(
                args.bit_depth,
                args.jpeg_quality,
                args.jpeg_subsampling,
                args.compression,
            )?;
        }
        if let Some(space) = args.workflow.saving_color_space.as_deref() {
            spektrafilm_math::colorspace::resolve(space).map_err(anyhow::Error::msg)?;
        }
        let plan = plan.map(|p| {
            p.with_saving(
                &args.workflow,
                &params.io.output_color_space,
                params.io.output_cctf_encoding,
            )
        });
        if let Some(plan) = &plan {
            plan.validate_saving()?;
        }
        plan
    };
    process_report::warn_edits(&edits);
    if args.debug {
        eprintln!(
            "Debug: route={}, input state={}, seed={} ({}), computation precision={}; no execution observed{}",
            route.as_str(),
            header.as_ref().map_or("input", |h| kind_name(h.state)),
            params.random_seed,
            seed_source,
            if cfg!(feature = "precision-f64") {
                "f64"
            } else {
                "f32"
            },
            if args.dry_run { " (dry run)" } else { " yet" }
        );
        eprintln!(
            "Debug: inherited operations={}, planned operations={}",
            completed(header.as_ref()).join(", "),
            planned(route).join(", ")
        );
    }
    if args.dry_run {
        let readout = if params.io.scan_film {
            &film
        } else {
            print
                .as_ref()
                .or_else(|| {
                    header
                        .as_ref()
                        .and_then(|h| h.print.as_ref().map(|p| &p.profile))
                })
                .unwrap_or(&film)
        };
        let report = dry_report(
            args,
            data,
            header.as_ref(),
            route,
            &params,
            declaration,
            &film,
            readout,
            &film_name,
            print_name.as_deref(),
            &edits,
            feature_report,
            plan.as_ref(),
            input_is_raw,
            seed_source,
        )?;
        match args.dry_run_format.unwrap_or(DiscoveryFormat::Json) {
            DiscoveryFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
            DiscoveryFormat::Text => process_report::print_text(&report),
        }
        return Ok(());
    }
    let output = args
        .output
        .as_deref()
        .context("--output is required for execution")?;
    // Complete required member verification precedes compute-device creation.
    let mut restored = if header.is_some() {
        Some(stage_artifact::read(&args.input)?)
    } else {
        None
    };
    let input_identity = if artifact_output {
        Some(content_identity(
            if header.is_some() {
                args.input.join("stage.json")
            } else {
                args.input.clone()
            }
            .as_path(),
        )?)
    } else {
        None
    };
    let backend: Box<dyn spektrafilm_backend::ComputeBackend> = match args.backend {
        Some(Backend::Cpu) => Box::new(spektrafilm_backend::cpu_backend::CpuBackend),
        Some(Backend::Gpu) => Box::new(
            spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit()
                .ok_or_else(|| anyhow::anyhow!("GPU backend unavailable; use --backend cpu"))?,
        ),
        None => spektrafilm_gpu::select_backend().map_err(|e| anyhow::anyhow!("{e}"))?,
    };
    let context = invocation.operation.context();
    telemetry::observe_backend(backend.as_ref(), &context);
    eprintln!("Backend: {}", backend.name());
    if backend.is_gpu() {
        bail!(
            "selected GPU cannot execute material stage export or continuation; use --backend cpu"
        );
    }
    let seed = params.random_seed;
    let output_space = params.io.output_color_space.clone();
    let output_encoded = params.io.output_cctf_encoding;
    let mut metadata = None;
    let mut last = None;
    if header.is_some() {
        // Validate payload once; warm-cache attempts reuse the same restored state.
        let mut state = restored.take();
        let material = state.as_ref().expect("restored state");
        invocation
            .operation
            .set_input_dimensions(material.density.width, material.density.height);
        if context.enabled() {
            let selected = print
                .as_ref()
                .or_else(|| {
                    material
                        .header
                        .print
                        .as_ref()
                        .or(material.header.original_print.as_ref())
                        .map(|p| &p.profile)
                })
                .unwrap_or(&film);
            let mut configuration = spektrafilm_core::telemetry::RenderConfiguration::from_runtime(
                &params, &film, selected,
            );
            if route.scans_film() {
                configuration.print_stock = None;
            }
            invocation.operation.set_render_configuration(configuration);
        }
        if let Some(conversion) = &state
            .as_ref()
            .expect("restored state")
            .header
            .storage_conversion
        {
            eprintln!("Stage storage-to-computation precision conversion: {conversion:?}");
        }
        for index in 1..=args.iters {
            let input = if index == args.iters {
                state.take().expect("final state")
            } else {
                state.as_ref().expect("restored state").clone()
            };
            let (attempt, result) =
                telemetry::simulation_attempt(invocation, index as u64, |observation| {
                    let observed =
                        spektrafilm_backend::bind_backend(backend.as_ref(), observation.clone());
                    stage_execution::continue_stage(
                        input,
                        params.clone(),
                        print.clone(),
                        &data.path,
                        observed.as_ref(),
                        seed,
                    )
                    .map_err(anyhow::Error::msg)
                });
            last = Some(result?);
            invocation.operation.set_saved_attempt(attempt);
        }
    } else {
        let image = if input_is_raw {
            spektrafilm_raw::load(&args.input, &raw)
                .map_err(anyhow::Error::msg)?
                .image
        } else {
            let loaded = image_io::load(&args.input)?;
            metadata = loaded.metadata;
            loaded.image
        };
        invocation
            .operation
            .set_input_dimensions(image.width, image.height);
        let image = if params.settings.preview_mode {
            resize_for_preview(&image, params.settings.preview_max_size)
        } else {
            image
        };
        let runtime = if let Some(print) = print {
            Runtime::new(film, print, params, &data.path)
        } else {
            Runtime::new_film(film, params, &data.path)
        }
        .map_err(anyhow::Error::msg)?;
        if context.enabled() {
            let mut configuration = runtime.telemetry_configuration();
            if !route.prints() {
                configuration.print_stock = None;
            }
            invocation.operation.set_render_configuration(configuration);
        }
        let mut image = Some(image);
        for index in 1..=args.iters {
            let input = if index == args.iters {
                image.take().expect("final input")
            } else {
                image.as_ref().expect("prepared input").clone()
            };
            let (attempt, result) =
                telemetry::simulation_attempt(invocation, index as u64, |observation| {
                    let observed =
                        spektrafilm_backend::bind_backend(backend.as_ref(), observation.clone());
                    runtime
                        .process_stage(input, observed.as_ref(), seed)
                        .map(|state| stage_execution::StageResult::Material(Box::new(state)))
                        .map_err(anyhow::Error::msg)
                });
            last = Some(result?);
            invocation.operation.set_saved_attempt(attempt);
        }
    }
    let result = last.context("missing stage execution result")?;
    match result {
        stage_execution::StageResult::Material(mut state) => {
            state.header.provenance.input_identity =
                input_identity.expect("material content identity");
            if state.header.state == MaterialKind::Film {
                state.header.downstream_print_name = retained_print_selection;
            }
            stage_artifact::write(output, &state)?;
            invocation
                .operation
                .set_published_dimensions(state.density.width as u32, state.density.height as u32);
            eprintln!("Saved stage: {}", output.display());
            if let Some(path) = &args.raw_out {
                write_raw(path, &state.density).with_context(|| {
                    format!(
                        "primary artifact {} is already published; writing raw dump {} failed",
                        output.display(),
                        path.display()
                    )
                })?;
            }
        }
        stage_execution::StageResult::Image(image) => {
            let plan = plan.context("missing image output plan")?;
            let saving = image_io::convert_image(
                &image,
                &output_space,
                output_encoded,
                &plan.saving_space,
                plan.saving_encoded,
            )?;
            invocation.operation.set_saving_configuration(
                spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
                    plan.format.image_format(),
                    &plan.save_options(),
                ),
            );
            let prepared = image_io::prepare_save_observed(
                output,
                &saving,
                plan.save_options(),
                metadata.as_ref(),
                &context,
            )?;
            let report = prepared
                .publish(true)
                .map_err(|e| anyhow::Error::msg(e.to_string()))?;
            invocation
                .operation
                .set_published_dimensions(image.width as u32, image.height as u32);
            if let Some(warning) = report.metadata_warning {
                eprintln!("Warning: {warning}");
            }
            eprintln!(
                "Saved: {} ({}x{})",
                output.display(),
                image.width,
                image.height
            );
            if let Some(path) = &args.raw_out {
                write_raw(path, &image).with_context(|| {
                    format!(
                        "primary image {} is already published; writing raw dump {} failed",
                        output.display(),
                        path.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn write_raw(path: &Path, image: &ImageBuf) -> Result<()> {
    use std::io::Write;
    let values: Vec<f64> = image.data.iter().map(|&v| v as f64).collect();
    std::fs::File::create(path)?.write_all(bytemuck_cast_f64_to_bytes(&values))?;
    eprintln!(
        "raw f64 buffer: {} ({} values)",
        path.display(),
        values.len()
    );
    Ok(())
}

fn kind_name(kind: MaterialKind) -> &'static str {
    match kind {
        MaterialKind::Film => "film",
        MaterialKind::Print => "print",
    }
}
fn completed(header: Option<&StageHeader>) -> Vec<&'static str> {
    match header.map(|h| h.state) {
        None => vec![],
        Some(MaterialKind::Film) => vec!["film"],
        Some(MaterialKind::Print) => vec!["film", "print"],
    }
}
fn planned(route: WorkflowRoute) -> Vec<&'static str> {
    let mut operations = Vec::new();
    if route.exposes_film() {
        operations.push("film");
    }
    if route.prints() {
        operations.push("print");
    }
    if route.artifact_output().is_none() {
        operations.push("scan");
    }
    operations
}
fn validate_artifact_writer(args: &ProcessArgs) -> Result<()> {
    if args.format.is_some()
        || args.bit_depth.is_some()
        || args.jpeg_quality.is_some()
        || args.jpeg_subsampling.is_some()
        || args.compression.is_some()
        || args.workflow.saving_color_space.is_some()
        || args.workflow.saving_cctf_encoding.is_some()
    {
        bail!(
            "stage artifact output rejects image writer flags (--format, --bit-depth, JPEG/compression and saving color/transfer options)"
        );
    }
    Ok(())
}
fn validate_destination_aliases(
    output: &Path,
    protected: &[&Path],
    diagnostics: Option<&Path>,
) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let output = std::fs::canonicalize(parent)?.join(
        output
            .file_name()
            .context("artifact destination needs a directory name")?,
    );
    for input in protected.iter().copied().chain(diagnostics) {
        let absolute = if input.exists() {
            std::fs::canonicalize(input)?
        } else {
            std::path::absolute(input)?
        };
        if absolute == output || absolute.starts_with(&output) || output.starts_with(&absolute) {
            bail!(
                "artifact destination aliases an invocation input or output: {}",
                input.display()
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dry_report(
    args: &ProcessArgs,
    data: &DataSelection,
    header: Option<&StageHeader>,
    route: WorkflowRoute,
    params: &spektrafilm_core::params::RuntimeParams,
    declaration: Option<process_params::InputDeclaration>,
    film: &profile::Profile,
    readout: &profile::Profile,
    film_name: &str,
    print_name: Option<&str>,
    edits: &[process_params::EditFeedback],
    features: serde_json::Value,
    plan: Option<&OutputPlan>,
    input_is_raw: bool,
    seed_source: &str,
) -> Result<serde_json::Value> {
    let artifact_output = route.artifact_output().is_some();
    let mut unchecked = vec!["compute_device", "output_geometry"];
    if header.is_some() {
        unchecked.extend(["artifact_payload", "artifact_integrity"]);
    } else {
        unchecked.extend(["pixel_decode", "metered_exposure"]);
    }
    if !artifact_output && plan.is_none() {
        unchecked.push("output_format");
    }
    let artifact = header.map(|h| json!({
        "state": h.state, "boundary": h.boundary, "schema_version": h.schema_version,
        "representation_version": h.representation_version, "producer_precision": h.provenance.computation_precision,
        "film": {"name": h.film.profile.info.stock, "sha256": h.film.sha256},
        "print": h.print.as_ref().map(|p| json!({"name":p.profile.info.stock,"sha256":p.sha256}))
    }));
    let mut input = json!({"kind": if header.is_some() {"stage_artifact"} else if input_is_raw {"raw"} else {"raster"},
        "declaration": declaration, "raw_loading": input_is_raw.then(|| json!({"white_balance":args.workflow.raw_white_balance,"temperature":args.workflow.raw_temperature,"tint":args.workflow.raw_tint,"lens_correction":args.workflow.lens_correction}))});
    if let Some(artifact) = artifact {
        input["artifact"] = artifact;
    }
    let mut output = json!({"kind":if artifact_output {"stage_artifact"} else {"image"},"path":args.output,
        "format":if artifact_output {Some("sfstage")} else {plan.map(|p|p.format.name())},
        "bit_depth":plan.map(|p|p.depth.bits()),"color_space":plan.map(|p|&p.saving_space),
        "cctf_encoding":plan.map(|p|p.saving_encoded),"compression":null,"jpeg_quality":plan.and_then(|p|p.jpeg_quality),"jpeg_subsampling":null});
    if let Some(plan) = plan {
        output["compression"] = json!(plan.compression.map(|v| match v {
            Compression::Zip => "zip",
            Compression::None => "none",
        }));
        output["jpeg_subsampling"] = json!(plan.jpeg_subsampling.map(|v| match v {
            JpegSubsampling::Yuv444 => "444",
            JpegSubsampling::Yuv420 => "420",
        }));
    }
    if !artifact_output && plan.is_none() {
        output["bit_depth"] = json!(
            OutputPlan::validate_unresolved(
                args.bit_depth,
                args.jpeg_quality,
                args.jpeg_subsampling,
                args.compression
            )?
            .map(|depth| depth.bits())
        );
        output["color_space"] = json!(args.workflow.saving_color_space);
        output["cctf_encoding"] = json!(args.workflow.saving_cctf_encoding);
    }
    let capture_source = if params.camera.capture_illuminant.is_none() {
        "unset"
    } else if edits
        .iter()
        .any(|edit| edit.path == "camera.capture_illuminant" && edit.source.kind == "set")
    {
        "set"
    } else {
        "preset"
    };
    let dye_active = spektrafilm_core::stages::grain_v3::active(params);
    Ok(
        json!({"data":{"path":data.path,"source":data.source},"film_profile":film_name,"print_profile":print_name,
        "parameters":params,"scanner_provenance":ScannerProvenance::from_params(params),"feature_profiles":features,
        "film_grain_dye":{"active":dye_active,"effective_backend":null,"note":dye_active.then_some("Dye executor resolves when rendering; compute-device availability is unchecked.")},
        "film_grain_fotufilm":spektrafilm_core::stages::grain_fotufilm::configuration_report(params),
        "capture_illuminant":{"requested":params.camera.capture_illuminant,"effective":route.exposes_film().then(||params.camera.capture_illuminant.as_deref().unwrap_or(&film.info.reference_illuminant)),"source":capture_source,"profile_reference_illuminant":film.info.reference_illuminant,"status":if route.exposes_film(){"active"}else{"inactive"},"reason":if route.exposes_film(){None}else{Some("route computes no film exposure from scene RGB")}},
        "readout_illuminant":{"effective":readout.readout_illuminant(),"rule":readout.readout_rule()},
        "input":input,"output":output,"execution":{"input_state":header.map_or("input",|h|kind_name(h.state)),"completed_operations":completed(header),"planned_operations":planned(route)},
        "seed":params.random_seed,"seed_source":seed_source,"edits":edits,"unchecked":unchecked,
        "backend":{"requested":args.backend.map(|b|match b{Backend::Cpu=>"cpu",Backend::Gpu=>"gpu"}),"policy":args.backend.map_or("auto",|b|match b{Backend::Cpu=>"cpu",Backend::Gpu=>"gpu"}),"environment":std::env::var("SPEKTRAFILM_BACKEND").ok()}}),
    )
}

fn validate_read_only_input(args: &ProcessArgs) -> Result<()> {
    let root = std::fs::canonicalize(&args.input)?;
    let header = stage_artifact::read_header(&args.input)?;
    let mut members = vec![root.join("stage.json")];
    members.extend(
        header
            .density
            .iter()
            .chain(header.prepared_source.iter())
            .map(|member| root.join(&member.path)),
    );
    for destination in args
        .output
        .as_deref()
        .into_iter()
        .chain(args.raw_out.as_deref())
        .chain(args.diagnostics.as_deref())
    {
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let resolved = if destination.exists() {
            std::fs::canonicalize(destination)?
        } else {
            std::fs::canonicalize(parent)?.join(
                destination
                    .file_name()
                    .context("output path needs a file name")?,
            )
        };
        if members
            .iter()
            .any(|member| spektrafilm_core::telemetry::paths_alias(destination, member))
        {
            bail!(
                "stage artifact input is read-only; destination aliases a required member: {}",
                destination.display()
            );
        }
        if resolved == root || resolved.starts_with(&root) {
            bail!(
                "stage artifact input is read-only; destination is inside {}",
                args.input.display()
            );
        }
    }
    Ok(())
}

fn content_identity(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("sha256-{:x}", digest.finalize()))
}
