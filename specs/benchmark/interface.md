# Benchmark machine interface

This document defines the target syntax and serialized behavior of `spektrafilm-bench`. It does not change the renderer's [CLI parameter language](../cli-parameters/spec.md). The [product specification](spec.md) owns evaluation meaning. The [measurement protocol](metrics.md) owns metrics.

## Commands

The executable must accept the following command grammar:

```text
spektrafilm-bench --help
spektrafilm-bench --version
spektrafilm-bench schema --kind suite|target|policy|run|comparison
spektrafilm-bench validate --kind suite|target|policy|run|comparison --file FILE
spektrafilm-bench run --suite FILE --target FILE --out DIRECTORY [--case ID ...]
spektrafilm-bench compare --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [--policy FILE] [--gate]
spektrafilm-bench report --comparison DIRECTORY --out DIRECTORY
```

Every option must have help. Help, version, and schema discovery must work without a renderer, image decoder, input assets, or graphics device. Version must identify the benchmark implementation and supported document versions.

`schema` must emit the authoritative JSON Schema for one document kind. `validate` must validate a document and resolve the local dependencies needed for that kind. It must not render images. It must report which runtime conditions remain unobserved. A valid target document must not imply that a GPU is available.

`run` must resolve the suite and target before rendering. Repeatable `--case` must select exact case identities in suite order. Duplicate selections and unknown identities must fail. Omitting `--case` must select all cases.

`compare` must load completed run records and masters. It must not invoke a renderer. It must match cases by case identity and seeds by their explicit value, not directory order. It must reject incompatible suite or protocol identities. It must retain declared target differences. Reference objectives must use the reference associated with the case.

`--gate` must require `--policy`. It must make a non-pass evaluation produce a failing command status after the comparison has been published. Without `--gate`, a successfully published comparison must not fail solely because its evaluation is non-pass.

`report` must derive its views from the comparison and cited artifacts. It must not recompute metrics. Its output must remain usable without the run directories by copying required review artifacts and retaining their provenance. An external original that cannot be redistributed must remain excluded from a distributable report; the report must identify the omission rather than substitute another image.

## Machine output and command status

Every successfully parsed command must emit at most one UTF-8 JSON result object on stdout. Human messages, child-process output, progress, and errors must use stderr. The runner must capture renderer output separately. JSON numbers must be finite. Error output must identify a stable error code, the document or case when known, and the failed boundary.

A success result must identify `schema_version`, `command`, `execution_status`, and the result artifact when one is produced. A failure result must identify `execution_status` as `failed` and contain structured errors. Human error text must not be the only machine discriminator.

The executable must return status `2` for argument syntax errors. It must return status `1` for validation, rendering, measurement, publication, or requested gate failure. It must return status `0` when the requested operation completed without those failures. The distinction between execution and evaluation must remain visible in JSON even when both use status `1`.

A run with a failed selected case must have `execution_status` equal to `failed`. A comparison with an unavailable measurement caused by an unsupported reference scope may complete execution but must not pass a policy that requires that measurement. A numerical computation failure must remain an execution failure.

## Carrier and validation

All documents must use UTF-8 JSON with `schema_version` equal to `1`. Authored suite, target, and policy documents must reject duplicate keys and unknown fields. They must not support includes, environment interpolation, expressions, shell commands, or executable plugins.

The schemas must reject missing fields, invalid types, nonfinite numbers, invalid ranges, duplicate identities, invalid seed values, unsafe artifact paths, and unresolved identifiers. They must enforce field applicability. Runtime parameter and source-declaration semantics must use the renderer's shared definitions rather than a second benchmark parameter registry.

Generated run and comparison readers may ignore unknown fields within a supported schema version. Required semantic fields must still be validated. Unsupported versions must fail rather than select defaults. Generated documents must preserve full unsigned seed and counter values.

Authored file paths must resolve relative to the document that contains them. Executable paths must be explicit paths, not commands resolved from `PATH`. Run and comparison artifact paths must be relative to their containing directory. Artifact paths must not contain an absolute root, parent traversal, or a symbolic link that escapes that directory. File dependencies outside the artifact directory may be consumed through authored paths but must be hashed and identified.

## Suite document

A suite must contain `schema_version`, `id`, `protocol`, and a nonempty `cases` array. `id` must be a stable nonempty string. `protocol` must select `appearance-v1` and contain its declared `pixels_per_degree`, region sampling scales, grain residual and spectrum settings, and hotspot selection settings. Every protocol value that changes a measurement must contribute to the resolved protocol digest.

A case must contain `id`, `kind`, `input`, `configuration`, `seeds`, `regions`, and `measurements`. `kind` must be `probe` or `scene`. Case identities must be unique. `seeds` must be a nonempty array of distinct integers in the renderer's seed domain. Each selected case must be rendered for each selected seed.

`input` must contain `path` and `sha256`. Raster input must also contain `color_space` and `cctf_decoding`. RAW input must use the adapter declaration and may contain the dedicated RAW options specified below. The input hash must match before decoding. File extension must not infer a color declaration.

