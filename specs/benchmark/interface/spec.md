# Benchmark machine interface

This document defines the target syntax and serialized behavior of `spektrafilm-bench`. It does not change the renderer's [CLI parameter language](../../cli/spec.md). The [product specification](../spec.md) owns evaluation meaning. The [measurement protocol](../metrics/spec.md) owns metrics.

## Commands

The executable must accept the following command grammar:

```text
spektrafilm-bench --help
spektrafilm-bench --version
spektrafilm-bench schema --kind suite|target|policy|run|comparison|configuration|reference|regions|preparation|review
spektrafilm-bench validate --kind suite|target|policy|run|comparison|configuration|reference|regions|preparation|review --file FILE
spektrafilm-bench run --suite FILE --target FILE --out DIRECTORY [--case ID ...] [--dry-run] [--stage-evidence]
spektrafilm-bench run --input FILE --configuration FILE --target FILE --out DIRECTORY [--input-kind raster|raw] [--input-color-space NAME --input-cctf-decoding true|false] [--seed UINT32 ...] [--regions FILE] [--reference FILE --reference-info FILE] [--preparation FILE] [--dry-run] [--stage-evidence]
spektrafilm-bench compare --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [--policy FILE] [--gate]
spektrafilm-bench compare --run DIRECTORY --against input|reference|both --out DIRECTORY
spektrafilm-bench generate-report --comparison DIRECTORY --out DIRECTORY
spektrafilm-bench generate-report --manifest FILE --out DIRECTORY
spektrafilm-bench generate-regions --manifest FILE --set ID --out FILE
spektrafilm-bench inspect --comparison DIRECTORY [--case ID ...] [--region ID ...] [--metric ID ...]
spektrafilm-bench target --renderer FILE --data-dir DIRECTORY --backend cpu|gpu --out FILE
```

Every option of the benchmark executable and of the fixture entry point must have a nonempty help description. Help, version, and schema discovery must work without a renderer, image decoder, input assets, or graphics device. Version must identify the benchmark implementation and supported document versions. Benchmark help must identify the fixture entry point. Fixture help must name the benchmark command that consumes the generated bundle. The executable and fixture entry point must satisfy [Rust delivery](../rust-delivery/spec.md).

`schema` must emit the authoritative JSON Schema for one document kind. It must describe the required nested records and explain fields whose units, coordinate space, or applicability are not evident from their type. `validate` must validate a document and resolve the local dependencies needed for that kind. It must not render images. Its existing `unchecked` list must identify checks that require another document or runtime observations. Standalone policy validation must not imply that selectors, measurement selection, or protocol binding have been checked against a suite. A valid target document must not imply that a GPU is available.

`run` must resolve the suite and target before rendering. Repeatable `--case` must select exact case identities in suite order. Duplicate selections and unknown identities must fail. Omitting `--case` must select all cases.

`run --dry-run` must use the same renderer configuration resolution, seed, environment isolation, selected profile containment, and case-owned geometry checks as execution. It must process all selected cases and seeds without decoding or rendering images. Identical seed failures must be grouped with their affected seed values and retained renderer logs. Unknown or duplicate case selections must identify the rejected identities and available choices. Renderer configuration errors must retain the actionable child error text and evidence paths.

The dry-run destination must retain `preflight.json`, suite and target snapshots, resolved configurations, logs, and hashed evidence. A successful preflight must identify `mode: dry_run` and `execution_status: validated`; a failed preflight must identify `execution_status: failed` and structured errors. It must never publish `run.json` or completed render masters. Its `unchecked` list must identify image decoding, compute-device availability, observed output geometry, and numerical results. Preflight does not qualify a run or replace execution-time checks.

`--stage-evidence` must request the renderer's native stage bundle for each simulation. It must accept suite and single-input execution. Capture must require the CPU route with `settings.preview_mode` disabled; unsupported requests must fail with an actionable error, not silently change rendering settings. Preflight must resolve both simulation and source-only configurations, retain the capture request, and identify applied exposure and stage values as unobserved. The request must not enable capture for the separate source-only decode.

`run` must require exactly one of `--suite` and `--input`. Single-input options must conflict with `--suite`; `--case` must apply only to suite execution. `--configuration` must contain the existing case configuration object, with its file paths resolved relative to that file. `--input-kind` must default to `raster`; RAW must be selected explicitly and must use the renderer adapter. Raster input must require both source declaration flags. Repeated `--seed` must select distinct unsigned 32-bit seeds and default to `[0]`. `--regions` must contain the existing region array; omission must select no fixed regions. `--reference` and `--reference-info` must require each other. Reference info must contain the existing reference object except `path` and `sha256`, which the CLI must derive from the selected local file. Nested dependencies must resolve relative to the reference-info file.

