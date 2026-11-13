//! Development measurement: readings from declared sources, through the
//! shared parameter surface and the core-owned execution path. The commands
//! report values; they never fit, render, or judge.

pub mod run;
pub mod stimuli;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use spektrafilm_core::WorkflowRoute;

pub use spektrafilm_benchmark::readings::{
    StageAvailability, StageReading, Statistics, read_stage_bundle, sha256_file, statistics,
};
pub use spektrafilm_benchmark::readings::{
    csv_field, number_field, publish_staged, write_measure_named, write_named,
};
use spektrafilm_core::params::{RuntimeParams, Tap};
use spektrafilm_core::presets::PresetLibrary;
use spektrafilm_core::process_params::{
    self, FilmSelection, InputDeclaration, ProcessParamsRequest,
};
use spektrafilm_core::profile::Profile;
use spektrafilm_core::runtime::DigestMode;

use super::cli::{ChainArgs, ChainTap, MeasureEntry, MeasureStage};

/// The input declaration a measurement supplies to the shared resolver.
pub struct InputRequest<'a> {
    /// The declared color space of the measurement's values, when the entry
    /// needs one.
    pub color_space: Option<&'a str>,
    /// Whether the declared values are transfer-encoded.
    pub cctf_decoding: bool,
    /// Whether the chain requires a full input declaration.
    pub require: bool,
}

/// A resolved measurement chain: profiles, effective parameters, identity.
pub struct ResolvedChain {
    pub film: Profile,
    pub print: Profile,
    pub params: RuntimeParams,
    pub film_name: String,
    pub print_name: String,
    /// The effective input declaration and where it came from.
    pub declaration: InputDeclaration,
    /// Winning `--set` edits with their effective status.
    pub edits: Vec<Value>,
    /// The `--set` sources of this invocation, in order, with file paths
    /// canonicalized so a re-resolution reads the same files.
    pub sources: Vec<String>,
    /// The `--preset` selections of this invocation, in order, with file paths
    /// canonicalized.
    pub presets: Vec<String>,
    /// Every declared parameter file: canonical path and content hash.
    pub files: Vec<Value>,
    /// Which stock-specific application the resolution used.
    pub digest_mode: &'static str,
}

impl ResolvedChain {
    /// The requested value of one `--set` leaf, when a source supplied it.
    pub fn requested_edit(&self, path: &str) -> Option<&Value> {
        self.edits
            .iter()
            .find(|edit| edit.get("path").and_then(Value::as_str) == Some(path))
            .and_then(|edit| edit.get("requested"))
    }
}

