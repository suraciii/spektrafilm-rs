# Instrument machine interface

The instrument (`spektrafilm-lab`) is the single research command surface for film-profile development, calibration, controlled experiments, and appearance evaluation. It has one command grammar, one document family, and one machine-output contract.

Normal image editing and delivery do not need it. It computes and publishes evidence; interpretation belongs to the caller and to the policies a document declares.

This document owns the command grammar, the [document registry](#document-registry), the machine-output contract, and the serialized documents. It does not change the renderer's [CLI parameter language](../cli/spec.md). The [benchmark product specification](../benchmark/spec.md) owns evaluation meaning, the [measurement protocol](../benchmark/metrics/spec.md) owns metric domains and algorithms, the [standard configurations](../benchmark/standard-configurations/spec.md) own their rendering purposes, the [profile authoring contract](../profile-authoring/spec.md) owns profile fitting, and [Rust delivery](../benchmark/rust-delivery/spec.md) owns distribution.

## Authority

- Owning module: the instrument binary and its command plumbing. The evaluation library it orchestrates stays renderer-independent and depends only on the shared mathematics crate.
- Consumes: the renderer's public CLI and machine-readable output, the shared parameter resolver, the shared metric implementations, the shared document loader, the shared native-stage bundle format.
- Must not own: rendering mathematics, a second parameter registry, a second metric implementation, a second record format, or a second chain-entry mechanism.

## Unix endpoint model

The instrument is a Unix-style set of smart endpoints connected by dumb file and process boundaries. Each endpoint has one semantic operation, reads declared inputs, and publishes one immutable record or artifact. It must not inspect human output, invoke a downstream endpoint, or decide the next research step.

The boundary between endpoints is an authored document, a retained record directory, or a machine-readable command result. A binary stage buffer remains a referenced artifact rather than an implicit shell value. Paths are explicit whenever provenance, relative dependencies, hashes, or artifact identity matter; stdin is not an undeclared alternate document source.

An endpoint may be used in a shell pipeline through its stdout result, stderr diagnostics, and named artifacts. stdout carries exactly one machine-readable result object, stderr carries human messages and progress, and neither stream is a hidden control channel. A future streaming form must define its carrier and identity explicitly instead of changing the meaning of an existing command.

The instrument command surface is not a workflow language. It has no includes, environment interpolation, shell commands, expressions, conditionals, loops, parameter arithmetic, wildcard selectors, or optimizer instructions. A declaration selects and describes one endpoint invocation; it does not describe a graph of invocations.

The renderer parameter language remains the only parameter DSL. `--preset`, `--params`, and `--set` retain the renderer's syntax and precedence. Instrument documents carry typed process options and research identities, but they do not reinterpret or extend renderer parameter paths. Region geometry, stage selection, seed identity, and artifact destinations are instrument declarations, not runtime parameters.

The canonical research pipe is:

```text
declaration → run → retained evidence → sample → retained readings → compare → inspect/report
```

Each edge is replaceable by a file, a retained directory, or an external tool. The instrument must not collapse this pipe into a command that renders, samples, scores, selects a candidate, and publishes a report as one hidden operation.

The supported command families have distinct endpoint roles:

| Family | Endpoint role | May consume | Must not do |
|---|---|---|---|
| `run suite` | execute one authored image-domain suite | suite and target declarations | execute an image adapter, measure stimuli, sample, compare, or select |
| `run image` | adapt one local image into one image-domain run | image, configuration, and target declarations | execute a suite, measure stimuli, sample, compare, or select |
| `measure run` | execute one declared point-wise stimulus chain | stimuli and chain declarations | execute an image-domain run, sample, compare, or select |
| `sample` | read declared windows from one source | image or retained stage evidence plus `regions` | render, resize, infer geometry, or score |
| `compare runs` | compare two completed image-domain runs | retained run records | compare readings, invoke a renderer, or select a candidate |
| `compare readings` | compare two retained reading records | retained reading records | compare image-domain runs, invoke a renderer, or select a candidate |
| `compare pairs` | create explicitly requested pairs from one retained run | one retained run record | compare two runs, infer a default pair, or invoke a renderer |
| `inspect` / `report comparison` | project retained evidence | completed comparison | recalculate metrics |
| `verify` | re-hash retained identity | one retained record | render, rewrite, compare, or judge |

The parent names `run`, `measure`, and `compare` group leaf endpoints. A parent must not own execution semantics. Each leaf has one input kind, one core operation, one publication boundary, and one handler.

The canonical research pipe is:

```text
declaration → run suite|image or measure run → retained evidence → sample → retained readings → compare runs|readings|pairs → inspect/report comparison
```

Each edge is replaceable by a file, a retained directory, or an external tool. The instrument must not collapse this pipe into a command that renders, samples, scores, selects a candidate, and publishes a report as one hidden operation.

The endpoint name is authoritative. A retained document kind and schema version are validated by the selected endpoint; they must not select another endpoint from field presence or directory shape.

The supported command families carry the compatibility promise of this document and of [Rust delivery](../benchmark/rust-delivery/spec.md). `experiment run`, `experiment export`, `spectral evaluate`, and `fit` remain experimental endpoints under their own specifications. An experimental endpoint may change or disappear between revisions, but its result must still satisfy the document, artifact, and machine-output contracts below.

## Commands

The executable must accept the following canonical command grammar:

```text
spektrafilm-lab --help
spektrafilm-lab --version
spektrafilm-lab schema --kind KIND
spektrafilm-lab validate --kind KIND --file FILE
spektrafilm-lab target --renderer FILE --data-dir DIRECTORY --backend cpu|gpu --out FILE
spektrafilm-lab run suite --suite FILE --target FILE --out DIRECTORY [RUN OPTIONS]
spektrafilm-lab run image --input FILE --configuration FILE --target FILE --out DIRECTORY [RUN OPTIONS]
spektrafilm-lab measure run --stimuli FILE --enter BOUNDARY --out DIRECTORY [CHAIN OPTIONS]
spektrafilm-lab sample --source IMAGE --input-color-space NAME --input-cctf-decoding true|false --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-evidence DIRECTORY --stage STAGE --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-run DIRECTORY|RUN.JSON --case ID --seed UINT32 --stage STAGE --regions FILE --out DIRECTORY
spektrafilm-lab compare runs --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [COMPARISON OPTIONS]
spektrafilm-lab compare readings --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [--reference DIRECTORY]
spektrafilm-lab compare pairs --run DIRECTORY --against input|reference|both --out DIRECTORY
spektrafilm-lab inspect --comparison DIRECTORY [--case ID ...] [--region ID ...] [--metric ID ...]
spektrafilm-lab verify --record DIRECTORY [--data-dir DIRECTORY]
spektrafilm-lab report comparison --comparison DIRECTORY --out DIRECTORY
spektrafilm-lab regions grid --id ID --rows N --columns N --origin X,Y --patch-size WxH --gap X,Y [--sample WxH] [--space SPACE] [--role ROLE] [--measurements FAMILY,...] --out FILE
spektrafilm-lab fixtures --renderer FILE --data-dir DIRECTORY --out DIRECTORY [--natural-input FILE | --download-natural]
spektrafilm-lab experiment run --experiment FILE --out DIRECTORY [CHAIN OPTIONS] [--output-format FORMAT] [--force]
spektrafilm-lab experiment export --stage-evidence DIRECTORY --out DIRECTORY [--stage ID ...] [--force]
spektrafilm-lab spectral evaluate --spectra FILE {--preset NAME ... | --profile FILE} [--set PATH=VALUE|FILE ...] [--data-dir DIRECTORY] --out DIRECTORY
spektrafilm-lab fit [FIT OPTIONS]
```

Each canonical leaf has one typed argument group and one semantic handler. The handler must not select a different contract from optional field presence or from the shape of a retained document.

The former flat forms may remain only as deprecated compatibility aliases during one migration revision. An alias must map to one canonical handler before endpoint execution, must accept no new options, and must reject mixed endpoint options with `argument_syntax`. The alias layer must not contain document-shape dispatch. New documentation and examples must use the canonical grammar. A later compatibility revision may remove the aliases.

The command family names are semantic, not workflow steps. `measure run` publishes one retained measurement; `compare readings` reads only retained records; neither command invokes the next family. A shell pipeline may connect them by paths and result objects, but the instrument does not own that control flow.

`KIND` is a [document registry](#document-registry) kind. `BOUNDARY` is `spectra`, `rgb_in`, `log_e_film`, or `cmy_film`. `STAGE` is `rgb_in`, `log_e_film`, `cmy_film`, `log_e_print`, `cmy_print`, or `rgb_out`. `CHAIN OPTIONS` is the parameter surface of `process`.

Every option of the instrument and of its `fixtures` command must have a nonempty help description. Help, version, and schema discovery must work without a renderer, image decoder, input assets, or graphics device. Version must identify the instrument implementation and supported document versions. Instrument help must identify the `fixtures` command. `fixtures` help must name the command that consumes the generated bundle. The instrument must satisfy [Rust delivery](../benchmark/rust-delivery/spec.md).

`schema` must emit the authoritative JSON Schema for one document kind. For the `metrics` vocabulary it must emit the metric-identity vocabulary instead of a JSON Schema. It must describe the required nested records and explain fields whose units, coordinate space, or applicability are not evident from their type. `validate` must validate a document and resolve the local dependencies needed for that kind. It must not render images. Standalone policy validation must not imply that selectors, measurement selection, or protocol binding have been checked against a suite. A valid target document must not imply that a GPU is available.
`run suite` must resolve one suite and one target before rendering. Repeatable `--case` must select exact case identities in suite order. Duplicate selections and unknown identities must fail. Omitting `--case` must select all cases.

`run image` must resolve one local image, one configuration, and one target before rendering. It must not accept suite-only selectors. It must retain the generated local-image suite and case identity in the run record.

`sample --from-run` reads exactly one completed case row selected by the pair `(case, seed)` from a retained image-domain `run` document. `RUN.JSON` may be the run document itself or its containing directory. The selected row must request and retain available stage evidence; the row's stage manifest reference and the run artifact index must agree on path and digest, and the retained path must remain under `cases/{case}/seed-{seed}/`. The stage bundle remains authoritative for stage layout, domain, units, color interpretation, transfer state, coordinate space, and buffer digest. Missing cases, failed rows, unrequested or unavailable capture, missing stages, malformed artifacts, traversal paths, and cross-seed artifact references are measurement failures; the command never falls back to another seed or stage.

`run --dry-run` must use the same renderer configuration resolution, seed, environment isolation, selected profile containment, and case-owned geometry checks as execution. It must process all selected cases and seeds without decoding or rendering images. Identical seed failures must be grouped with their affected seed values and retained renderer logs. Unknown or duplicate case selections must identify the rejected identities and available choices. Renderer configuration errors must retain the actionable child error text and evidence paths.

The dry-run destination must retain `preflight.json`, suite and target snapshots, resolved configurations, logs, and hashed evidence. A successful preflight must identify `mode: dry_run` and `execution_status: validated`; a failed preflight must identify `execution_status: failed` and structured errors. It must never publish `run.json` or completed render masters. Its `unchecked` list must identify image decoding, compute-device availability, observed output geometry, and numerical results. Preflight does not qualify a run or replace execution-time checks.

`--stage-evidence` must request the renderer's native stage bundle for each simulation. It must accept suite and single-input execution. Capture must require the CPU route with `settings.preview_mode` disabled; unsupported requests must fail with an actionable error, not silently change rendering settings. Preflight must resolve both simulation and source-only configurations, retain the capture request, and identify applied exposure and stage values as unobserved. The request must not enable capture for the separate source-only decode.

`run` must require exactly one of `--suite` and `--input`. Single-input options must conflict with `--suite`; `--case` must apply only to suite execution. `--configuration` must contain the existing case configuration object, with its file paths resolved relative to that file. `--input-kind` must default to `raster`; RAW must be selected explicitly and must use the renderer adapter. Raster input must require both source declaration flags. Repeated `--seed` must select distinct unsigned 32-bit seeds and default to `[0]`. `--regions` must contain the existing region array; omission must select no fixed regions. `--reference` and `--reference-info` must require each other. Reference info must contain the existing reference object except `path` and `sha256`, which the CLI must derive from the selected local file. Nested dependencies must resolve relative to the reference-info file.

`configuration`, `regions`, and `preparation` schema kinds must expose their existing case subdocuments. The `reference` kind must expose reference info without `path` and `sha256`. Standalone validation must check their shape and available local dependencies and identify remaining cross-document applicability checks. It must not establish scene correspondence, successful rendering, or reference suitability without the required observations.

The single-input form must construct and validate one `scene` case with identity `image`, suite identity `local-image`, full-image families `numeric`, `color`, `tone`, and `flip`, and the versioned standard appearance protocol (60 pixels per degree, scales `[1,0.4,0.16]`, flat-mean grain residual, high-pass radius 4, border 1, Hann spectrum, six hotspots of minimum area 151). These settings must be retained in the generated suite, not hidden evaluator state. Case configuration must remain authoritative for crop, resize, and simulation parameters; target overrides must keep their existing applicability. The generated suite must be retained within the run without writing into the source repository.

`--preparation` must contain `original` with `path` and `sha256`, and a nonempty `operations` array of descriptive strings. The original path must resolve relative to that file and enter retained dependencies. The instrument must record but must not execute these operations. Suite cases may contain the same optional `preparation` object. The actual prepared input hash must remain authoritative.

`compare runs` must load two completed image-domain run records and their masters. It must not invoke a renderer. It must match cases by case identity and seeds by their explicit value, not directory order. It must reject incompatible suite or protocol identities. It must retain declared target differences. Reference objectives must use the reference associated with the case.

`compare readings` must load two retained reading records and join them by `(sample_id, stage)`. Duplicate keys fail. Missing keys remain unmatched with an explicit reason. Shared keys with incompatible domain, units, channel order, or coordinate space fail as an identity mismatch. It must not read image-domain masters or invoke a renderer. An optional retained reference is input evidence, not a policy document.

`compare pairs` must load one completed image-domain run and select only the explicitly requested `input`, `reference`, or `both` pairs. It must not compare two runs, infer a default pair, or apply a policy. A selected pair without its required reference must fail with an actionable error. The result must remain `not_evaluated`.

`compare runs` may accept `--policy` and `--gate`. `--gate` must require `--policy`. It must make a non-pass evaluation produce a failing command status after the comparison has been published. Without `--gate`, a successfully published comparison must not fail solely because its evaluation is non-pass.
The comparison source must derive its views from the comparison and cited artifacts. It must not recompute metrics. Its output must remain usable without the run directories by copying required review artifacts and retaining their provenance. An external original that cannot be redistributed must remain excluded from a distributable report; the report must identify the omission rather than substitute another image.

`regions grid` must expand one finite grid declaration into a normal `regions` document without reading an image, invoking a renderer, computing measurements, or generating HTML. It must validate positive dimensions, nonnegative origin and gap, checked arithmetic, sample-window containment, coordinate-space selection, role, and measurement families before publication. The generated document is the only input geometry consumed by later `sample` and run endpoints; downstream endpoints must not interpret a grid declaration.

Irregular geometry must be authored directly as a `regions` document. The instrument must not replace a removed review manifest with repeated `--image`, `--pair`, `--default-pair`, or `--region` options.

Prepared-image visual review, if provided, belongs to the benchmark product's separate command and document contract. It is not an instrument endpoint, is not registered as an instrument document kind, and is not accepted by `report comparison` or `regions grid`.

`inspect` must validate and read a retained comparison without invoking a renderer or computing measurements. It must emit the retained comparison identity, execution and evaluation states, conditions, summary, failures, findings, and compact measurements. The compact measurements must be one top-level `measurements` array in version and single-run comparisons. Rows in both modes must carry the same fields, except that a single-run row must add `pair_id` to identify its pair. Per-seed/pair observations and cross-seed `seed_measurements` must remain separate. Measurement identity, domain, units, estimator, sample count, availability, value or reason, seed population, and artifact references must remain intact. Exact repeatable case, region, and metric filters must reject unknown selections with available choices; they must not silently return an empty result. Referenced evidence must resolve relative to the emitted `artifact_base`. A valid read of a failed comparison must complete successfully while exposing its failed comparison state. Inspection must not modify evidence, rank unlike units, or infer acceptance.

Inspection must expose each selected case and seed's retained `run_evidence` in `case_conditions`, with a `roles` object keyed by run role. These records must include source and target identities, declared preparation, effective conditions, and observed exposure and print normalization when collected. They must retain native-stage artifact links and availability without reading human logs or recomputing image statistics. Region-pair diagnostics must preserve their own identity and both endpoint identities.

Inspection must also project retained `region_readouts` and `stage_readouts` arrays with case and seed identities. Case and region selectors must apply to these arrays; metric selectors must apply only to scalar measurements and findings. `readout_status` must distinguish retained arrays from `not_retained` fields in historical comparisons. Inspection must not backfill historical values.

`target` must construct the existing target document with canonical renderer/data paths, the actual renderer SHA-256, an ID derived from the destination filename, the explicit backend, and empty changes. It must reuse target validation and refuse any existing destination, including a symlink. It must not invoke the renderer, probe a GPU, select simulation settings, or generate test inputs. Fixture targets must share this construction rule while retaining their own IDs, edits, and environment.

Report location links must use `#case=ENCODED_ID&seed=UINT32[&pair=ENCODED_ID[&region=ENCODED_ID]]`, with percent-encoded exact identities. Version pairs use `baseline/candidate`; single-run pairs use their retained IDs. A location must select the matching case/seed/pair, open the requested region, and scroll to it after layout. Navigation must update the fragment and support reload and browser history offline. Detail disclosures must expose keyboard-operable, accessible controls. Malformed or unknown locations must show an accessible error without displaying an unrelated selected result.

Every command result must identify `schema_version`, `command`, `execution_status`, and the terminal artifact when one exists. `schema --kind` emits one schema object without a result wrapper. An execution failure identifies `execution_status: failed` and structured errors. A requested gate failure after successful computation retains `execution_status: completed` and exposes its non-pass `evaluation_status`.

`schema --kind review` and `validate --kind review` must return a stable unsupported-kind error. A report command must accept only retained comparison evidence. The removed manifest source must not be represented as a compatibility alias or a second result kind.

`sample` must validate coordinate-space compatibility before reading any region. Equal dimensions do not establish compatibility. A `source` region is compatible with a declared raster or `source_pixels`; a `render` region is compatible with `output_pixels`; a native stage region is compatible only with the same native stage name. The sampler must not resize, crop, pad, translate, or silently map a mismatch.
`validate` must decide every condition that is decidable from the document, its declared dependencies, and the local filesystem, and must list in `unchecked` every check it did not perform, including checks that need another document, a renderer, a compute device, or a measurement. A check the validator did not perform must not be omitted from `unchecked`, and a condition it could have decided must not appear there. Passing `validate` must never be presented as proof that an execution will succeed.

`run` and `compare` must report progress on stderr as line-oriented text. Every selected case and seed must produce a start line and a completion line. A completion line must identify the item and its elapsed seconds. The last progress output before the command result must state the completed and failed item counts and the total elapsed seconds. Progress must not require a terminal.

A command result must identify `schema_version`, `command`, `execution_status`, and the result artifact when one is produced. An execution failure must identify `execution_status` as `failed` and contain structured errors. A published failed manifest must remain linked from the result; its detailed errors must not replace the command's structured error summary. Human error text must not be the only machine discriminator. A requested gate failure after successful computation must retain `execution_status` as `completed` and expose its non-pass `evaluation_status`.

The executable must return status `2` for argument syntax errors. It must return status `1` for validation, rendering, measurement, publication, or requested gate failure. It must return status `0` when the requested operation completed without those failures. The distinction between execution and evaluation must remain visible in JSON even when both use status `1`.

A run with a failed selected case must have `execution_status` equal to `failed`. A comparison with an unavailable measurement caused by an unsupported reference scope may complete execution but must not pass a policy that requires that measurement. A numerical computation failure must remain an execution failure.

## Carrier and validation

All documents must use UTF-8 JSON and must carry `schema_version` equal to `1`, including the kinds that a suite embeds as subdocuments (`configuration`, `reference`, `regions`, `preparation`) and the retained records. Authored documents must reject duplicate keys and unknown fields. They must not support includes, environment interpolation, expressions, shell commands, or executable plugins.

Schemas must describe document structure, types, ranges, and field applicability. The validator must also check duplicate identities, nonfinite numbers, unsafe artifact paths, and identifiers resolvable from the supplied document. Checks that require file contents must use the declared dependencies. Invalid authored probe point counts, coordinate ordering, mutually exclusive fields, and auxiliary reference document structure must fail validation before rendering. Runtime parameter and source-declaration semantics must use the renderer's shared definitions rather than a second parameter registry.

Generated run and comparison readers may ignore unknown fields within a supported schema version. Required semantic fields must still be validated. Unsupported versions must fail rather than select defaults. Generated documents must preserve full unsigned seed and counter values.

Authored file paths must resolve relative to the document that contains them. A document that names a chain carries parameter values, not a parameter language: it must use the `process` surface's names and precedence, and resolving it must be the resolution the user binaries perform. A parameter file a document names must resolve inside that document's directory tree; a path that escapes it must fail. Executable paths must be explicit paths, not commands resolved from `PATH`. Run and comparison artifact paths must be relative to their containing directory. Artifact paths must not contain an absolute root, parent traversal, or a symbolic link that escapes that directory. File dependencies outside the artifact directory may be consumed through authored paths but must be hashed and identified.

The runner must retain bytes from the filesystem-resolved dependency path. It must preserve the absolute authored path separately for offline lookup, including parent components after symbolic links. Comparison must match that authored identity before looking up a lexically normalized identity from a historical run. Current-directory components must not distinguish equivalent authored paths. Neither lookup may resolve the original filesystem.

## Document registry

Every JSON document the instrument reads or writes has one kind and one owning section. Every document carries `schema_version`.

`schema --kind KIND` must emit the authoritative JSON Schema for every kind, including the nested records a consumer must read to diagnose a result: measurement rows, findings, summaries, failure records, and artifact references. `validate --kind KIND` must accept every authored kind and resolve the local dependencies that kind needs.

| Kind | State | Owner |
|---|---|---|
| `target` | authored | [Target document](#target-document) |
| `suite` | authored | [Suite document](#suite-document) |
| `configuration` | authored | [Suite document](#suite-document) |
| `regions` | authored | [Regions document](#regions-document) |
| `stimuli` | authored | [Measurement](#measurement) |
| `policy` | authored | [Policy document](#policy-document) |
| `reference`, `preparation` | authored | [Suite document](#suite-document) |
| `material_measurement`, `correspondence_review` | authored | [Suite document](#suite-document) |
| `experiment` | authored | [Experiment runs](experiment/spec.md) |
| `run` | retained | [Run record](#run-record) |
| `comparison` | retained | [Comparison record](#comparison-record) |
| `preflight` | retained | [Commands](#commands) |
| `provenance` | retained | [Standard configuration generation](#standard-configuration-generation) |
| `report` | retained | [Commands](#commands) |
| `resolved`, `command`, `export` | retained | [Experiment runs](experiment/spec.md) |
| `evidence` | retained | [Stage evidence](../cli/spec.md#stage-evidence) |
| `metrics` | vocabulary | [Measurement protocol](../benchmark/metrics/spec.md) |

A kind's state decides which commands accept it: an authored kind is written by a person or another tool and may be validated; a retained kind is published by the instrument, is immutable, and is consumed by `compare`, `inspect`, `verify`, `report`, and the matching `sample` adapters (`--from-evidence` or `--from-run`). `metrics` is not a document: it is the closed vocabulary of metric identities a policy may name, and the same vocabulary the measurement protocol owns.

## Measurement

A chain measurement executes the resolved chain on a declared stimulus set and records the boundary values it reads. It computes values and writes artifacts; it must not fit parameters, render images, judge results, or generate pages.

### Stimulus boundaries

A measurement enters the chain at one declared boundary: `spectra`, `rgb_in`, `log_e_film`, or `cmy_film`. They reuse the existing tap vocabulary.

The entry must use the canonical `taps.inject` control, and the executed range must end at or after it. The command must not add a second chain-entry mechanism or a second execution path.

The measured range is declared through the same controls the user binaries read: `--enter` becomes the resolved `taps.inject`, and the collection boundary is the chain's own `taps.collect` when it reaches every requested reading, otherwise the last requested reading. A declared `taps.inject` that contradicts `--enter`, a `taps.collect` that stops before a requested reading, and a reading upstream of the entry must all fail before executing, each naming the conflict. The retained effective parameter document therefore describes the range that executed, and `verify` re-resolves the same range from the recorded declaration.

A stimulus that enters at `rgb_in` declares its color space and its transfer state in the stimulus document, exactly as a raster input to `process` does. A stimulus that enters at `spectra` declares its color space, its transfer state, and its illuminant pair in the stimulus document; the command must not accept these declarations as command options, because the document is their single authority. A film-domain stimulus declares neither, because its values are already film-domain numbers.

The `spectra` entry uses the same front half as [spectral evaluation](spectral-evaluation/spec.md). `spectral evaluate` remains the roundtrip-error report against truth; `measure run` reports readings and must not compute a truth value.

### Point-wise requirement

Readings are defined only for a point-wise chain: no operation in the measured range mixes samples, measures the whole frame, resizes it, or draws stochastic values.

An operation lies inside the measured range when it executes and can change a requested reading. Both are decided by the stage the operation first changes, which the instrument must establish by measurement rather than by the flag's name:

| Operation | First stage it changes |
|---|---|
| `io.upscale_factor`, `io.crop` | `log_e_film` |
| `camera.auto_exposure`, `camera.diffusion_filter.active`, `camera.lens_blur_um` | `log_e_film` |
| `film_render.halation.active` | `log_e_film` |
| `film_render.dir_couplers.active` | `cmy_film` |
| `film_render.grain.active` | `cmy_film` |
| `enlarger.diffusion_filter.active` | `log_e_print` |
| `print_render.glare.active`, `scanner.lens_blur`, `scanner.unsharp_mask`, `scanner.auto_setup` | `rgb_out` |

An operation executes when the range passes through it: the entry boundary is where the stimulus enters, so an operation whose first changed stage is at or before the entry cannot reach a reading, and an operation whose first changed stage is after the last requested reading has not changed one yet. An operation is listed when both hold.

Which stages execute follows the executed path, not the route alone. The whole chain (the stimulus injected at `rgb_in` and collected at `rgb_out`) runs the route's own stage list: `input` returns after the crop and rescale, the convert routes build the film density by inverting the scan model and never run the metering or the development stages, the print stage needs a print route, `io.scan_film` scans the developed film directly instead, and the scanner runs on every route but `input`. A narrower range walks the tap topology from the entry to the collection boundary instead, and the nodes inside that range fire whatever the route says: only the metering stays skipped on the convert routes, and only `io.scan_film` still leaves the print taps out. That is why the route alone cannot decide eligibility: measured on the 12-sample chart, the whole chain with `--read rgb_out` leaves the collected buffer byte-identical while toggling `camera.auto_exposure`, `film_render.halation.active`, `film_render.dir_couplers.active`, `film_render.grain.active`, or `print_render.glare.active` on `input > convert-film > scan`, `input`, and the camera and development controls on `input > convert-film > print > scan`; and `input` with `--enter rgb_in --read log_e_film` moves every `log_e_film` reading by the metered stop (0.1618 in log10) when `camera.auto_exposure` is disabled, because that range runs the metering node.

An operation the executed path skips is not listed; neither is a controller whose active condition the core does not read (`print_render.glare.percent` at zero) nor one with no reader at all (`enlarger.lens_blur` is applied by no stage, see the parity matrix). The measured table is evidence, not a style choice: on a 12x4 chart, halation moves `log_e_film` by 1.9e-02 and the DIR couplers move `cmy_film` by 8.5e-01 while leaving `log_e_film` exact.

The command must resolve the chain before executing it. When one of these operations is active in the measured range, the command must fail before executing, and it must name the `--set` edits that disable it. The command must not disable an operation on the caller's behalf.
### Unix-style endpoint acceptance

The following scenarios are product acceptance, not help-text examples:

1. A retained stage bundle can be passed to `sample`, and its declared stage domain, units, channel order, transfer state, buffer digest, and coordinate space are read from the bundle rather than repeated on the command line.
2. A `regions` document can be reused unchanged to sample two retained runs and an explicitly declared image source. The three commands produce records with stable sample identities and no inferred geometry.
3. `compare` reads the two retained reading records and writes per-sample deltas and group summaries without invoking a renderer. A shared key with different units or channel order fails; a missing key is reported as unmatched; an unavailable metric carries a reason and never serializes as zero.
4. Capturing stdout separately from stderr yields one parseable result object on stdout while progress and human diagnostics remain on stderr. Redirecting stderr or piping stdout through a JSON consumer does not change command behavior.
5. A command cannot make a downstream decision: `run` does not sample or compare, `sample` does not score, `compare` does not select a candidate, and `report` does not recalculate a metric.
6. A declaration containing an include, environment substitution, expression, shell command, wildcard selector, or optimizer instruction is rejected as an unknown field or unsupported syntax before execution.
7. An experiment declaring seeds `[0, 42]` either publishes two independently identified retained results or fails before rendering with a stable validation error; it must not publish one result while recording both seeds as observed.
8. The future explicit forms `compare runs` and `compare readings` select their handlers from the command grammar, not from the presence or absence of fields in `run.json`.

The acceptance fixture must use one ordinary 45-region declaration and one non-grid explicit regions declaration. Passing the fixture must prove that the endpoint contract is generic and does not encode a TE226-specific geometry or a universal appearance score.

### Stimulus document

A stimulus document declares the values a chain measurement injects. It must contain `schema_version`, an `id`, a `domain`, and the sample arrays beside it as `.npy`/`.npz` files.

`cctf_decoding` must name the state the chain reads the stored values in, matching `io.input_cctf_decoding`: `true` when the stored values are transfer-encoded and the chain must decode them, and `false` when they are already transfer-decoded.

`domain` must match the `--enter` boundary and be one of `reflectance`, `scene_rgb`, `film_log_e`, or `film_density`. A `scene_rgb` document must declare `color_space` and `cctf_decoding`, exactly as a raster input to `process` declares them. A `reflectance` document must declare `color_space`, `cctf_decoding`, and its illuminant pair as `condition: {"reference": ..., "target": ...}`. A film-domain document must declare none of them, because its values are already film-domain numbers.

The document must declare the wavelength grid or the band it covers, the per-sample `source` and `partition` labels, and the provenance hashes of the arrays it names. A sample outside the declared band is invalid. The command must not clip or extrapolate it.

### `measure run`

```text
spektrafilm-lab measure run --stimuli PATH --enter BOUNDARY [--read STAGE ...] [--dry-run] [CHAIN OPTIONS] --out DIRECTORY
```

- `--stimuli` declares a [stimulus document](#stimulus-document).
- CHAIN OPTIONS are the parameter surface of `process`: `--preset`, `--film`, `--paper`, `--set`, `--route`, `--scan-output`, and `--data-dir`. The command must resolve them with the shared resolver. It must not accept a stored parameter document as the chain declaration.
- `--read` selects stages from the existing tap vocabulary. Omitting `--read` must read every available stage.
- The command must execute the core-owned pipeline path of the resolved chain. It must not walk the stages beside an executed run or reconstruct pipeline mathematics.
- The command must not write image output.
- `--dry-run` must resolve the chain, evaluate the point-wise requirement, and write the identity record only. It must not execute the chain, write `readings.csv`, or publish a reading.

### `sample`

```text
spektrafilm-lab sample --source IMAGE --input-color-space NAME --input-cctf-decoding true|false --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-evidence DIRECTORY --stage STAGE --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-run DIRECTORY|RUN.JSON --case ID --seed UINT32 --stage STAGE --regions FILE --out DIRECTORY
```

- `--source` declares a raster image. It must require the same input declaration as `process`: a source color space and a transfer state.
- `--from-evidence` reads one stage of a retained evidence bundle. The coordinate space, channel order, and units of that stage must come from the bundle manifest.
- `--from-run` resolves exactly one completed `(case, seed)` row from a retained image-domain run, then reads its requested available stage from the indexed `stage-evidence.json`. The run record and stage bundle are authoritative; missing or failed rows, unrequested or unavailable capture, missing stages, malformed artifacts, unsafe paths, digest mismatches, and cross-seed references must fail without fallback.
- `--regions` declares reading positions with stable sample identities: explicit windows with their labels and groups, in the coordinate space the document declares. A window that leaves the source bounds must fail with the sample identity. The command must not clamp, pad, extrapolate, or repair a sample.
- The command must not resize, re-encode, or convert the source beyond the declared input declaration.
- A reading describes the values it stores: applying the declared transfer decoding makes the reading linear, and its record must say so while the source block keeps the source's own declaration. A stage that declares no transfer state records none.
- Every record carries the common fields, including `executed`: a sample never executes a chain.

### `verify`

```text
spektrafilm-lab verify --record DIRECTORY [--data-dir DIRECTORY]
```

- The command must read the record's kind and re-hash what that record names. It must not render, rewrite, or judge the record.
- For an image-domain run it must re-resolve the retained suite, target, and case configuration, and re-hash the renderer executable, the selected data inventory, the profiles, and the authored dependencies.
- For a chain run it must re-resolve the recorded declaration, and re-hash the renderer executable, the selected data inventory, and the profiles.
- For a comparison it must re-hash the retained run and comparison identities and the evidence the comparison cites.
- It must report every difference from the recorded identity.
- It must re-hash every stage buffer the record retained, whether or not the reading requested that stage.
- It writes no artifact; it prints its report, so a verification cannot overwrite the record it verifies.

### Guard runs

A guard run is a retained measurement of a frozen stimulus set. It uses the same commands and artifacts as any other measurement. The instrument must not add a guard command, flag, or tolerance field. A comparison against retained readings is `compare`.

## Regions document

A regions document declares reading positions. One kind serves both reading domains: an image-domain case, a sampling command, and a reference declare regions in it.

A regions document must contain `schema_version`, an `id`, and a nonempty `regions` array. A region must contain `id`, `geometry`, `role`, and `measurements`, and may contain `label`, `source`, and `partition`. `role` must identify `skin`, `neutral`, `shadow`, `highlight`, `color`, `flat_field`, `edge`, `texture`, or `other`. `measurements` must select applicable measurement families. `source` and `partition` are group labels for reporting. Identities must be unique within the document, must match `^[A-Za-z0-9][A-Za-z0-9_.-]*$`, and `full_image` stays reserved for the whole declared source.

`geometry` must be either a half-open rectangle with the named fields `x`, `y`, `width`, `height`, or a lossless mask with its `path`, `sha256`, and declared `width` and `height`. Widths and heights must be positive integers. Every geometry must declare its coordinate space. `source` names the declared source image's pixels; `render` names a completed simulation's output pixels; native stage names are `source_pixels`, `working_pixels`, `film_frame_pixels`, `grain_aperture_pixels`, or `output_pixels`. Mask paths must resolve relative to the document that contains them and enter the retained dependency inventory as immutable evidence.

Regions may overlap: a reading carries its own identity, and a narrower window inside a wider one is a declared intent, not a conflict. Correspondence is never inferred from equal dimensions, labels, or shared region identities.
`regions grid` is the only instrument geometry generator. It writes a normal `regions` document and does not read images or produce a review page. Non-grid geometry is authored directly. A suite case, a sampling command, and a reference may consume the same regions shape through one validator; benchmark image-domain consumers currently accept only `source` and `render`, while native stage values require a producer mapping and are rejected there with `unsupported_coordinate_space` until one exists.

## Suite document

A suite must contain `schema_version`, `id`, `protocol`, and a nonempty `cases` array. `id` must be a stable nonempty string. `protocol` must select `appearance-v1` and contain its declared `pixels_per_degree`, region sampling scales, grain residual and spectrum settings, and hotspot selection settings. Every protocol value that changes a measurement must contribute to the resolved protocol digest.

A case must contain `id`, `kind`, `input`, `configuration`, `seeds`, `regions`, and `measurements`. `kind` must be `probe` or `scene`. Case identities must be unique. `seeds` must be a nonempty array of distinct integers in the renderer's seed domain. Each selected case must be rendered for each selected seed.

`input` must contain `path` and `sha256`. Raster input must also contain `color_space` and `cctf_decoding`. RAW input must use the adapter declaration and may contain the dedicated RAW options specified below. The input hash must match before decoding. File extension must not infer a color declaration.

`configuration` must use the following fields: `film`, `paper`, `presets`, `params`, `set_files`, `set`, `route`, `scan_output`, and `raw`. Fields not required by the renderer may be omitted. `presets` must contain explicit preset-file paths, not mutable user-library names. `params` must be a legacy parameter snapshot path. `set_files` must contain sparse parameter-file paths. Every `set` entry in case configuration, target changes, or case changes must contain renderer-compatible `PATH=VALUE` assignments, including comma-separated assignments with quoted strings and arrays; file carriers must occur only in `set_files`. File edits must apply in listed order before listed inline edits. Their semantic validation and precedence must follow the renderer's resolver. No instrument field may replace a runtime field alias.

`raw` may contain `white_balance`, `temperature`, `tint`, and `lens_correction` with the dedicated renderer option semantics. These fields must apply only to RAW cases. An incompatible combination must fail before rendering.

A case's `regions` element must use the region shape the [regions document](#regions-document) defines, in render coordinates. Region identities must be unique within a case. Bounds must be validated against the actual master. Fixed regions must not be silently resized to fit a changed render.

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
## Prepared-image review boundary

The instrument does not define or accept a prepared-image review manifest. `review` is not a document-registry kind, and `schema --kind review` and `validate --kind review` must return a stable unsupported-kind error. The instrument's report source is a retained comparison only.

The benchmark product may retain a separate prepared-image viewer for local visual inspection. Its image inventory, per-image geometry, default pair, captions, display conversion, and report builder belong to the benchmark command and its own contract. The instrument must not import that manifest, derive regions from it, shape-dispatch on it, or expose aliases for it.


## Target document

A target must contain `schema_version`, `id`, `renderer`, `data_dir`, `backend`, and `changes`. `renderer` must contain an executable path and its SHA-256. `backend` must be `cpu` or `gpu`. `data_dir` must be explicit. Renderer identity validation must precede execution.

`changes` must contain arrays `set_files` and `set`, which may be empty. They must apply after each case's configuration using the existing sparse-file and inline-assignment semantics. They must not edit seed, input identity, or geometry outside the permitted renderer fields. The comparison must expose the resolved differences rather than trusting a change description.

A target may contain `case_changes`, keyed by exact case identity. Each value must use the same `set_files` and `set` fields and must apply after target-wide changes. Unknown case identities must fail. This surface must not support conditions or wildcard case selection.

Explicit renderer and data identities must support executable or profile/data revisions without parameter edits. The run must hash the complete selected data directory with a deterministic relative-path inventory. The resolved inventory must identify file bytes, not modification times. Before any rendering, the runner must check the actual film and paper profile names selected by the renderer's dry run, including preset-provided references, and reject profile paths that resolve outside the target's data directory. Target changes must not silently substitute a profile file from another directory. A data directory that cannot be inventoried, including one containing symlinks, must not abort the run without a manifest: the run must still publish `run.json`, must fail every case that did not already fail for a more specific reason such as profile confinement, and must remain ineligible as a qualified baseline.

The target must not own output container settings, input geometry, or seed selection. The runner must control artifact paths, request floating TIFF masters, preserve scanner output color and transfer state, and request execution diagnostics. Source declarations must remain case-owned. Before image rendering, the runner must compare target-resolved input geometry and `settings.preview_mode` / `settings.preview_max_size` against the case-only dry resolution. Target-wide and case-specific changes, whether inline assignments or sparse files, must fail if these case-owned resolved fields differ.

The runner must isolate user preset configuration and remove inherited simulation-control environment values before rendering. An allowlist of environment variables that influence loading, numerical execution, or device selection must be recorded with the instrument implementation. Allowed values must contribute to run identity. Experimental environment controls must require an explicit target extension in a new interface version; unrecorded experimental values must not affect a qualified run.

## Policy document

A policy must contain `schema_version`, `id`, `protocol_digest`, `calibration`, and `rules`. It must bind to the resolved measurement protocol. `calibration` must identify hashed evidence, its case population, reviewed labels, observation scale, and scope. An uncalibrated exploratory policy must identify that state and must not produce `passed` qualification.

The resolved protocol digest must include case source declarations and hashes, selected measurement families, probe definitions, region geometry and mask hashes, reference dependencies, and seed populations. It must also include the instrument implementation revision, metric and color-transform versions, relevant numerical dependency versions, and the LittleCMS version and transform settings for ICC references. `validate --kind suite` must expose this digest for policy authors. Cross-implementation evidence and policy handling must follow [Rust delivery](../benchmark/rust-delivery/spec.md#existing-evidence-and-protocol-identity).

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

Completed runs must retain a floating `source_raw` with `source_raw_metadata` from the real renderer's input route, including source color interpretation and observed dimensions. The input route must use the same RAW decoding options and case-owned geometry as simulation, force output to linear ACES2065-1, disable the independent CMYK ink style, and force `taps.inject=rgb_in` and `taps.collect=rgb_out`. Source configuration and observed diagnostics must remain retained dependencies of an input comparison. Original source dimensions and preparation metadata must remain separately identified. Input output geometry must be checked before declaring pixelwise correspondence with simulation.

Completed case artifact references must be objects with an indexed `path`. A malformed required reference must return a structured command failure.

The artifact index must identify relative path, content hash, media type, role, and dimensions or sample layout when applicable. All retained paths must resolve. Partial work must not be indexed as a complete master. A failed run must retain successfully completed cases and explicit failures, but must not be reused as a qualified baseline.

A requested native stage bundle must be retained under the case's `stage_evidence` reference. Its manifest and every buffer must be indexed artifacts. Validation must check the producer schema version, terminal state, supported numerical layout, byte length, finite samples, content hashes, stage identities, geometry, and declared availability. A missing or failed required capture must prevent a completed run. A stage skipped by the selected workflow must remain not applicable rather than become zero-valued evidence. The renderer's native-stage contract owns buffer and preparation semantics; the instrument must not reconstruct them.

### Chain runs

A chain run writes the same `run` kind. It must carry `kind` set to `run` or `sample`, so `verify` can read a record without being told what it is.

- `declaration`: the chain options, the entry boundary, the read stages, and the declared stimulus document with its hashes — everything needed to resolve the same chain again.
- `stimulus`: the stimulus manifest path and the array hashes. A sampled source records its own `source` block instead, because no chain was resolved to read it.
- `identity` for an executed run: the renderer's path, content hash, and version, the selected data inventory digest, the profile identities, the declared parameter sources and the content hash of every parameter file they name, the stock-specific application, the effective parameter document, the execution precision, and the seed. `verify` diffs that document leaf by leaf, so a re-verification names the parameter that moved instead of only its digest.
- `stages`: one record per read stage with its identity, domain, units, channel order, the declared color space and transfer state, the buffer hash, and its availability together with a stable reason when the stage did not run.
- `groups` and `summaries`: the group identities and their per-channel statistics.
- `executor`: the backend and the precision of an executed run.
- `evidence/`: the retained stage buffers of the executed run, in the form the user binaries write.
- `readings.csv`: one row per sample and stage — `sample_id`, `source`, `partition`, `stage`, `domain`, `units`, `channel_order`, the channel values, and `valid`. A value that is not finite is an empty field with `valid` false; a placeholder number must never be written in its place.

A declaration must be replayable from any working directory: a declared parameter or preset file is recorded with its canonical path and content hash. Identity fields reuse the vocabulary the renderer and the image-domain records already use (`renderer`, `data_dir`, the backend) rather than inventing a competing name.

Where a record restates a shared convention — the interpolated percentile estimator, the bounded-display color distance — the specification of the shared owner is authoritative and the record names it.

## Comparison record

A comparison directory must contain `comparison.json`, input run identities, the policy snapshot when selected, scalar measurements, map and curve artifacts, and selected detail-region records. Every finding must resolve to retained artifacts or verified source run artifacts.

`comparison.json` must contain `schema_version`, `kind` equal to `comparison`, `id`, `execution_status`, `evaluation_status`, `protocol`, `selection`, `condition_differences`, `cases`, `findings`, `summary`, and `artifacts`. Version comparisons must contain `baseline` and `candidate`, and must contain `policy` when selected. Single-run comparisons must contain `mode` equal to `single_run`, `run`, and `default_pair` equal to `input` or `reference`. Each case must retain a `pairs` array with unique `id`, `left_role`, `right_role`, `pixel_aligned`, `status`, pair-owned measurements and review artifacts, and reference applicability when selected. Input pairs must use roles `input` and `simulation`; reference pairs must use `reference` and `simulation`. Unavailable image evidence must remain labeled and must not fall back to another role. Pair ids must also identify scalar measurements and findings in single-run results.

Each completed case must retain `run_evidence` keyed by `baseline` and `candidate` for a version comparison, or `simulation` for a single-run comparison. It must preserve run, target, input, data and configuration identities, declared preparation, actual command and observed stage context. Referenced native bundles must be copied with their relative buffer dependencies into the comparison artifact index. Report and inspection consumers must not require the original run directory to recover this evidence.

New comparisons must retain `region_readouts` on version case records or single-run pair records, and `stage_readouts` once per case. The [comparison schema](../../crates/spektrafilm-benchmark/assets/comparison.schema.json) owns their field structure. The [measurement protocol](../benchmark/metrics/spec.md#retained-region-coordinates) owns estimators and eligibility. Historical version-1 records may omit these arrays. Consumers must not interpret omission as an empty measured population or a zero result. Available observations must contain finite values; unavailable observations must contain a reason and no value. Color readouts must identify `pair_id`; native-stage readouts must identify `run_role`. Artifact references must resolve inside the retained comparison.

`evaluation_status` must be `not_evaluated`, `passed`, `review_required`, `failed`, or `insufficient_evidence`. No policy must select `not_evaluated`. A protected or expected-change rule violation must select `failed`. A missing required rule measurement or an uncalibrated policy must select `insufficient_evidence` unless an observed violation already selects `failed`. An unresolved compatibility difference or required human decision must select `review_required` unless a stronger failure applies. `passed` must require all required rules and qualification conditions to be satisfied.

Each scalar measurement must identify case, seed population, region, metric identity, domain, units, estimator, sample count, availability, and artifact references. Available measurements must contain `value`. Other states must contain `reason` and no value. Curves, spectra, and maps must retain their sample coordinates, units, dimensions, and hashes.

Each finding must contain `id`, `case_id`, `region_id`, `metric`, `decision`, `evidence`, and the relevant rule identity when policy-derived. Automatic detail records must retain their rectangle, ranking metric, ranking value, and merge/selection method. A prose explanation may supplement but must not replace these fields.

The summary must retain selected and completed case counts, required and unavailable rule counts, protected failures, and worst-case findings. It must not contain a universal appearance score.

### Reading comparisons

A comparison of two retained readings matches rows by composite sample and stage identity. Two sides that disagree on the domain, unit, or channel order of a shared key are an incompatible identity set: the command must fail and name the mismatch instead of truncating the join. Ambiguous identities, a repeated key inside one side, must fail as well. Keys that only one side carries are reported as unmatched with the side that carries them.

Channel deltas are published only for pairs whose color space and transfer state agree, including two absent declarations. A pair whose declarations differ reports no channel deltas and an unavailable reason. The color-distance metric converts through linear sRGB into the bounded-display Lab domain, states that domain in its records, and marks a reading it had to bound.

Availability is decided per pair: the baseline/candidate, candidate/reference, and baseline/reference pairs each decide from their own two sides and their own metric. A decision must never be borrowed from another pair.

Each comparison carries its own name, so a record cannot be read as another comparison: the primary comparison is `candidate_minus_baseline`, and a reference adds `candidate_minus_reference` and `baseline_minus_reference`.

The group labels of a reading (`source`, `partition`) are provenance, not identity: two sides that label one shared key differently must report the label difference as an identity difference and still compare the values. Only the value's own declaration (domain, units, channel order) makes a shared key incomparable.

An identity difference between the two runs is evidence, not an error. The command must report the difference together with the numeric deltas. It must not refuse a comparison because the two chains differ; detecting that difference is the purpose of a retained reference reading.

`--reference-run` adds a third retained reading. Its similarities and differences must be reported under its own role, not relabelled as the baseline or the candidate. A reference that shares no sample and stage with the pair, or whose readings are not eligible for the metric, must report an unavailable distance with a stable reason, and the comparison must still publish its other measurements. An unavailable measurement must never serialize as a distance, as zero, or as an absent row without a reason.

The command must compute per-sample deltas and group summaries through the shared metric implementation. It must not add a second color-difference, equal-lightness, or region-aggregation algorithm. It must read the artifacts the measurement wrote, including their quoting: a sample identity with a comma, a quote, or a line break must survive the round trip.

A reading comparison writes `deltas.csv` with `pair`, `sample_id`, `source`, `partition`, `stage`, `domain`, `units`, the minuend side's channels, the subtrahend side's channels, the channel deltas, the color distance, `valid`, and `clipped`. One row per joined sample and stage per comparison, so reference comparisons keep their per-sample rows; `d` is the minuend side minus the subtrahend side of that comparison; the distance column stays empty when the stage declares no color space; `clipped` counts the channels the distance had to bound; and a pair whose two sides declare different color interpretations writes no channel deltas, with empty delta fields and `valid` false, so a file-level aggregation cannot mix a pair the record itself calls incomparable.

## Publication and examples

`run`, `compare`, and `report` must require a destination that does not exist. They must not replace completed evidence. They must publish a terminal manifest only after referenced local artifacts have been finalized and hashed. A handled operation failure may publish a failed terminal manifest and valid partial evidence. Forced termination before the terminal manifest must leave an incomplete directory that cannot be treated as a completed run.

A gate failure must preserve the completed comparison. A report-generation failure must not modify its source comparison. Publication failures must not produce a success result. This contract does not promise a transaction across different destination directories or power-loss durability.

The implementation must ship schema-validated examples for every authored document kind. Command examples must execute with packaged fixture assets and the real renderer. Documented instrument usage examples must be checked against the implemented command and flag surface by a permanent test. A documented command or flag the executable does not accept must fail that test. The schema, parser, examples, and validator must change together. The interface must not claim executable examples or a validated schema before these artifacts are exercised.

An executable example must use fixture generation, `validate`, `run`, `compare`, and `report` to complete a comparison without source-code inspection. The example must read metric identities, units, availability, and evidence links from the resulting measurements before authoring a policy. It must identify the exploratory policy's qualification limit. This workflow must use the existing commands and document fields; it must not require a new discovery language.

## Standard configuration generation

The `fixtures` command must accept this grammar without a new selector or configuration language:

```text
spektrafilm-lab fixtures --renderer FILE --data-dir DIRECTORY --out DIRECTORY [--natural-input FILE | --download-natural]
```

`--natural-input` and `--download-natural` must be mutually exclusive. The generator must preserve the existing pinned natural-source validation and attribution rules.

Every invocation must generate the existing probe assets, suites, targets, exploratory policy, and provenance. It must also generate `look.toml`, `color-tone.json`, and `appearance.json`. `look.toml` must use the renderer's portable preset schema. The two JSON files must contain the existing standalone `configuration` object, not a new document envelope. The [standard configuration product contract](../benchmark/standard-configurations/spec.md) defines their rendering purposes and suite coverage.

Both configurations must reference `look.toml` through `presets: ["look.toml"]`. They must use existing `route`, `set_files`, and `set` carriers. Their paths must resolve relative to the containing configuration file. Generated suite cases must carry the equivalent configuration objects because the suite schema has no configuration include field.

Both configuration objects must set `route` to `input > film > print > scan` and `set_files` to an empty array. Their ordered `set` arrays must begin with these shared assignments:

```json
[
  "camera.auto_exposure=false",
  "camera.exposure_compensation_ev=0",
  "camera.film_format_mm=35",
  "scanner.auto_setup=off",
  "scanner.finish=neutral",
  "cmyk.active=false",
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

The packaged usage example must generate once, use `color-tone.json` and `appearance.json` with a declared local image, and select seeds `0`, `42`, and `99` explicitly for full appearance. It must show how to construct a second target and reuse the same bundle for a version comparison. It must use the existing `run`, `compare`, `inspect`, and `report` commands. These examples must be executed during delivery acceptance rather than presented as already supported.

## Acceptance

- `schema --kind` must cover every kind in the [document registry](#document-registry), and `validate --kind` must accept every authored kind, including the kinds a suite embeds and the experiment document.
- A command that fails must name a stable error code, the file the offending value lives in, and its JSON Pointer; a command that reads more than one document must name which document failed.
- `validate` must decide every locally decidable condition, including a probe's point count and ordering and a policy rule's metric identity against the selected families, and must list every other check in `unchecked`.
- Repeated runs on identical inputs must produce identical artifacts.
- On the `log_e_film` stage, a reading must match the `log_e_film` buffer of `process --stage-evidence` within a relative tolerance of 1e-9, with the operations the guard names for that range off.
- On a default chain, `measure run` must fail before executing and name every active operation in the measured range with its remediation, and a `--read log_e_film` range must list only the operations that reach `log_e_film`.
- With a frozen chart image and a frozen regions document, `sample` must reproduce the frozen values, and sampling a retained encoded stage must record a linear reading beside the source's own encoded declaration.
- A reference that is unmatched or ineligible must report an unavailable reason per pair, and the comparison must still publish its other measurements.
- A route that skips a boundary must not list that boundary's operations, and a range that executes them must not rely on the route: a chain measured through `input > convert-film > scan` with the camera and development controls left active must complete when it covers the whole chain, while the same chain on a route that runs them must fail and name them, and a narrower range on a skipping route (`input` with `--enter rgb_in --read log_e_film`, `input > convert-film > scan` with `--read cmy_film`) must fail and name the stages its tap topology fires.
- A comparison must publish each comparison's own rows and own availability: every `deltas.csv` row must name its comparison, a shared key whose group labels differ must still compare with the label difference reported, and a key whose units or channel order differ must fail.
- A run that fails after executing, and a sampling of a bundle whose buffer disagrees with its manifest, must leave no artifact behind.
- A record must carry `kind`, and `verify --record` must read a run or a comparison without being told which one it is.
- Every artifact must survive a round trip: a command must read back what it wrote, including identities that carry separators.

## Laws

1. One fact, one name: a fact shared by a command, a document, and a record is spelled the same way in all three.
2. A retained record is immutable and self-describing: it names its kind, its schema version, and the identity that produced it.
3. An unavailable measurement is never zero and never an absent row without a reason.
4. Validation never claims more than it checked.
5. No second implementation of a metric, a parameter language, a record format, or a chain-entry mechanism.
6. Publication is all-or-nothing per destination: a reader may read the presence of a record as the presence of a complete result.
7. Stability is bound to a kind and its schema version, never to a command name.
8. The instrument decides nothing about quality: it computes values, publishes evidence, and applies only the policy a document declares.