`configuration`, `regions`, and `preparation` schema kinds must expose their existing case subdocuments. The `reference` kind must expose reference info without `path` and `sha256`. Standalone validation must check their shape and available local dependencies and identify remaining cross-document applicability checks. It must not establish scene correspondence, successful rendering, or reference suitability without the required observations.

The single-input form must construct and validate one `scene` case with identity `image`, suite identity `local-image`, full-image families `numeric`, `color`, `tone`, and `flip`, and the versioned standard appearance protocol (60 pixels per degree, scales `[1,0.4,0.16]`, flat-mean grain residual, high-pass radius 4, border 1, Hann spectrum, six hotspots of minimum area 151). These settings must be retained in the generated suite, not hidden evaluator state. Case configuration must remain authoritative for crop, resize, and simulation parameters; target overrides must keep their existing applicability. The generated suite must be retained within the run without writing into the source repository.

`--preparation` must contain `original` with `path` and `sha256`, and a nonempty `operations` array of descriptive strings. The original path must resolve relative to that file and enter retained dependencies. The benchmark must record but must not execute these operations. Suite cases may contain the same optional `preparation` object. The actual prepared input hash must remain authoritative.

`compare` must load completed run records and masters. It must not invoke a renderer. It must match cases by case identity and seeds by their explicit value, not directory order. It must reject incompatible suite or protocol identities. It must retain declared target differences. Reference objectives must use the reference associated with the case.

Single-run `compare` must require `--run` and `--against` together and conflict with `--baseline`, `--candidate`, `--policy`, and `--gate`. It must load retained input and simulation masters without invoking the renderer. `input` must select input/simulation, `reference` must select reference/simulation, and `both` must retain both pairs with reference selected by default. Reference selection must fail if a selected case has no reference. Measurements must obey reference scope and processing limits; visual-only pairs must retain an inspectable unavailable measurement reason instead of a numeric score. Signed processing changes must mean simulation minus input. Reference distance must identify simulation distance to reference and must not fabricate a baseline improvement. Single-run comparisons must remain `not_evaluated`.

`--gate` must require `--policy`. It must make a non-pass evaluation produce a failing command status after the comparison has been published. Without `--gate`, a successfully published comparison must not fail solely because its evaluation is non-pass.

`generate-report` must render exactly one offline inspection page from exactly one source: retained comparison evidence (`--comparison`) or a prepared image manifest (`--manifest`). The two source flags must conflict and exactly one must be provided. The command must not invoke a renderer or recompute measurements; presentation choices beyond the source document belong to the generated page. The command result must identify the page source as `comparison` or `manifest`.

The comparison source must derive its views from the comparison and cited artifacts. It must not recompute metrics. Its output must remain usable without the run directories by copying required review artifacts and retaining their provenance. An external original that cannot be redistributed must remain excluded from a distributable report; the report must identify the omission rather than substitute another image.

The manifest source must read a review manifest without invoking a renderer or computing comparison metrics. The authored default pair must select the initially displayed pair, and invalid defaults must fail before publication; the page must allow selecting any declared image on either side and swapping the sides. `validate --kind review` must check authored structure, identities, grid arithmetic, region declarations, measurement intent, and local file dependencies; its `unchecked` list must name image decoding, observed geometry, and display color conversion.

`generate-regions` must derive a region array from one grid set of a review manifest without invoking a renderer or computing measurements; it may read image dimensions to check bounds. `--set` must identify a set that declares `grid`; unknown identities must be rejected with the available choices, and a set without derived geometry must fail. The set's declared measurement intent, with its defaults, must determine the sample window centered inside each patch window, the region role, and the measurement families. The command must reuse the review page's window arithmetic and reject windows or samples that leave the decoded image bounds. The written file must validate as the existing `regions` kind and must be usable as a case `regions` value, a reference `regions` value, and `run --regions` input; generated identities must follow the review patch names. The command must refuse an existing destination.

`inspect` must validate and read a retained comparison without invoking a renderer or computing measurements. It must emit the retained comparison identity, execution and evaluation states, conditions, summary, failures, findings, and compact measurements. The compact measurements must be one top-level `measurements` array in version and single-run comparisons. Rows in both modes must carry the same fields, except that a single-run row must add `pair_id` to identify its pair. Per-seed/pair observations and cross-seed `seed_measurements` must remain separate. Measurement identity, domain, units, estimator, sample count, availability, value or reason, seed population, and artifact references must remain intact. Exact repeatable case, region, and metric filters must reject unknown selections with available choices; they must not silently return an empty result. Referenced evidence must resolve relative to the emitted `artifact_base`. A valid read of a failed comparison must complete successfully while exposing its failed comparison state. Inspection must not modify evidence, rank unlike units, or infer acceptance.