/// Resolve the measurement chain through the shared parameter surface.
pub fn resolve_chain(
    args: &ChainArgs,
    data_dir: &Path,
    input: InputRequest<'_>,
) -> Result<ResolvedChain> {
    if args.preset.is_empty() && args.film.is_none() {
        return Err(anyhow!(
            "a chain declaration is required: pass --preset or --film"
        ));
    }
    if args.paper.is_some() && args.preset.is_empty() && args.film.is_none() {
        return Err(anyhow!("--paper requires --preset or --film"));
    }
    let library = PresetLibrary::default();
    let mut presets = Vec::with_capacity(args.preset.len());
    let mut preset_selectors = Vec::with_capacity(args.preset.len());
    for selector in &args.preset {
        let selected = crate::preset::select(&library, selector)?;
        let replay_selector = match selected.path {
            Some(path) => std::fs::canonicalize(&path)
                .with_context(|| format!("resolving selected preset: {}", path.display()))?
                .display()
                .to_string(),
            None => format!("built-in:{}", selected.preset.name),
        };
        preset_selectors.push(replay_selector);
        presets.push(selected.preset);
    }
    // The user binaries drop the stock-specific overrides of a profile only
    // when this internal switch asks for a frozen protocol; a measurement
    // must resolve the same chain as the run it describes.
    let preserve_user_edits = std::env::var_os("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
        .as_deref()
        == Some(std::ffi::OsStr::new("1"));
    let digest_mode = if preserve_user_edits {
        DigestMode::PreserveUserEdits
    } else {
        DigestMode::ApplyStockSpecifics
    };
    let resolved = process_params::resolve(
        ProcessParamsRequest {
            film: args.film.as_deref().map(FilmSelection::Name),
            paper: args.paper.as_deref(),
            presets: &presets,
            params_file: None,
            sources: &args.sources,
            feature_profiles: &[],
            route: args.route.as_deref(),
            scan_film: false,
            input_is_raw: false,
            scan_output: args.scan_output.as_deref(),
            input_color_space: input.color_space,
            input_cctf_decoding: Some(input.cctf_decoding),
            require_input_declaration: input.require,
            digest_mode,
        },
        data_dir,
    )
    .map_err(anyhow::Error::msg)
    .context("resolving the measurement chain")?;
    let edits = serde_json::to_value(&resolved.edits)
        .map_err(anyhow::Error::msg)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(ResolvedChain {
        film: resolved.film,
        print: resolved.print,
        params: resolved.params,
        film_name: resolved.film_name,
        print_name: resolved.print_name,
        declaration: resolved.input_declaration,
        edits,
        sources: canonical_declared(&args.sources),
        files: declared_file_records(&preset_selectors, &args.sources)?,
        presets: preset_selectors,
        digest_mode: if preserve_user_edits {
            "preserve_user_edits"
        } else {
            "apply_stock_specifics"
        },
    })
}

/// The latest stage a reading reaches: the boundary the measured range must
/// collect, and the default collection boundary.
pub fn last_read_stage(read: &[String]) -> Result<MeasureStage> {
    MeasureStage::ALL
        .iter()
        .copied()
        .filter(|stage| read.iter().any(|id| stage.id() == id))
        .max()
        .context("a measurement reads at least one stage")
}

/// Apply the measured range to a resolved chain: the entry boundary becomes the
/// injected tap and the collection boundary is the last requested reading
/// unless the chain declares one that reaches it. Both `measure run` and
/// `measure verify` go through this, so a re-verification resolves exactly the
/// range the run executed.
pub fn resolve_range(
    params: &mut RuntimeParams,
    entry: MeasureEntry,
    read: &[String],
) -> Result<(Tap, Tap)> {
    let entry_tap = entry.tap();
    if let Some(declared) = params.taps.inject.as_deref()
        && declared != entry_tap.name()
    {
        bail!(
            "--enter {} injects at {}, but the chain declares taps.inject={declared}; \
             declare one boundary, or drop the conflicting --set",
            entry.name(),
            entry_tap.name()
        );
    }
    let last_read = last_read_stage(read)?;
    if last_read.position() < entry_tap.position() {
        bail!(
            "--enter {} injects at {}, which is after --read {}; \
             a stage before the entry cannot be measured",
            entry.name(),
            entry_tap.name(),
            last_read.id()
        );
    }
    let collect = match params.taps.collect.as_deref() {
        Some(declared) => {
            let collect = Tap::parse(declared)
                .map_err(anyhow::Error::msg)
                .context("params taps.collect")?;
            if collect.position() < last_read.position() {
                bail!(
                    "the chain collects at {} (taps.collect), which is before --read {}; \
                     the reading would not exist",
                    collect.name(),
                    last_read.id()
                );
            }
            collect
        }
        None => last_read.tap(),
    };
    params.taps.inject = Some(entry_tap.name().to_owned());
    params.taps.collect = Some(collect.name().to_owned());
    Ok((entry_tap, collect))
}

/// One active control that prevents a point-wise reading, with the exact
/// `--set` that removes it.
pub struct Offender {
    pub path: &'static str,
    pub value: String,
    pub disable: &'static str,
}

/// Collect the active operations that would make the declared readings describe
/// a different signal than the declared stimulus.
///
/// Each control is listed only when the measured range can contain it: the
/// operation executes when the entry is at or before the boundary it consumes
/// (`entry.position() < affects.position()`), and it can change a reading only
/// at or after the first stage it changes (`affects.position() <= last_read`).
/// The stages are measured against the core stages, not inferred from the flag
/// names: on a 12x4 chart, halation moves `log_e_film` by 1.9e-02 and nothing
/// below it, the DIR couplers move `cmy_film` by 8.5e-01 and leave `log_e_film`
/// exact (`0.000e+00`), grain moves `cmy_film` by 2.9e+00, and
/// `print_render.glare`, `scanner.lens_blur`, and `scanner.unsharp_mask` move
/// `rgb_out` only.
///
/// Which of those stages execute at all follows the executed path, not only the
/// route: the whole chain runs the route's own stage list, while a narrower
/// range walks the tap topology and ignores the route.
pub fn point_wise_offenders(
    params: &RuntimeParams,
    entry: MeasureEntry,
    last_read: MeasureStage,
) -> Vec<Offender> {
    // Which stages the executed path fires, mirrored from the pipeline rather
    // than guessed from the flag names. The resolved range picks the path:
    //
    // - the whole chain runs `pipeline::execution::process_full`, whose route
    //   branches decide the stage list: `input` returns after the crop and
    //   rescale, the convert routes build the film density by inversion
    //   (`process_convert`) and never meter or develop, every other route
    //   meters, films, and develops. The print stage needs a print route, and
    //   `io.scan_film` scans the developed film directly instead, which also
    //   removes the viewing glare (`stages::scanning`). The scanner runs on
    //   every route that reaches a scan, which is every route but `input`.
    // - any narrower range walks the tap topology (`Pipeline::run_from`) from
    //   the entry to the collection boundary, which ignores the route: the
    //   nodes in that range fire whatever the route says. Two switches survive
    //   it, so they stay: the convert routes still skip the metering
    //   (`exposure::expose_full_input` returns early) and `io.scan_film` still
    //   leaves the print taps out of the topology.
    //
    // Assuming the route for both paths would admit a whole-frame metering or a
    // spatial film stage on a narrower range, which is exactly what the
    // requirement forbids.
    let route = params.workflow.route_enum();
    let convert = route.is_convert_film();
    let inject = params
        .taps
        .inject
        .as_deref()
        .and_then(|tap| Tap::parse(tap).ok());
    let collect = params
        .taps
        .collect
        .as_deref()
        .and_then(|tap| Tap::parse(tap).ok());
    let whole_chain = inject == Some(Tap::RgbIn) && collect == Some(Tap::RgbOut);
    let (metering, film, print, scan) = if whole_chain {
        let input_only = matches!(route, WorkflowRoute::Input);
        (
            !input_only && !convert,
            !input_only && !convert,
            route.prints() && !params.io.scan_film,
            !input_only,
        )
    } else {
        (!convert, true, !params.io.scan_film, true)
    };
    let entry = entry.position();
    let last = last_read.position();
    let mut out = Vec::new();
    let mut add =
        |path: &'static str, value: String, disable: &'static str, affects: MeasureStage| {
            if entry < affects.position() && affects.position() <= last {
                out.push(Offender {
                    path,
                    value,
                    disable,
                });
            }
        };
    // Input geometry runs before the boundary the entry injects at, so it can
    // never reach an `rgb_in` reading; it changes every later stage's frame.
    if params.io.upscale_factor != 1.0 {
        add(
            "io.upscale_factor",
            format!("{}", params.io.upscale_factor),
            "1",
            MeasureStage::LogEFilm,
        );
    }
    if params.io.crop {
        add(
            "io.crop",
            "true".to_owned(),
            "false",
            MeasureStage::LogEFilm,
        );
    }
    if metering && params.camera.auto_exposure {
        add(
            "camera.auto_exposure",
            "true".to_owned(),
            "false",
            MeasureStage::LogEFilm,
        );
    }
    if film {
        if params.camera.diffusion_filter.active {
            add(
                "camera.diffusion_filter.active",
                "true".to_owned(),
                "false",
                MeasureStage::LogEFilm,
            );
        }
        if params.camera.lens_blur_um > 0.0 {
            add(
                "camera.lens_blur_um",
                format!("{}", params.camera.lens_blur_um),
                "0",
                MeasureStage::LogEFilm,
            );
        }
        if params.film_render.halation.active {
            add(
                "film_render.halation.active",
                "true".to_owned(),
                "false",
                MeasureStage::LogEFilm,
            );
        }
        if params.film_render.dir_couplers.active {
            add(
                "film_render.dir_couplers.active",
                "true".to_owned(),
                "false",
                MeasureStage::CmyFilm,
            );
        }
        if params.film_render.grain.active {
            add(
                "film_render.grain.active",
                "true".to_owned(),
                "false",
                MeasureStage::CmyFilm,
            );
        }
    }
    if print {
        // `enlarger.lens_blur` is not listed: no stage reads it
        // (`docs/parity/parity_matrix.md`), so it cannot change a reading.
        if params.enlarger.diffusion_filter.active {
            add(
                "enlarger.diffusion_filter.active",
                "true".to_owned(),
                "false",
                MeasureStage::LogEPrint,
            );
        }
    }
    // Viewing glare needs the print path, exactly as the scanner gates it.
    if print && params.print_render.glare.active && params.print_render.glare.percent > 0.0 {
        add(
            "print_render.glare.active",
            "true".to_owned(),
            "false",
            MeasureStage::RgbOut,
        );
    }
    if scan && params.scanner.scan_output == "negative_to_positive" {
        add(
            "scanner.scan_output",
            "true".to_owned(),
            "false",
            MeasureStage::RgbOut,
        );
    }
    if scan && params.scanner.lens_blur > 0.0 {
        add(
            "scanner.lens_blur",
            format!("{}", params.scanner.lens_blur),
            "0",
            MeasureStage::RgbOut,
        );
    }
    if scan && params.scanner.unsharp_mask.iter().any(|value| *value > 0.0) {
        add(
            "scanner.unsharp_mask",
            format!(
                "[{},{}]",
                params.scanner.unsharp_mask[0], params.scanner.unsharp_mask[1]
            ),
            "[0,0]",
            MeasureStage::RgbOut,
        );
    }
    out
}

/// The chain identity every measurement carries. Field names follow the
/// identities the benchmark retains.
pub fn chain_identity(
    chain: &ResolvedChain,
    data_dir: &Path,
    entry: &str,
    read: &[String],
    seed: u32,
    stimulus: Value,
) -> Result<Value> {
    let params_bytes = serde_json::to_vec(&chain.params).map_err(anyhow::Error::msg)?;
    Ok(json!({
        "renderer": renderer_record()?,
        "data_dir": data_inventory_record(data_dir)?,
        "film_profile": profile_record(data_dir, &chain.film_name)?,
        "print_profile": profile_record(data_dir, &chain.print_name)?,
        "parameters": {
            "sha256": format!("{:x}", Sha256::digest(&params_bytes)),
            "sources": chain.sources.clone(),
            "presets": chain.presets.clone(),
            "files": chain.files.clone(),
            "digest_mode": chain.digest_mode,
            "edits": chain.edits.clone(),
            "seed": seed,
            "effective": chain.params,
        },
        "route": chain.params.workflow.route,
        "scan_output": chain.params.scanner.scan_output,
        "input_declaration": chain.declaration.clone(),
        "entry": entry,
        "read": read,
        "stimulus": stimulus,
    }))
}

/// A profile's name and content hash, from the data directory.
pub fn profile_record(data_dir: &Path, name: &str) -> Result<Value> {
    let path = data_dir.join("profiles").join(format!("{name}.json"));
    if path.is_file() {
        Ok(json!({
            "name": name,
            "path": path.display().to_string(),
            "sha256": sha256_file(&path)?,
        }))
    } else {
        Ok(json!({"name": name}))
    }
}

/// The selected data directory and a deterministic inventory digest over it.
pub fn data_inventory_record(data_dir: &Path) -> Result<Value> {
    let mut files: Vec<(String, String)> = Vec::new();
    collect_files(data_dir, data_dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for (relative, digest) in &files {
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    Ok(json!({
        "path": data_dir.display().to_string(),
        "file_count": files.len(),
        "manifest_sha256": format!("{:x}", hasher.finalize()),
    }))
}

fn collect_files(root: &Path, directory: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)
        .with_context(|| format!("reading {}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", directory.display()))?;
    entries.sort();
    for path in entries {
        let metadata =
            std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
        if metadata.is_dir() {
            collect_files(root, &path, out)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, sha256_file(&path)?));
        }
    }
    Ok(())
}