`configuration` must use the following fields: `film`, `paper`, `presets`, `params`, `set_files`, `set`, `route`, `scan_output`, and `raw`. Fields not required by the renderer may be omitted. `presets` must contain explicit preset-file paths, not mutable user-library names. `params` must be a legacy parameter snapshot path. `set_files` must contain sparse parameter-file paths. Every `set` entry in case configuration, target changes, or case changes must contain renderer-compatible `PATH=VALUE` assignments, including comma-separated assignments with quoted strings and arrays; file carriers must occur only in `set_files`. File edits must apply in listed order before listed inline edits. Their semantic validation and precedence must follow the renderer's resolver. No benchmark field may replace a runtime field alias.

`raw` may contain `white_balance`, `temperature`, `tint`, and `lens_correction` with the dedicated renderer option semantics. These fields must apply only to RAW cases. An incompatible combination must fail before rendering.

A region must contain `id`, `label`, `role`, `geometry`, and `measurements`. `role` must identify `skin`, `neutral`, `shadow`, `highlight`, `color`, `flat_field`, `edge`, `texture`, or `other`. `geometry` must contain either a half-open rectangle `[x, y, width, height]` in render coordinates or a lossless mask path with its hash and declared dimensions. Region identities must be unique within a case. Bounds must be validated against the actual master. Fixed regions must not be silently resized to fit a changed render.

Tone-response probe geometries may be declared in `protocol.probe.tone_response` or overridden by a case's `probe.tone_response`. Both locations must use the same rectangle or hashed-mask validation. Mask paths must resolve relative to the suite document, enter the run's dependency inventory, and be retained as immutable evidence. Comparison must resolve the effective probe geometry only from retained dependencies, without requiring original mask files or the invocation working directory.

The case's `measurements` must select full-image measurement families from `numeric`, `color`, `tone`, `flip`, `grain`, `halation`, and `edge`. Region `measurements` must select applicable families in that region. The protocol must resolve each family to explicit scalar identities, maps, curves, or spectra. A family selector must not request a hidden aggregate quality score.

A case may contain `reference`, `split`, and `split_group`. `split` must be `calibration` or `evaluation`. A case with fitted reference objectives must provide `split_group`. Related captures must use the same group.

A reference must contain `id`, `path`, `sha256`, `scope`, `color`, `provenance`, and `processing`. `scope` must be `material`, `aligned_scene`, `regional`, or `visual`. `color` must select an explicit declaration or an embedded ICC profile with its hash. Unknown color interpretation must make color measurements unavailable.

`provenance` must identify source, attribution, redistribution permission, capture conditions, and development and digitization conditions. Unknown conditions must be represented explicitly, not fabricated. `processing` must identify known grading, histogram matching, compression, sharpening, noise reduction, and resampling, including unknown entries.

An `aligned_scene` reference must provide a frozen transform or already aligned derivative, its correspondence-review record, and a common-area mask. A `regional` reference must provide corresponding reference-region geometries. A `material` reference must provide the measurement domain, instrument, units, and conditions required by its selected metrics. A `visual` reference must not select full-reference scalar measurements.

Reference transforms, masks, review records, and derivatives must be hashed dependencies. Their geometry and purpose must be explicit. The evaluator must not fit a new transform during `compare`.

## Target document

A target must contain `schema_version`, `id`, `renderer`, `data_dir`, `backend`, and `changes`. `renderer` must contain an executable path and its SHA-256. `backend` must be `cpu` or `gpu`. `data_dir` must be explicit. Renderer identity validation must precede execution.

`changes` must contain arrays `set_files` and `set`, which may be empty. They must apply after each case's configuration using the existing sparse-file and inline-assignment semantics. They must not edit seed, input identity, or geometry outside the permitted renderer fields. The comparison must expose the resolved differences rather than trusting a change description.

A target may contain `case_changes`, keyed by exact case identity. Each value must use the same `set_files` and `set` fields and must apply after target-wide changes. Unknown case identities must fail. This surface must not support conditions or wildcard case selection.

Explicit renderer and data identities must support executable or profile/data revisions without parameter edits. The run must hash the complete selected data directory with a deterministic relative-path inventory. The resolved inventory must identify file bytes, not modification times. Before any rendering, the runner must check the actual film and paper profile names selected by the renderer's dry run, including preset-provided references, and reject profile paths that resolve outside the target's data directory. Target changes must not silently substitute a profile file from another directory.

The target must not own output container settings, input geometry, or seed selection. The runner must control artifact paths, request floating TIFF masters, preserve scanner output color and transfer state, and request execution diagnostics. Source declarations must remain case-owned. Before image rendering, the runner must compare target-resolved input geometry and `settings.preview_mode` / `settings.preview_max_size` against the case-only dry resolution. Target-wide and case-specific changes, whether inline assignments or sparse files, must fail if these case-owned resolved fields differ.

The runner must isolate user preset configuration and remove inherited simulation-control environment values before rendering. An allowlist of environment variables that influence loading, numerical execution, or device selection must be recorded with the benchmark implementation. Allowed values must contribute to run identity. Experimental environment controls must require an explicit target extension in a new interface version; unrecorded experimental values must not affect a qualified run.