Inspection must expose each selected case and seed's retained `run_evidence` in `case_conditions`, with a `roles` object keyed by run role. These records must include source and target identities, declared preparation, effective conditions, and observed exposure and print normalization when collected. They must retain native-stage artifact links and availability without reading human logs or recomputing image statistics. Region-pair diagnostics must preserve their own identity and both endpoint identities.

Inspection must also project retained `region_readouts` and `stage_readouts` arrays with case and seed identities. Case and region selectors must apply to these arrays; metric selectors must apply only to scalar measurements and findings. `readout_status` must distinguish retained arrays from `not_retained` fields in historical comparisons. Inspection must not backfill historical values.

`target` must construct the existing target document with canonical renderer/data paths, the actual renderer SHA-256, an ID derived from the destination filename, the explicit backend, and empty changes. It must reuse target validation and refuse any existing destination, including a symlink. It must not invoke the renderer, probe a GPU, select simulation settings, or generate test inputs. Fixture targets must share this construction rule while retaining their own IDs, edits, and environment.

Report location links must use `#case=ENCODED_ID&seed=UINT32[&pair=ENCODED_ID[&region=ENCODED_ID]]`, with percent-encoded exact identities. Version pairs use `baseline/candidate`; single-run pairs use their retained IDs. A location must select the matching case/seed/pair, open the requested region, and scroll to it after layout. Navigation must update the fragment and support reload and browser history offline. Detail disclosures must expose keyboard-operable, accessible controls. Malformed or unknown locations must show an accessible error without displaying an unrelated selected result.

## Machine output and command status

Help and version must emit their documented text. `schema` must emit one JSON Schema object without a command-result wrapper. Every other successfully parsed command must emit one UTF-8 JSON result object on stdout. Human messages, child-process output, progress, and errors must use stderr. The runner must capture renderer output separately. JSON numbers must be finite. Error output must identify a stable error code, the document or case when known, and the failed boundary. When a different invocation can succeed, the human-readable error must name the recovery action. An existing destination must be reported with the instruction to remove it or choose another destination. An incomplete destination must be reported as unusable as completed evidence.

`run` and `compare` must report progress on stderr as line-oriented text. Every selected case and seed must produce a start line and a completion line. A completion line must identify the item and its elapsed seconds. The last progress output before the command result must state the completed and failed item counts and the total elapsed seconds. Progress must not require a terminal.

A command result must identify `schema_version`, `command`, `execution_status`, and the result artifact when one is produced. An execution failure must identify `execution_status` as `failed` and contain structured errors. A published failed manifest must remain linked from the result; its detailed errors must not replace the command's structured error summary. Human error text must not be the only machine discriminator. A requested gate failure after successful computation must retain `execution_status` as `completed` and expose its non-pass `evaluation_status`.

The executable must return status `2` for argument syntax errors. It must return status `1` for validation, rendering, measurement, publication, or requested gate failure. It must return status `0` when the requested operation completed without those failures. The distinction between execution and evaluation must remain visible in JSON even when both use status `1`.

A run with a failed selected case must have `execution_status` equal to `failed`. A comparison with an unavailable measurement caused by an unsupported reference scope may complete execution but must not pass a policy that requires that measurement. A numerical computation failure must remain an execution failure.

## Carrier and validation

All documents must use UTF-8 JSON with `schema_version` equal to `1`. Authored suite, target, and policy documents must reject duplicate keys and unknown fields. They must not support includes, environment interpolation, expressions, shell commands, or executable plugins.

Schemas must describe document structure, types, ranges, and field applicability. The validator must also check duplicate identities, nonfinite numbers, unsafe artifact paths, and identifiers resolvable from the supplied document. Checks that require file contents must use the declared dependencies. Invalid authored probe point counts, coordinate ordering, mutually exclusive fields, and auxiliary reference document structure must fail validation before rendering. Runtime parameter and source-declaration semantics must use the renderer's shared definitions rather than a second benchmark parameter registry.

Generated run and comparison readers may ignore unknown fields within a supported schema version. Required semantic fields must still be validated. Unsupported versions must fail rather than select defaults. Generated documents must preserve full unsigned seed and counter values.

Authored file paths must resolve relative to the document that contains them. Executable paths must be explicit paths, not commands resolved from `PATH`. Run and comparison artifact paths must be relative to their containing directory. Artifact paths must not contain an absolute root, parent traversal, or a symbolic link that escapes that directory. File dependencies outside the artifact directory may be consumed through authored paths but must be hashed and identified.