/// Canonicalize the declared entries that name files. A preset name and a
/// `path=value` assignment are not files and stay as declared; a relative file
/// path is resolved here, so re-resolving a recorded declaration reads the same
/// file no matter which working directory it runs from.
fn canonical_declared(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match Path::new(entry).is_file() {
            true => std::fs::canonicalize(entry)
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| entry.clone()),
            false => entry.clone(),
        })
        .collect()
}

/// The content hash of every parameter file a declaration names, in
/// declaration order. A comment-only change to a parameter file is a change the
/// identity reports.
fn declared_file_records(presets: &[String], sources: &[String]) -> Result<Vec<Value>> {
    let mut out: Vec<Value> = Vec::new();
    for entry in presets.iter().chain(sources) {
        let path = Path::new(entry);
        if !path.is_file() {
            continue;
        }
        let canonical =
            std::fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))?;
        let text = canonical.display().to_string();
        if out
            .iter()
            .any(|record| record.get("path").and_then(Value::as_str) == Some(text.as_str()))
        {
            continue;
        }
        out.push(json!({"path": text, "sha256": sha256_file(&canonical)?}));
    }
    Ok(out)
}

/// The running lab executable: path, content hash, version, precision. The
/// measurement executes the pipeline in this process, so this executable is the
/// renderer that produced the readings.
pub fn renderer_record() -> Result<Value> {
    let path = std::env::current_exe().context("resolving the lab executable")?;
    Ok(json!({
        "path": path.display().to_string(),
        "sha256": sha256_file(&path)?,
        "version": env!("CARGO_PKG_VERSION"),
        "precision": "f64",
    }))
}