## Policy document

A policy must contain `schema_version`, `id`, `protocol_digest`, `calibration`, and `rules`. It must bind to the resolved measurement protocol. `calibration` must identify hashed evidence, its case population, reviewed labels, observation scale, and scope. An uncalibrated exploratory policy must identify that state and must not produce `passed` qualification.

The resolved protocol digest must include case source declarations and hashes, selected measurement families, probe definitions, region geometry and mask hashes, reference dependencies, and seed populations. It must also include metric and color-transform versions, installed numerical dependency versions, and the LittleCMS version and transform settings for ICC references. `validate --kind suite` must expose this digest for policy authors.

Each rule must contain `id`, `purpose`, `selector`, `metric`, and its constraints. `purpose` must be `protect`, `expect`, or `reference`. `selector` must identify exact case and region identities or the explicit full-image region. Wildcards, expressions, and implicit aggregate selectors must not be accepted.

A protected rule must contain an inclusive finite `min` or `max`, or both, in the metric's units. A reference rule must constrain candidate-to-reference distance or candidate-minus-baseline reference distance with an explicitly named metric. An expected-change rule must state the direction or interval expected for one signed metric. Expected changes must not mask protected failures.

Every rule must declare `required`. An unavailable required rule must prevent a pass. An optional unavailable rule must remain visible and must not count as satisfied. Evaluation must not infer thresholds from a candidate's measurements.

A human approval may be attached to a comparison as a separate review artifact with reviewer, comparison digest, decision, and rationale. It must not modify the numerical decision or claim a new measured value.

## Run record

A run directory must contain `run.json`, the suite and target snapshots, resolved dependency inventories, per-case resolved configuration, per-seed floating masters, diagnostics, and captured renderer output. The record must distinguish source documents from resolved configuration.

`run.json` must contain `schema_version`, `id`, `execution_status`, `suite`, `target`, `protocol`, `selection`, `dependencies`, `cases`, and `artifacts`. Document and dependency identities must use SHA-256. `selection` must state whether it covers the full suite.

Each case result must identify case identity, seed, terminal status, input and output dimensions, configuration artifact, master artifact, diagnostics artifact, and errors when present. Actual backend, execution path, host precision, GPU precision, and adapter facts must come from observed diagnostics. Missing observations must remain unavailable.

The artifact index must identify relative path, content hash, media type, role, and dimensions or sample layout when applicable. All retained paths must resolve. Partial work must not be indexed as a complete master. A failed run must retain successfully completed cases and explicit failures, but must not be reused as a qualified baseline.

## Comparison record

A comparison directory must contain `comparison.json`, input run identities, the policy snapshot when selected, scalar measurements, map and curve artifacts, and selected detail-region records. Every finding must resolve to retained artifacts or verified source run artifacts.

`comparison.json` must contain `schema_version`, `id`, `execution_status`, `evaluation_status`, `baseline`, `candidate`, `protocol`, `selection`, `condition_differences`, `cases`, `findings`, `summary`, and `artifacts`. It must contain `policy` when a policy is selected.

`evaluation_status` must be `not_evaluated`, `passed`, `review_required`, `failed`, or `insufficient_evidence`. No policy must select `not_evaluated`. A protected or expected-change rule violation must select `failed`. A missing required rule measurement or an uncalibrated policy must select `insufficient_evidence` unless an observed violation already selects `failed`. An unresolved compatibility difference or required human decision must select `review_required` unless a stronger failure applies. `passed` must require all required rules and qualification conditions to be satisfied.

Each scalar measurement must identify case, seed population, region, metric identity, domain, units, estimator, sample count, availability, and artifact references. Available measurements must contain `value`. Other states must contain `reason` and no value. Curves, spectra, and maps must retain their sample coordinates, units, dimensions, and hashes.

Each finding must contain `id`, `case_id`, `region_id`, `metric`, `decision`, `evidence`, and the relevant rule identity when policy-derived. Automatic detail records must retain their rectangle, ranking metric, ranking value, and merge/selection method. A prose explanation may supplement but must not replace these fields.

The summary must retain selected and completed case counts, required and unavailable rule counts, protected failures, and worst-case findings. It must not contain a universal appearance score.

## Publication and examples

`run`, `compare`, and `report` must require a destination that does not exist. They must not replace completed evidence. They must publish a terminal manifest only after referenced local artifacts have been finalized and hashed. A handled operation failure may publish a failed terminal manifest and valid partial evidence. Forced termination before the terminal manifest must leave an incomplete directory that cannot be treated as a completed run.

A gate failure must preserve the completed comparison. A report-generation failure must not modify its source comparison. Publication failures must not produce a success result. This contract does not promise a transaction across different destination directories or power-loss durability.

The implementation must ship schema-validated examples for every authored document kind. Command examples must execute with packaged fixture assets and the real renderer. The schema, parser, examples, and validator must change together. The interface must not claim executable examples or a validated schema before these artifacts are exercised.