The runner must retain bytes from the filesystem-resolved dependency path. It must preserve the absolute authored path separately for offline lookup, including parent components after symbolic links. Comparison must match that authored identity before looking up a lexically normalized identity from a historical run. Current-directory components must not distinguish equivalent authored paths. Neither lookup may resolve the original filesystem.

## Suite document

A suite must contain `schema_version`, `id`, `protocol`, and a nonempty `cases` array. `id` must be a stable nonempty string. `protocol` must select `appearance-v1` and contain its declared `pixels_per_degree`, region sampling scales, grain residual and spectrum settings, and hotspot selection settings. Every protocol value that changes a measurement must contribute to the resolved protocol digest.

A case must contain `id`, `kind`, `input`, `configuration`, `seeds`, `regions`, and `measurements`. `kind` must be `probe` or `scene`. Case identities must be unique. `seeds` must be a nonempty array of distinct integers in the renderer's seed domain. Each selected case must be rendered for each selected seed.

`input` must contain `path` and `sha256`. Raster input must also contain `color_space` and `cctf_decoding`. RAW input must use the adapter declaration and may contain the dedicated RAW options specified below. The input hash must match before decoding. File extension must not infer a color declaration.

`configuration` must use the following fields: `film`, `paper`, `presets`, `params`, `set_files`, `set`, `route`, `scan_output`, and `raw`. Fields not required by the renderer may be omitted. `presets` must contain explicit preset-file paths, not mutable user-library names. `params` must be a legacy parameter snapshot path. `set_files` must contain sparse parameter-file paths. Every `set` entry in case configuration, target changes, or case changes must contain renderer-compatible `PATH=VALUE` assignments, including comma-separated assignments with quoted strings and arrays; file carriers must occur only in `set_files`. File edits must apply in listed order before listed inline edits. Their semantic validation and precedence must follow the renderer's resolver. No benchmark field may replace a runtime field alias.

`raw` may contain `white_balance`, `temperature`, `tint`, and `lens_correction` with the dedicated renderer option semantics. These fields must apply only to RAW cases. An incompatible combination must fail before rendering.

A region must contain `id`, `label`, `role`, `geometry`, and `measurements`. `role` must identify `skin`, `neutral`, `shadow`, `highlight`, `color`, `flat_field`, `edge`, `texture`, or `other`. `geometry` must contain either a half-open rectangle `[x, y, width, height]` in render coordinates or a lossless mask path with its hash and declared dimensions. Region identities must be unique within a case. Bounds must be validated against the actual master. Fixed regions must not be silently resized to fit a changed render.

Tone-response probe geometries may be declared in `protocol.probe.tone_response` or overridden by a case's `probe.tone_response`. Both locations must use the same rectangle or hashed-mask validation. Mask paths must resolve relative to the suite document, enter the run's dependency inventory, and be retained as immutable evidence. Comparison must resolve the effective probe geometry only from retained dependencies, without requiring original mask files or the invocation working directory.

The case's `measurements` must select full-image measurement families from `numeric`, `color`, `tone`, `flip`, `grain`, `halation`, and `edge`. Region `measurements` must select applicable families in that region. The protocol must resolve each family to explicit scalar identities, maps, curves, or spectra. A family selector must not request a hidden aggregate quality score.

A case may contain `region_pairs`, an array of objects with `id`, `label`, `left`, and `right`. Pair identities must be unique and must not collide with region identities. Both endpoints must name distinct existing regions in that case. A pair must not infer correspondence across cases or image sets.

The protocol may contain `color` with a finite positive `hue_chroma_floor`. This declaration selects regional hue diagnostics and must contribute to protocol identity. Omitting it must not select an implicit hue threshold. The measurement protocol owns the diagnostic definitions and availability rules.

A case may contain `reference`, `split`, and `split_group`. `split` must be `calibration` or `evaluation`. A case with fitted reference objectives must provide `split_group`. Related captures must use the same group.

A reference must contain `id`, `path`, `sha256`, `scope`, `color`, `provenance`, and `processing`. `scope` must be `material`, `aligned_scene`, `regional`, or `visual`. `color` must select an explicit declaration or an embedded ICC profile with its hash. Unknown color interpretation must make color measurements unavailable.

`provenance` must identify source, attribution, redistribution permission, capture conditions, and development and digitization conditions. Unknown conditions must be represented explicitly, not fabricated. `processing` must identify known grading, histogram matching, compression, sharpening, noise reduction, and resampling, including unknown entries.

An `aligned_scene` reference must provide a frozen transform or already aligned derivative, its correspondence-review record, and a common-area mask. A `regional` reference must provide corresponding reference-region geometries. A `material` reference must provide the measurement domain, instrument, units, and conditions required by its selected metrics. A `visual` reference must not select full-reference scalar measurements.

A `visual` reference may provide explicit reference-region geometries for crop inspection. The comparison must retain these geometries without calculating reference distances. A report must not borrow input-to-simulation alignment to locate a region in an unaligned reference.

Reference transforms, masks, review records, and derivatives must be hashed dependencies. Their geometry and purpose must be explicit. The evaluator must not fit a new transform during `compare`.

## Review manifest

A review manifest must contain `schema_version: 1`, `title`, `sets`, `images`, and `default_pair`. The manifest and each image may contain a `description` string, omitted as empty. Unknown fields, duplicate JSON keys, and duplicate identities must fail. Authored image paths must resolve relative to this manifest. Set and image array order must be preserved.

A set must contain `id`, `label`, and exactly one of `grid` or `regions`. A `grid` must contain positive integer `rows` and `columns`, nonnegative integer `origin: [x, y]`, positive integer `patch_size: [width, height]`, and nonnegative integer `gap: [x, y]`. All geometry uses decoded image pixels. Patch windows are half-open rectangles. Column labels use `A` through `Z`, then `AA` and subsequent spreadsheet-style names; rows start at `1`. Windows must follow row-major order. Grid arithmetic must not overflow. Generation must check every window against actual image bounds and reject dimension differences within one grid set. A grid set may also declare a `measurement` object with positive integer `sample: [width, height]`, a `role`, and a `measurements` array. The object must declare the sample window centered inside each patch window, the derived region role, and the derived measurement families, using the existing region role and measurement family names. The sample must fit inside the declared patch window. Omitted fields must select the whole window, `other`, and no measurement families. A set with explicit `regions` must not declare `measurement`.

A `regions` set must contain a nonempty array of region records with `id`, `label`, and a `geometry` object keyed by image identity. Each key must identify an image of the same set; each value must be a half-open `[x, y, width, height]` rectangle in that image's decoded pixels. A region may cover a subset of its set's images. Duplicate region identities, unresolved image keys, and overlapping rectangles on one image must fail validation; rectangle bounds must be checked against decoded image dimensions during generation. Region identities are scoped to their set: the same identity in another set must not pair.

Each image must contain `id`, `set`, `path`, `label`, `color_space`, and `cctf_decoding`. The `set` value must identify a declared set. The input must be a prepared RGB raster supported by the existing image reader. A camera RAW file must first use the existing RAW decoder and an explicitly established preparation; its name or extension must not imply a grid, a region, or a source declaration. Explicit declarations must remain authoritative over embedded metadata and undeclared adjacent sidecars. The report must retain the original path and SHA-256 and identify the display conversion separately.

`images` must contain at least two records. `default_pair` must contain `left` and `right` image identities. Same-set and cross-set defaults are permitted. A cross-set pair must remain a visual comparison with independent selection. A `regions` set must remain independently selectable on each side; its shared region identities are explicit author declarations, not an inferred aligned pair.

Successful generation must produce `index.html`, native display assets, and terminal `report.json`. The report must embed its view data for offline loading. Its command result must identify the HTML artifact, report manifest, and image and set counts. Metadata must not execute as HTML or JavaScript. The fixture generator must publish a schema-validated review manifest example that refers to its existing controlled images and exercises both set forms.

Migration: the `report` and `review` commands merge into `generate-report`, and the `regions` command becomes `generate-regions` with its measurement intent read from the manifest. The `review` kind replaces the `chart-report` v1 manifest. `charts` become `sets`, `images[].chart` becomes `images[].set`, and each set keeps its `grid` or declares explicit `regions`. A chart-report v1 manifest must not be accepted as a review manifest.

## Target document

A target must contain `schema_version`, `id`, `renderer`, `data_dir`, `backend`, and `changes`. `renderer` must contain an executable path and its SHA-256. `backend` must be `cpu` or `gpu`. `data_dir` must be explicit. Renderer identity validation must precede execution.

`changes` must contain arrays `set_files` and `set`, which may be empty. They must apply after each case's configuration using the existing sparse-file and inline-assignment semantics. They must not edit seed, input identity, or geometry outside the permitted renderer fields. The comparison must expose the resolved differences rather than trusting a change description.

A target may contain `case_changes`, keyed by exact case identity. Each value must use the same `set_files` and `set` fields and must apply after target-wide changes. Unknown case identities must fail. This surface must not support conditions or wildcard case selection.

Explicit renderer and data identities must support executable or profile/data revisions without parameter edits. The run must hash the complete selected data directory with a deterministic relative-path inventory. The resolved inventory must identify file bytes, not modification times. Before any rendering, the runner must check the actual film and paper profile names selected by the renderer's dry run, including preset-provided references, and reject profile paths that resolve outside the target's data directory. Target changes must not silently substitute a profile file from another directory. A data directory that cannot be inventoried, including one containing symlinks, must not abort the run without a manifest: the run must still publish `run.json`, must fail every case that did not already fail for a more specific reason such as profile confinement, and must remain ineligible as a qualified baseline.

The target must not own output container settings, input geometry, or seed selection. The runner must control artifact paths, request floating TIFF masters, preserve scanner output color and transfer state, and request execution diagnostics. Source declarations must remain case-owned. Before image rendering, the runner must compare target-resolved input geometry and `settings.preview_mode` / `settings.preview_max_size` against the case-only dry resolution. Target-wide and case-specific changes, whether inline assignments or sparse files, must fail if these case-owned resolved fields differ.

The runner must isolate user preset configuration and remove inherited simulation-control environment values before rendering. An allowlist of environment variables that influence loading, numerical execution, or device selection must be recorded with the benchmark implementation. Allowed values must contribute to run identity. Experimental environment controls must require an explicit target extension in a new interface version; unrecorded experimental values must not affect a qualified run.

## Policy document

A policy must contain `schema_version`, `id`, `protocol_digest`, `calibration`, and `rules`. It must bind to the resolved measurement protocol. `calibration` must identify hashed evidence, its case population, reviewed labels, observation scale, and scope. An uncalibrated exploratory policy must identify that state and must not produce `passed` qualification.

The resolved protocol digest must include case source declarations and hashes, selected measurement families, probe definitions, region geometry and mask hashes, reference dependencies, and seed populations. It must also include the benchmark implementation revision, metric and color-transform versions, relevant numerical dependency versions, and the LittleCMS version and transform settings for ICC references. `validate --kind suite` must expose this digest for policy authors. Cross-implementation evidence and policy handling must follow [Rust delivery](../rust-delivery/spec.md#existing-evidence-and-protocol-identity).

Each rule must contain `id`, `purpose`, `selector`, `metric`, and its constraints. `purpose` must be `protect`, `expect`, or `reference`. `selector` must identify exact case and region identities or the explicit full-image region. Wildcards, expressions, and implicit aggregate selectors must not be accepted.

Before computing measurements, `compare` must reject a rule whose metric identity is unknown or cannot be produced by the selected case and region measurement families. A supported selected metric that lacks usable evidence must remain unavailable under the existing evaluation rules. The implementation must reuse measurement definitions for this check rather than maintain a second policy metric registry.

A protected rule must contain an inclusive finite `min` or `max`, or both, in the metric's units. A reference rule must constrain candidate-to-reference distance or candidate-minus-baseline reference distance with an explicitly named metric. An expected-change rule must state the direction or interval expected for one signed metric. Expected changes must not mask protected failures.

Every rule must declare `required`. An unavailable required rule must prevent a pass. An optional unavailable rule must remain visible and must not count as satisfied. Evaluation must not infer thresholds from a candidate's measurements.

A human approval may be attached to a comparison as a separate review artifact with reviewer, comparison digest, decision, and rationale. It must not modify the numerical decision or claim a new measured value.

## Run record

A run directory must contain `run.json`, the suite and target snapshots, resolved dependency inventories, per-case resolved configuration, per-seed floating masters, diagnostics, and captured renderer output. The record must distinguish source documents from resolved configuration.

`run.json` must contain `schema_version`, `id`, `execution_status`, `suite`, `target`, `protocol`, `selection`, `dependencies`, `cases`, and `artifacts`. Document and dependency identities must use SHA-256. `selection` must state whether it covers the full suite.

Each case result must identify case identity, seed, terminal status, input and output dimensions, configuration artifact, master artifact, diagnostics artifact, and errors when present. Actual backend, execution path, host precision, GPU precision, and adapter facts must come from observed diagnostics. Missing observations must remain unavailable.

Each completed case must retain the full executed simulation command, including numerical-output, diagnostics, and requested stage-evidence arguments. The source-only command must remain separate. Required source evidence must remain required even when both source-master fields are absent; a valid manifest digest does not establish semantic completeness.

Completed runs must retain a floating `source_raw` with `source_raw_metadata` from the real renderer's input route, including source color interpretation and observed dimensions. The input route must use the same RAW decoding options and case-owned geometry as simulation, force output to linear ACES2065-1, disable the independent magazine finishing treatment, and force `taps.inject=rgb_in` and `taps.collect=rgb_out`. Source configuration and observed diagnostics must remain retained dependencies of an input comparison. Original source dimensions and preparation metadata must remain separately identified. Input output geometry must be checked before declaring pixelwise correspondence with simulation.

Completed case artifact references must be objects with an indexed `path`. A malformed required reference must return a structured command failure.

The artifact index must identify relative path, content hash, media type, role, and dimensions or sample layout when applicable. All retained paths must resolve. Partial work must not be indexed as a complete master. A failed run must retain successfully completed cases and explicit failures, but must not be reused as a qualified baseline.

A requested native stage bundle must be retained under the case's `stage_evidence` reference. Its manifest and every buffer must be indexed artifacts. Validation must check the producer schema version, terminal state, supported numerical layout, byte length, finite samples, content hashes, stage identities, geometry, and declared availability. A missing or failed required capture must prevent a completed run. A stage skipped by the selected workflow must remain not applicable rather than become zero-valued evidence. The renderer's native-stage contract owns buffer and preparation semantics; the benchmark must not reconstruct them.

## Comparison record

A comparison directory must contain `comparison.json`, input run identities, the policy snapshot when selected, scalar measurements, map and curve artifacts, and selected detail-region records. Every finding must resolve to retained artifacts or verified source run artifacts.

`comparison.json` must contain `schema_version`, `id`, `execution_status`, `evaluation_status`, `protocol`, `selection`, `condition_differences`, `cases`, `findings`, `summary`, and `artifacts`. Version comparisons must contain `baseline` and `candidate`, and must contain `policy` when selected. Single-run comparisons must contain `mode` equal to `single_run`, `run`, and `default_pair` equal to `input` or `reference`. Each case must retain a `pairs` array with unique `id`, `left_role`, `right_role`, `pixel_aligned`, `status`, pair-owned measurements and review artifacts, and reference applicability when selected. Input pairs must use roles `input` and `simulation`; reference pairs must use `reference` and `simulation`. Unavailable image evidence must remain labeled and must not fall back to another role. Pair ids must also identify scalar measurements and findings in single-run results.

Each completed case must retain `run_evidence` keyed by `baseline` and `candidate` for a version comparison, or `simulation` for a single-run comparison. It must preserve run, target, input, data and configuration identities, declared preparation, actual command and observed stage context. Referenced native bundles must be copied with their relative buffer dependencies into the comparison artifact index. Report and inspection consumers must not require the original run directory to recover this evidence.

New comparisons must retain `region_readouts` on version case records or single-run pair records, and `stage_readouts` once per case. The [comparison schema](../../../crates/spektrafilm-benchmark/assets/comparison.schema.json) owns their field structure. The [measurement protocol](../metrics/spec.md#retained-region-coordinates) owns estimators and eligibility. Historical version-1 records may omit these arrays. Consumers must not interpret omission as an empty measured population or a zero result. Available observations must contain finite values; unavailable observations must contain a reason and no value. Color readouts must identify `pair_id`; native-stage readouts must identify `run_role`. Artifact references must resolve inside the retained comparison.

`evaluation_status` must be `not_evaluated`, `passed`, `review_required`, `failed`, or `insufficient_evidence`. No policy must select `not_evaluated`. A protected or expected-change rule violation must select `failed`. A missing required rule measurement or an uncalibrated policy must select `insufficient_evidence` unless an observed violation already selects `failed`. An unresolved compatibility difference or required human decision must select `review_required` unless a stronger failure applies. `passed` must require all required rules and qualification conditions to be satisfied.

Each scalar measurement must identify case, seed population, region, metric identity, domain, units, estimator, sample count, availability, and artifact references. Available measurements must contain `value`. Other states must contain `reason` and no value. Curves, spectra, and maps must retain their sample coordinates, units, dimensions, and hashes.

Each finding must contain `id`, `case_id`, `region_id`, `metric`, `decision`, `evidence`, and the relevant rule identity when policy-derived. Automatic detail records must retain their rectangle, ranking metric, ranking value, and merge/selection method. A prose explanation may supplement but must not replace these fields.

The summary must retain selected and completed case counts, required and unavailable rule counts, protected failures, and worst-case findings. It must not contain a universal appearance score.

## Publication and examples

`run`, `compare`, and `generate-report` must require a destination that does not exist. They must not replace completed evidence. They must publish a terminal manifest only after referenced local artifacts have been finalized and hashed. A handled operation failure may publish a failed terminal manifest and valid partial evidence. Forced termination before the terminal manifest must leave an incomplete directory that cannot be treated as a completed run.

A gate failure must preserve the completed comparison. A report-generation failure must not modify its source comparison. Publication failures must not produce a success result. This contract does not promise a transaction across different destination directories or power-loss durability.

The implementation must ship schema-validated examples for every authored document kind. Command examples must execute with packaged fixture assets and the real renderer. Documented benchmark usage examples must be checked against the implemented command and flag surface by a permanent test. A documented command or flag the executable does not accept must fail that test. The schema, parser, examples, and validator must change together. The interface must not claim executable examples or a validated schema before these artifacts are exercised.

An executable example must use fixture generation, `validate`, `run`, `compare`, and `generate-report` to complete a comparison without source-code inspection. The example must read metric identities, units, availability, and evidence links from the resulting measurements before authoring a policy. It must identify the exploratory policy's qualification limit. This workflow must use the existing commands and document fields; it must not require a new discovery language.

## Standard configuration generation

The existing fixture executable must accept this grammar without a new selector or configuration language:

```text
spektrafilm-bench-fixtures --renderer FILE --data-dir DIRECTORY --out DIRECTORY [--natural-input FILE | --download-natural]
```

`--natural-input` and `--download-natural` must be mutually exclusive. The generator must preserve the existing pinned natural-source validation and attribution rules.

Every invocation must generate the existing probe assets, suites, targets, exploratory policy, and provenance. It must also generate `look.toml`, `color-tone.json`, and `appearance.json`. `look.toml` must use the renderer's portable preset schema. The two JSON files must contain the existing standalone `configuration` object, not a new document envelope. The [standard configuration product contract](../standard-configurations/spec.md) defines their rendering purposes and suite coverage.

Both configurations must reference `look.toml` through `presets: ["look.toml"]`. They must use existing `route`, `set_files`, and `set` carriers. Their paths must resolve relative to the containing configuration file. Generated suite cases must carry the equivalent configuration objects because the suite schema has no configuration include field.

Both configuration objects must set `route` to `input > film > print > scan` and `set_files` to an empty array. Their ordered `set` arrays must begin with these shared assignments:

```json
[
  "camera.auto_exposure=false",
  "camera.exposure_compensation_ev=0",
  "camera.film_format_mm=35",
  "scanner.auto_setup=off",
  "scanner.finish=neutral",
  "magazine_print_color.active=false",
  "io.output_color_space=sRGB",
  "io.output_cctf_encoding=true",
  "io.crop=false",
  "io.upscale_factor=1",
  "settings.preview_mode=false",
  "settings.use_enlarger_lut=false",
  "settings.use_scanner_lut=false"
]
```

`color-tone.json` must append `debug.deactivate_spatial_effects=true`, `debug.deactivate_stochastic_effects=true`, and `film_render.grain.active=false`. `appearance.json` must append `debug.deactivate_spatial_effects=false` and `debug.deactivate_stochastic_effects=false`. Neither object may add `film` or `paper` selections that compete with the exported look. The generator must use the renderer's resolution to validate the effective settings; standalone shape validation is not proof of supported renderer parameters.

The generator must obtain the look through the selected renderer's public `preset show built-in:Kodak Gold 200 --format toml --data-dir DIRECTORY` command. It must capture child output and diagnostics separately. It must not copy a preset from the checkout or resolve a bare user-library name. Preset export or validation failure must fail generation without selecting a substitute look. Profile references must permit intentional runtime profile revisions rather than require baseline content hashes.

The generator must validate both configuration documents and all generated suites and targets before publishing success. `provenance.json` must record the standard configuration revision `standard-print-scan-v1`, selected built-in preset selector, renderer SHA-256, and relative paths and SHA-256 values for the three new files:

```json
"standard_configurations": {
  "revision": "standard-print-scan-v1",
  "preset_selector": "built-in:Kodak Gold 200",
  "renderer": {"path": "ABSOLUTE_RENDERER_PATH", "sha256": "RENDERER_SHA256"},
  "files": [
    {"path": "look.toml", "sha256": "LOOK_SHA256"},
    {"path": "color-tone.json", "sha256": "COLOR_TONE_SHA256"},
    {"path": "appearance.json", "sha256": "APPEARANCE_SHA256"}
  ]
}
```

Existing natural-source provenance must remain intact. A change to authored standard settings or appearance-case coverage must update this revision; runtime dependency hashes remain authoritative for file identity.

The successful stdout result must include `schema_version`, `command: fixtures`, `execution_status: completed`, the output directory as `artifact`, the existing `suites` list, `preset: look.toml`, and `configurations` with `color_tone: color-tone.json` and `appearance: appearance.json`. Returned file paths must be relative to `artifact`. Failure must use the existing structured error and exit-status contract. A partial directory without a successful final provenance record must not count as a generated bundle.

The generator must refuse any existing destination. It must not invoke rendering or device discovery. It must not create runs, comparisons, or calibrated policy evidence. Generated baseline and candidate target files remain examples, not completed baseline or candidate runs.

The packaged usage example must generate once, use `color-tone.json` and `appearance.json` with a declared local image, and select seeds `0`, `42`, and `99` explicitly for full appearance. It must show how to construct a second target and reuse the same bundle for a version comparison. It must use the existing `run`, `compare`, `inspect`, and `generate-report` commands. These examples must be executed during delivery acceptance rather than presented as already supported.
