# Instrument machine interface

The instrument (`spektrafilm-lab`) is the single research command surface for film-profile development, calibration, controlled experiments, and appearance evaluation. It has one command grammar, one document family, and one machine-output contract.

Normal image editing and delivery do not need it. It computes and publishes evidence; interpretation belongs to the caller and to the policies a document declares.

- This document owns the command grammar, the [document registry](#document-registry), the machine-output contract, and the serialized documents. It does not change the renderer's [CLI parameter language](../cli/spec.md).
- The [benchmark product specification](../benchmark/spec.md) owns evaluation meaning, the [measurement protocol](../benchmark/metrics/spec.md) owns metric domains and algorithms, the [standard configurations](../benchmark/standard-configurations/spec.md) own their rendering purposes, the [profile authoring contract](../profile-authoring/spec.md) owns profile fitting, and [Rust delivery](../benchmark/rust-delivery/spec.md) owns distribution.
- Ordinary suite/image execution MUST NOT provide optional native capture. Dedicated experiment/measurement endpoints MUST use the common core observer.
- Experiment export takes positional `INPUT`. [Native observations](native-observations.md) owns the retained scientific wire contract; [stage artifacts](../cli/stage-artifacts.md) separately owns resumable film/print states, and [debug diagnostics](../cli/spec.md#debug-diagnostics) owns human process diagnostics.

## Authority

- Owning module: the instrument binary and its command plumbing. The evaluation library it orchestrates stays renderer-independent and depends only on the shared mathematics crate.
- Consumes: the renderer's public CLI and machine-readable output for ordinary image runs, the shared parameter resolver, shared core observer for dedicated research, shared metric implementations, shared document loader, and [native observation bundle format](native-observations.md).
- MUST NOT own: rendering mathematics, a second parameter registry, a second metric implementation, a second record format, or a second chain-entry mechanism.

## Unix endpoint model

The instrument is a Unix-style set of smart endpoints connected by dumb file and process boundaries. Each endpoint has one semantic operation, reads declared inputs, and publishes one immutable record or artifact. It MUST NOT inspect human output, invoke a downstream endpoint, or decide the next research step.

The boundary between endpoints is an authored document, a retained record directory, or a machine-readable command result. A binary stage buffer remains a referenced artifact rather than an implicit shell value. Paths are explicit whenever provenance, relative dependencies, hashes, or artifact identity matter; stdin is not an undeclared alternate document source.

An endpoint MAY be used in a shell pipeline through its stdout result, stderr diagnostics, and named artifacts. stdout carries exactly one machine-readable result object, stderr carries human messages and progress, and neither stream is a hidden control channel. A future streaming form MUST define its carrier and identity explicitly instead of changing the meaning of an existing command.

The instrument command surface is not a workflow language. It has no includes, environment interpolation, shell commands, expressions, conditionals, loops, parameter arithmetic, wildcard selectors, or optimizer instructions. A declaration selects and describes one endpoint invocation; it does not describe a graph of invocations.

- The renderer parameter language remains the only parameter DSL. `--preset`, `--params`, and `--set` retain the renderer's syntax and precedence.
- Instrument documents carry typed process options and research identities, but they do not reinterpret or extend renderer parameter paths. Region geometry, stage selection, seed identity, and artifact destinations are instrument declarations, not runtime parameters.

The canonical research pipe is:
```text
declaration → run → retained evidence → sample → retained readings → compare → inspect/report
```

Each edge is replaceable by a file, a retained directory, or an external tool. The instrument MUST NOT collapse this pipe into a command that renders, samples, scores, selects a candidate, and publishes a report as one hidden operation.

The supported command families have distinct endpoint roles:

| Family | Endpoint role | MAY consume | MUST NOT do |
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

The parent names `run`, `measure`, and `compare` group leaf endpoints. A parent MUST NOT own execution semantics. Each leaf has one input kind, one core operation, one publication boundary, and one handler.

The canonical research pipe is:
```text
declaration → run suite|image or measure run → retained evidence → sample → retained readings → compare runs|readings|pairs → inspect/report comparison
```

Each edge is replaceable by a file, a retained directory, or an external tool. The instrument MUST NOT collapse this pipe into a command that renders, samples, scores, selects a candidate, and publishes a report as one hidden operation.

The endpoint name is authoritative. A retained document kind and schema version are validated by the selected endpoint; they MUST NOT select another endpoint from field presence or directory shape.

The supported command families carry the compatibility promise of this document and of [Rust delivery](../benchmark/rust-delivery/spec.md). `experiment run`, `experiment export`, `spectral evaluate`, and `fit` remain experimental endpoints under their own specifications. An experimental endpoint MAY change or disappear between revisions, but its result MUST still satisfy the document, artifact, and machine-output contracts below.

## Commands

The executable MUST accept the following canonical command grammar:
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
spektrafilm-lab experiment export INPUT --out DIRECTORY [--stage ID ...] [--force]
spektrafilm-lab spectral evaluate --spectra FILE {--preset NAME ... | --profile FILE} [--set PATH=VALUE|FILE ...] [--data-dir DIRECTORY] --out DIRECTORY
spektrafilm-lab fit [FIT OPTIONS]
```

Each canonical leaf has one typed argument group and one semantic handler. The handler MUST NOT select a different contract from optional field presence or from the shape of a retained document.

- The former flat forms MAY remain only as deprecated compatibility aliases during one migration revision. An alias MUST map to one canonical handler before endpoint execution, MUST accept no new options, and MUST reject mixed endpoint options with `argument_syntax`.
- The alias layer MUST NOT contain document-shape dispatch. New documentation and examples MUST use the canonical grammar.
- A later compatibility revision MAY remove the aliases.

The command family names are semantic, not workflow steps. `measure run` publishes one retained measurement; `compare readings` reads only retained records; neither command invokes the next family. A shell pipeline MAY connect them by paths and result objects, but the instrument does not own that control flow.

- `KIND` is a [document registry](#document-registry) kind. `BOUNDARY` is `spectra`, `rgb_in`, `log_e_film`, or `cmy_film`.
- `STAGE` is `rgb_in`, `log_e_film`, `cmy_film`, `log_e_print`, `cmy_print`, or `rgb_out`. `CHAIN OPTIONS` is the parameter surface of `process`.
- Every option of the instrument and of its `fixtures` command MUST have a nonempty help description. Help, version, and schema discovery MUST work without a renderer, image decoder, input assets, or graphics device.
- Version MUST identify the instrument implementation and supported document versions. Instrument help MUST identify the `fixtures` command.
- `fixtures` help MUST name the command that consumes the generated bundle. The instrument MUST satisfy [Rust delivery](../benchmark/rust-delivery/spec.md).
- `schema` MUST emit the authoritative JSON Schema for one document kind. For the `metrics` vocabulary it MUST emit the metric-identity vocabulary instead of a JSON Schema.
- It MUST describe the required nested records and explain fields whose units, coordinate space, or applicability are not evident from their type. `validate` MUST validate a document and resolve the local dependencies needed for that kind.
- It MUST NOT render images. Standalone policy validation MUST NOT imply that selectors, measurement selection, or protocol binding have been checked against a suite.
- A valid target document MUST NOT imply that a GPU is available. `run suite` MUST resolve one suite and one target before rendering.
- Repeatable `--case` MUST select exact case identities in suite order. Duplicate selections and unknown identities MUST fail.
- Omitting `--case` MUST select all cases.

`run image` MUST resolve one local image, one configuration, and one target before rendering. It MUST NOT accept suite-only selectors. It MUST retain the generated local-image suite and case identity in the run record.

- `sample --from-run` reads exactly one completed case row selected by the pair `(case, seed)` from a retained image-domain `run` document. `RUN.JSON` MAY be the run document itself or its containing directory.
- The selected row MUST request and retain available stage evidence; the row's stage manifest reference and the run artifact index MUST agree on path and digest, and the retained path MUST remain under `cases/{case}/seed-{seed}/`. The stage bundle remains authoritative for stage layout, domain, units, color interpretation, transfer state, coordinate space, and buffer digest.
- Missing cases, failed rows, unrequested or unavailable capture, missing stages, malformed artifacts, traversal paths, and cross-seed artifact references are measurement failures; the command never falls back to another seed or stage.
- `run --dry-run` MUST use the same renderer configuration resolution, seed, environment isolation, selected profile containment, and case-owned geometry checks as execution. It MUST process all selected cases and seeds without decoding or rendering images.
- Identical seed failures MUST be grouped with their affected seed values and retained renderer logs. Unknown or duplicate case selections MUST identify the rejected identities and available choices.
- Renderer configuration errors MUST retain the actionable child error text and evidence paths.
- The dry-run destination MUST retain `preflight.json`, suite and target snapshots, resolved configurations, logs, and hashed evidence. A successful preflight MUST identify `mode: dry_run` and `execution_status: validated`; a failed preflight MUST identify `execution_status: failed` and structured errors.
- It MUST never publish `run.json` or completed render masters. Its `unchecked` list MUST identify image decoding, compute-device availability, observed output geometry, and numerical results.
- Preflight does not qualify a run or replace execution-time checks.
- Ordinary `run suite` and `run image` MUST NOT expose optional native-stage capture. Their simulation and source-only executions produce no native observation bundle, and preflight MUST NOT retain a capture request.
- Native stages, observed camera exposure, and prepared print normalization that are not independently retained remain explicitly unavailable; authored controls and ordinary diagnostics are not substitute numerical observations. No sampling, comparison, inspection, or reporting endpoint MAY rerender or reconstruct those absent values.
- Dedicated `experiment run` and `measure run` retain research observations through the shared core observer under the [native observation contract](native-observations.md), not through process flags. Existing supported run records with retained capture remain readable by `sample --from-run`; that reader does not restore a capture option.
- `run` MUST require exactly one of `--suite` and `--input`. Single-input options MUST conflict with `--suite`; `--case` MUST apply only to suite execution.
- `--configuration` MUST contain the existing case configuration object, with its file paths resolved relative to that file. `--input-kind` MUST default to `raster`; RAW MUST be selected explicitly and MUST use the renderer adapter.
- Raster input MUST require both source declaration flags. Repeated `--seed` MUST select distinct unsigned 32-bit seeds and default to `[0]`.
- `--regions` MUST contain the existing region array; omission MUST select no fixed regions. `--reference` and `--reference-info` MUST require each other.
- Reference info MUST contain the existing reference object except `path` and `sha256`, which the CLI MUST derive from the selected local file. Nested dependencies MUST resolve relative to the reference-info file.
- `configuration`, `regions`, and `preparation` schema kinds MUST expose their existing case subdocuments. The `reference` kind MUST expose reference info without `path` and `sha256`.
- Standalone validation MUST check their shape and available local dependencies and identify remaining cross-document applicability checks. It MUST NOT establish scene correspondence, successful rendering, or reference suitability without the required observations.
- The single-input form MUST construct and validate one `scene` case with identity `image` and suite identity `local-image`. It MUST use full-image families `numeric`, `color`, `tone`, and `flip`, and the versioned standard appearance protocol.
- The protocol uses 60 pixels per degree, scales `[1,0.4,0.16]`, flat-mean grain residual, high-pass radius 4, border 1, Hann spectrum, and six hotspots of minimum area 151. These settings MUST be retained in the generated suite, not hidden evaluator state.
- Case configuration MUST remain authoritative for crop, resize, and simulation parameters; target overrides MUST keep their existing applicability. The generated suite MUST be retained within the run without writing into the source repository.
- `--preparation` MUST contain `original` with `path` and `sha256`, and a nonempty `operations` array of descriptive strings. The original path MUST resolve relative to that file and enter retained dependencies.
- The instrument MUST record but MUST NOT execute these operations. Suite cases MAY contain the same optional `preparation` object.
- The actual prepared input hash MUST remain authoritative.
- `compare runs` MUST load two completed image-domain run records and their masters. It MUST NOT invoke a renderer.
- It MUST match cases by case identity and seeds by their explicit value, not directory order. It MUST reject incompatible suite or protocol identities.
- It MUST retain declared target differences. Reference objectives MUST use the reference associated with the case.
- `compare readings` MUST load two retained reading records and join them by `(sample_id, stage)`. Duplicate keys fail.
- Missing keys remain unmatched with an explicit reason. Shared keys with incompatible domain, units, channel order, or coordinate space fail as an identity mismatch.
- It MUST NOT read image-domain masters or invoke a renderer. An optional retained reference is input evidence, not a policy document.
- `compare pairs` MUST load one completed image-domain run and select only the explicitly requested `input`, `reference`, or `both` pairs. It MUST NOT compare two runs, infer a default pair, or apply a policy.
- A selected pair without its required reference MUST fail with an actionable error. The result MUST remain `not_evaluated`.
- `compare runs` MAY accept `--policy` and `--gate`. `--gate` MUST require `--policy`.
- It MUST make a non-pass evaluation produce a failing command status after the comparison has been published. Without `--gate`, a successfully published comparison MUST NOT fail solely because its evaluation is non-pass.
- The comparison source MUST derive its views from the comparison and cited artifacts. It MUST NOT recompute metrics.
- Its output MUST remain usable without the run directories by copying required review artifacts and retaining their provenance. An external original that cannot be redistributed MUST remain excluded from a distributable report; the report MUST identify the omission rather than substitute another image.
- `regions grid` MUST expand one finite grid declaration into a normal `regions` document without reading an image, invoking a renderer, computing measurements, or generating HTML. It MUST validate positive dimensions, nonnegative origin and gap, checked arithmetic, sample-window containment, coordinate-space selection, role, and measurement families before publication.
- The generated document is the only input geometry consumed by later `sample` and run endpoints; downstream endpoints MUST NOT interpret a grid declaration.

Irregular geometry MUST be authored directly as a `regions` document. The instrument MUST NOT replace a removed review manifest with repeated `--image`, `--pair`, `--default-pair`, or `--region` options.

Prepared-image visual review, if provided, belongs to the benchmark product's separate command and document contract. It is not an instrument endpoint, is not registered as an instrument document kind, and is not accepted by `report comparison` or `regions grid`.

- `inspect` MUST validate and read a retained comparison without invoking a renderer or computing measurements. It MUST emit the retained comparison identity, execution and evaluation states, conditions, summary, failures, findings, and compact measurements.
- The compact measurements MUST be one top-level `measurements` array in version and single-run comparisons. Rows in both modes MUST carry the same fields, except that a single-run row MUST add `pair_id` to identify its pair.
- Per-seed/pair observations and cross-seed `seed_measurements` MUST remain separate. Measurement identity, domain, units, estimator, sample count, availability, value or reason, seed population, and artifact references MUST remain intact.
- Exact repeatable case, region, and metric filters MUST reject unknown selections with available choices; they MUST NOT silently return an empty result. Referenced evidence MUST resolve relative to the emitted `artifact_base`.
- A valid read of a failed comparison MUST complete successfully while exposing its failed comparison state. Inspection MUST NOT modify evidence, rank unlike units, or infer acceptance.
- Inspection MUST expose each selected case and seed's retained `run_evidence` in `case_conditions`, with a `roles` object keyed by run role. These records MUST include source and target identities, declared preparation, effective conditions, and observed exposure and print normalization when collected.
- They MUST retain native-stage artifact links and availability without reading human logs or recomputing image statistics. Region-pair diagnostics MUST preserve their own identity and both endpoint identities.
- Inspection MUST also project retained `region_readouts` and `stage_readouts` arrays with case and seed identities. Case and region selectors MUST apply to these arrays; metric selectors MUST apply only to scalar measurements and findings.
- `readout_status` MUST distinguish retained arrays from `not_retained` fields in historical comparisons. Inspection MUST NOT backfill historical values.
- `target` MUST construct the existing target document with canonical renderer/data paths, the actual renderer SHA-256, an ID derived from the destination filename, the explicit backend, and empty changes. It MUST reuse target validation and refuse any existing destination, including a symlink.
- It MUST NOT invoke the renderer, probe a GPU, select simulation settings, or generate test inputs. Fixture targets MUST share this construction rule while retaining their own IDs, edits, and environment.
- Report location links MUST use `#case=ENCODED_ID&seed=UINT32[&pair=ENCODED_ID[&region=ENCODED_ID]]`, with percent-encoded exact identities. Version pairs use `baseline/candidate`; single-run pairs use their retained IDs.
- A location MUST select the matching case/seed/pair, open the requested region, and scroll to it after layout. Navigation MUST update the fragment and support reload and browser history offline.
- Detail disclosures MUST expose keyboard-operable, accessible controls. Malformed or unknown locations MUST show an accessible error without displaying an unrelated selected result.
- Every command result MUST identify `schema_version`, `command`, `execution_status`, and the terminal artifact when one exists. `schema --kind` emits one schema object without a result wrapper.
- An execution failure identifies `execution_status: failed` and structured errors. A requested gate failure after successful computation retains `execution_status: completed` and exposes its non-pass `evaluation_status`.

`schema --kind review` and `validate --kind review` MUST return a stable unsupported-kind error. A report command MUST accept only retained comparison evidence. The removed manifest source MUST NOT be represented as a compatibility alias or a second result kind.

- `sample` MUST validate coordinate-space compatibility before reading any region. Equal dimensions do not establish compatibility.
- A `source` region is compatible with a declared raster or `source_pixels`; a `render` region is compatible with `output_pixels`; a native stage region is compatible only with the same native stage name. The sampler MUST NOT resize, crop, pad, translate, or silently map a mismatch.
- `validate` MUST decide every condition that is decidable from the document, its declared dependencies, and the local filesystem. It MUST list in `unchecked` every check it did not perform, including checks that need another document, a renderer, a compute device, or a measurement.
- A check the validator did not perform MUST NOT be omitted from `unchecked`, and a condition it could have decided MUST NOT appear there. Passing `validate` MUST never be presented as proof that an execution will succeed.
- `run` and `compare` MUST report progress on stderr as line-oriented text. Every selected case and seed MUST produce a start line and a completion line.
- A completion line MUST identify the item and its elapsed seconds. The last progress output before the command result MUST state the completed and failed item counts and the total elapsed seconds.
- Progress MUST NOT require a terminal.
- A command result MUST identify `schema_version`, `command`, `execution_status`, and the result artifact when one is produced. An execution failure MUST identify `execution_status` as `failed` and contain structured errors.
- A published failed manifest MUST remain linked from the result; its detailed errors MUST NOT replace the command's structured error summary. Human error text MUST NOT be the only machine discriminator.
- A requested gate failure after successful computation MUST retain `execution_status` as `completed` and expose its non-pass `evaluation_status`.
- The executable MUST return status `2` for argument syntax errors. It MUST return status `1` for validation, rendering, measurement, publication, or requested gate failure.
- It MUST return status `0` when the requested operation completed without those failures. The distinction between execution and evaluation MUST remain visible in JSON even when both use status `1`.

A run with a failed selected case MUST have `execution_status` equal to `failed`. A comparison with an unavailable measurement caused by an unsupported reference scope MAY complete execution but MUST NOT pass a policy that requires that measurement. A numerical computation failure MUST remain an execution failure.

## Carrier and validation

All documents MUST use UTF-8 JSON and MUST carry `schema_version` equal to `1`, including the kinds that a suite embeds as subdocuments (`configuration`, `reference`, `regions`, `preparation`) and the retained records. Authored documents MUST reject duplicate keys and unknown fields. They MUST NOT support includes, environment interpolation, expressions, shell commands, or executable plugins.

- Schemas MUST describe document structure, types, ranges, and field applicability. The validator MUST also check duplicate identities, nonfinite numbers, unsafe artifact paths, and identifiers resolvable from the supplied document.
- Checks that require file contents MUST use the declared dependencies. Invalid authored probe point counts, coordinate ordering, mutually exclusive fields, and auxiliary reference document structure MUST fail validation before rendering.
- Runtime parameter and source-declaration semantics MUST use the renderer's shared definitions rather than a second parameter registry.
- Generated run and comparison readers MAY ignore unknown fields within a supported schema version. Required semantic fields MUST still be validated.
- Unsupported versions MUST fail rather than select defaults. Generated documents MUST preserve full unsigned seed and counter values.
- Authored file paths MUST resolve relative to the document that contains them. A document that names a chain carries parameter values, not a parameter language: it MUST use the `process` surface's names and precedence, and resolving it MUST be the resolution the user binaries perform.
- A parameter file a document names MUST resolve inside that document's directory tree; a path that escapes it MUST fail. Executable paths MUST be explicit paths, not commands resolved from `PATH`.
- Run and comparison artifact paths MUST be relative to their containing directory. Artifact paths MUST NOT contain an absolute root, parent traversal, or a symbolic link that escapes that directory.
- File dependencies outside the artifact directory MAY be consumed through authored paths but MUST be hashed and identified.
- The runner MUST retain bytes from the filesystem-resolved dependency path. It MUST preserve the absolute authored path separately for offline lookup, including parent components after symbolic links.
- Comparison MUST match that authored identity before looking up a lexically normalized identity from a historical run. Current-directory components MUST NOT distinguish equivalent authored paths.
- Neither lookup MAY resolve the original filesystem.

## Document registry

Every JSON document the instrument reads or writes has one kind and one owning section. Every document carries `schema_version`.

`schema --kind KIND` MUST emit the authoritative JSON Schema for every kind, including the nested records a consumer MUST read to diagnose a result: measurement rows, findings, summaries, failure records, and artifact references. `validate --kind KIND` MUST accept every authored kind and resolve the local dependencies that kind needs.

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
| `evidence` | retained | [Native observations](native-observations.md) |
| `metrics` | vocabulary | [Measurement protocol](../benchmark/metrics/spec.md) |

- A kind's state decides which commands accept it. An authored kind is written by a person or another tool and MAY be validated.
- A retained kind is published by the instrument, is immutable, and is consumed by `compare`, `inspect`, `verify`, `report`, and the matching `sample` adapters (`--from-evidence` or `--from-run`). `metrics` is not a document: it is the closed vocabulary of metric identities a policy MAY name, and the same vocabulary the measurement protocol owns.

## Measurement

A chain measurement executes the resolved chain on a declared stimulus set and records the boundary values it reads. It computes values and writes artifacts; it MUST NOT fit parameters, render images, judge results, or generate pages.

### Stimulus boundaries

A measurement enters the chain at one declared boundary: `spectra`, `rgb_in`, `log_e_film`, or `cmy_film`. They reuse the existing tap vocabulary.

The entry MUST use the canonical `taps.inject` control, and the executed range MUST end at or after it. The command MUST NOT add a second chain-entry mechanism or a second execution path.

- The measured range is declared through the same controls the user binaries read: `--enter` becomes the resolved `taps.inject`, and the collection boundary is the chain's own `taps.collect` when it reaches every requested reading, otherwise the last requested reading.
- A declared `taps.inject` that contradicts `--enter`, a `taps.collect` that stops before a requested reading, and a reading upstream of the entry MUST all fail before executing, each naming the conflict. The retained effective parameter document therefore describes the range that executed, and `verify` re-resolves the same range from the recorded declaration.
- A stimulus that enters at `rgb_in` declares its color space and its transfer state in the stimulus document, exactly as a raster input to `process` does.
- A stimulus that enters at `spectra` declares its color space, its transfer state, and its illuminant pair in the stimulus document; the command MUST NOT accept these declarations as command options, because the document is their single authority. A film-domain stimulus declares neither, because its values are already film-domain numbers.

The `spectra` entry uses the same front half as [spectral evaluation](spectral-evaluation/spec.md). `spectral evaluate` remains the roundtrip-error report against truth; `measure run` reports readings and MUST NOT compute a truth value.

### Point-wise requirement

Readings are defined only for a point-wise chain: no operation in the measured range mixes samples, measures the whole frame, resizes it, or draws stochastic values.

An operation lies inside the measured range when it executes and can change a requested reading. Both are decided by the stage the operation first changes, which the instrument MUST establish by measurement rather than by the flag's name:

| Operation | First stage it changes |
|---|---|
| `io.upscale_factor`, `io.crop` | `log_e_film` |
| `camera.auto_exposure`, `camera.diffusion_filter.active`, `camera.lens_blur_um` | `log_e_film` |
| `film_render.halation.active` | `log_e_film` |
| `film_render.dir_couplers.active` | `cmy_film` |
| `film_render.grain.active` | `cmy_film` |
| `enlarger.diffusion_filter.active` | `log_e_print` |
| `print_render.glare.active`, `scanner.lens_blur`, `scanner.unsharp_mask` | `rgb_out` |

- An operation executes when the range passes through it: the entry boundary is where the stimulus enters. An operation whose first changed stage is at or before the entry cannot reach a reading.
- An operation whose first changed stage is after the last requested reading has not changed one yet. An operation is listed when both hold.
- Which stages execute follows the executed path, not the route alone. The whole chain (the stimulus injected at `rgb_in` and collected at `rgb_out`) runs the route's own stage list.
- `input` returns after the crop and rescale. The convert routes build the film density by inverting the scan model and never run the metering or the development stages.
- The print stage needs a print route. `io.scan_film` scans the developed film directly instead.
- The scanner runs on every route but `input`. A narrower range walks the tap topology from the entry to the collection boundary instead.
- The nodes inside that range fire whatever the route says: only the metering stays skipped on the convert routes, and only `io.scan_film` still leaves the print taps out. That is why the route alone cannot decide eligibility.
- Measured on the 12-sample chart, the whole chain with `--read rgb_out` leaves the collected buffer byte-identical while toggling the following controls on `input > convert-film > scan` and `input`: `camera.auto_exposure`, `film_render.halation.active`, `film_render.dir_couplers.active`, `film_render.grain.active`, or `print_render.glare.active`. The collected buffer is also byte-identical while toggling the camera and development controls on `input > convert-film > print > scan`.
- `input` with `--enter rgb_in --read log_e_film` moves every `log_e_film` reading by the metered stop (0.1618 in log10) when `camera.auto_exposure` is disabled, because that range runs the metering node.
- An operation the executed path skips is not listed. Neither is a controller whose active condition the core does not read (`print_render.glare.percent` at zero) nor one with no reader at all (`enlarger.lens_blur` is applied by no stage, see the parity matrix).
- The measured table is evidence, not a style choice: on a 12x4 chart, halation moves `log_e_film` by 1.9e-02 and the DIR couplers move `cmy_film` by 8.5e-01 while leaving `log_e_film` exact.

The command MUST resolve the chain before executing it. When one of these operations is active in the measured range, the command MUST fail before executing, and it MUST name the `--set` edits that disable it. The command MUST NOT disable an operation on the caller's behalf.
### Unix-style endpoint acceptance

The following scenarios are product acceptance, not help-text examples:

1. A retained stage bundle can be passed to `sample`, and its declared stage domain, units, channel order, transfer state, buffer digest, and coordinate space are read from the bundle rather than repeated on the command line.
2. A `regions` document can be reused unchanged to sample two retained runs and an explicitly declared image source. The three commands produce records with stable sample identities and no inferred geometry.
3. `compare` reads the two retained reading records and writes per-sample deltas and group summaries without invoking a renderer. A shared key with different units or channel order fails; a missing key is reported as unmatched; an unavailable metric carries a reason and never serializes as zero.
4. Capturing stdout separately from stderr yields one parseable result object on stdout while progress and human diagnostics remain on stderr. Redirecting stderr or piping stdout through a JSON consumer does not change command behavior.
5. A command cannot make a downstream decision: `run` does not sample or compare, `sample` does not score, `compare` does not select a candidate, and `report` does not recalculate a metric.
6. A declaration containing an include, environment substitution, expression, shell command, wildcard selector, or optimizer instruction is rejected as an unknown field or unsupported syntax before execution.
7. An experiment declaring seeds `[0, 42]` either publishes two independently identified retained results or fails before rendering with a stable validation error; it MUST NOT publish one result while recording both seeds as observed.
8. The future explicit forms `compare runs` and `compare readings` select their handlers from the command grammar, not from the presence or absence of fields in `run.json`.

The acceptance fixture MUST use one ordinary 45-region declaration and one non-grid explicit regions declaration. Passing the fixture MUST prove that the endpoint contract is generic and does not encode a TE226-specific geometry or a universal appearance score.

### Stimulus document

A stimulus document declares the values a chain measurement injects. It MUST contain `schema_version`, an `id`, a `domain`, and the sample arrays beside it as `.npy`/`.npz` files.

`cctf_decoding` MUST name the state the chain reads the stored values in, matching `io.input_cctf_decoding`: `true` when the stored values are transfer-encoded and the chain MUST decode them, and `false` when they are already transfer-decoded.

- `domain` MUST match the `--enter` boundary and be one of `reflectance`, `scene_rgb`, `film_log_e`, or `film_density`. A `scene_rgb` document MUST declare `color_space` and `cctf_decoding`, exactly as a raster input to `process` declares them.
- A `reflectance` document MUST declare `color_space`, `cctf_decoding`, and its illuminant pair as `condition: {"reference": ..., "target": ...}`. A film-domain document MUST declare none of them, because its values are already film-domain numbers.

The document MUST declare the wavelength grid or the band it covers, the per-sample `source` and `partition` labels, and the provenance hashes of the arrays it names. A sample outside the declared band is invalid. The command MUST NOT clip or extrapolate it.

### `measure run`
```text
spektrafilm-lab measure run --stimuli PATH --enter BOUNDARY [--read STAGE ...] [--dry-run] [CHAIN OPTIONS] --out DIRECTORY
```

- `--stimuli` declares a [stimulus document](#stimulus-document).
- CHAIN OPTIONS are the parameter surface of `process`: `--preset`, `--film`, `--paper`, `--set`, `--route`, `--scan-output`, and `--data-dir`. The command MUST resolve them with the shared resolver.
- It MUST NOT accept a stored parameter document as the chain declaration.
- `--read` selects stages from the existing tap vocabulary. Omitting `--read` MUST read every available stage.
- The command MUST execute the core-owned pipeline path of the resolved chain with the common core observer. The instrument owns requested readings, observer configuration, and publication under the [native observation contract](native-observations.md).
- It MUST NOT request process capture, walk stages beside an executed run, or reconstruct pipeline mathematics. The record MUST retain the owning replayable measurement invocation and observer configuration, not fabricated process argv.
- The command MUST NOT write image output.
- `--dry-run` MUST resolve the chain, evaluate the point-wise requirement, and write the identity record only. It MUST NOT execute the chain, write `readings.csv`, or publish a reading.

### `sample`
```text
spektrafilm-lab sample --source IMAGE --input-color-space NAME --input-cctf-decoding true|false --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-evidence DIRECTORY --stage STAGE --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-run DIRECTORY|RUN.JSON --case ID --seed UINT32 --stage STAGE --regions FILE --out DIRECTORY
```

- `--source` declares a raster image. It MUST require the same input declaration as `process`: a source color space and a transfer state.
- `--from-evidence` reads one stage of a retained evidence bundle. The coordinate space, channel order, and units of that stage MUST come from the bundle manifest.
- `--from-run` resolves exactly one completed `(case, seed)` row from a retained image-domain run, then reads its requested available stage from the indexed `stage-evidence.json`. The run record and stage bundle are authoritative; missing or failed rows, unrequested or unavailable capture, missing stages, malformed artifacts, unsafe paths, digest mismatches, and cross-seed references MUST fail without fallback.
- `--regions` declares reading positions with stable sample identities: explicit windows with their labels and groups, in the coordinate space the document declares. A window that leaves the source bounds MUST fail with the sample identity.
- The command MUST NOT clamp, pad, extrapolate, or repair a sample.
- The command MUST NOT resize, re-encode, or convert the source beyond the declared input declaration.
- A reading describes the values it stores: applying the declared transfer decoding makes the reading linear, and its record MUST say so while the source block keeps the source's own declaration. A stage that declares no transfer state records none.
- Every record carries the common fields, including `executed`: a sample never executes a chain.

### `verify`
```text
spektrafilm-lab verify --record DIRECTORY [--data-dir DIRECTORY]
```

- The command MUST read the record's kind and re-hash what that record names. It MUST NOT render, rewrite, or judge the record.
- For an image-domain run it MUST re-resolve the retained suite, target, and case configuration, and re-hash the renderer executable, the selected data inventory, the profiles, and the authored dependencies.
- For a chain run it MUST re-resolve the recorded declaration, and re-hash the renderer executable, the selected data inventory, and the profiles.
- For a comparison it MUST re-hash the retained run and comparison identities and the evidence the comparison cites.
- It MUST report every difference from the recorded identity.
- It MUST re-hash every stage buffer the record retained, whether or not the reading requested that stage.
- It writes no artifact; it prints its report, so a verification cannot overwrite the record it verifies.

### Guard runs

- A guard run is a retained measurement of a frozen stimulus set. It uses the same commands and artifacts as any other measurement.
- The instrument MUST NOT add a guard command, flag, or tolerance field. A comparison against retained readings is `compare`.

## Regions document

A regions document declares reading positions. One kind serves both reading domains: an image-domain case, a sampling command, and a reference declare regions in it.

- A regions document MUST contain `schema_version`, an `id`, and a nonempty `regions` array. A region MUST contain `id`, `geometry`, `role`, and `measurements`, and MAY contain `label`, `source`, and `partition`.
- `role` MUST identify `skin`, `neutral`, `shadow`, `highlight`, `color`, `flat_field`, `edge`, `texture`, or `other`. `measurements` MUST select applicable measurement families.
- `source` and `partition` are group labels for reporting. Identities MUST be unique within the document, MUST match `^[A-Za-z0-9][A-Za-z0-9_.-]*$`, and `full_image` stays reserved for the whole declared source.
- `geometry` MUST be either a half-open rectangle with the named fields `x`, `y`, `width`, `height`, or a lossless mask with its `path`, `sha256`, and declared `width` and `height`. Widths and heights MUST be positive integers.
- Every geometry MUST declare its coordinate space. `source` names the declared source image's pixels; `render` names a completed simulation's output pixels; native stage names are `source_pixels`, `working_pixels`, `film_frame_pixels`, `grain_aperture_pixels`, or `output_pixels`.
- Mask paths MUST resolve relative to the document that contains them and enter the retained dependency inventory as immutable evidence.
- Regions MAY overlap: a reading carries its own identity, and a narrower window inside a wider one is a declared intent, not a conflict. Correspondence is never inferred from equal dimensions, labels, or shared region identities.
- `regions grid` is the only instrument geometry generator. It writes a normal `regions` document and does not read images or produce a review page.
- Non-grid geometry is authored directly. A suite case, a sampling command, and a reference MAY consume the same regions shape through one validator.
- Benchmark image-domain consumers currently accept only `source` and `render`. Native stage values require a producer mapping and are rejected there with `unsupported_coordinate_space` until one exists.

## Suite document

- A suite MUST contain `schema_version`, `id`, `protocol`, and a nonempty `cases` array. `id` MUST be a stable nonempty string.
- `protocol` MUST select `appearance-v1` and contain its declared `pixels_per_degree`, region sampling scales, grain residual and spectrum settings, and hotspot selection settings. Every protocol value that changes a measurement MUST contribute to the resolved protocol digest.
- A case MUST contain `id`, `kind`, `input`, `configuration`, `seeds`, `regions`, and `measurements`. `kind` MUST be `probe` or `scene`.
- Case identities MUST be unique. `seeds` MUST be a nonempty array of distinct integers in the renderer's seed domain.
- Each selected case MUST be rendered for each selected seed.
- `input` MUST contain `path` and `sha256`. Raster input MUST also contain `color_space` and `cctf_decoding`.
- RAW input MUST use the adapter declaration and MAY contain the dedicated RAW options specified below. The input hash MUST match before decoding.
- File extension MUST NOT infer a color declaration.
- `configuration` MUST use the following fields: `film`, `paper`, `presets`, `params`, `set_files`, `set`, `route`, `scan_output`, and `raw`. Fields not required by the renderer MAY be omitted.
- `presets` MUST contain explicit preset-file paths, not mutable user-library names. `params` MUST be a legacy parameter snapshot path.
- `set_files` MUST contain sparse parameter-file paths. Every `set` entry in case configuration, target changes, or case changes MUST contain renderer-compatible `PATH=VALUE` assignments, including comma-separated assignments with quoted strings and arrays; file carriers MUST occur only in `set_files`.
- File edits MUST apply in listed order before listed inline edits. Their semantic validation and precedence MUST follow the renderer's resolver.
- No instrument field MAY replace a runtime field alias.

`raw` MAY contain `white_balance`, `temperature`, `tint`, and `lens_correction` with the dedicated renderer option semantics. These fields MUST apply only to RAW cases. An incompatible combination MUST fail before rendering.

- A case's `regions` element MUST use the region shape the [regions document](#regions-document) defines, in render coordinates. Region identities MUST be unique within a case.
- Bounds MUST be validated against the actual master. Fixed regions MUST NOT be silently resized to fit a changed render.
- Tone-response probe geometries MAY be declared in `protocol.probe.tone_response` or overridden by a case's `probe.tone_response`. Both locations MUST use the same rectangle or hashed-mask validation.
- Mask paths MUST resolve relative to the suite document, enter the run's dependency inventory, and be retained as immutable evidence. Comparison MUST resolve the effective probe geometry only from retained dependencies, without requiring original mask files or the invocation working directory.
- The case's `measurements` MUST select full-image measurement families from `numeric`, `color`, `tone`, `flip`, `grain`, `halation`, and `edge`. Region `measurements` MUST select applicable families in that region.
- The protocol MUST resolve each family to explicit scalar identities, maps, curves, or spectra. A family selector MUST NOT request a hidden aggregate quality score.
- A case MAY contain `region_pairs`, an array of objects with `id`, `label`, `left`, and `right`. Pair identities MUST be unique and MUST NOT collide with region identities.
- Both endpoints MUST name distinct existing regions in that case. A pair MUST NOT infer correspondence across cases or image sets.
- The protocol MAY contain `color` with a finite positive `hue_chroma_floor`. This declaration selects regional hue diagnostics and MUST contribute to protocol identity.
- Omitting it MUST NOT select an implicit hue threshold. The measurement protocol owns the diagnostic definitions and availability rules.
- A case MAY contain `reference`, `split`, and `split_group`. `split` MUST be `calibration` or `evaluation`.
- A case with fitted reference objectives MUST provide `split_group`. Related captures MUST use the same group.
- A reference MUST contain `id`, `path`, `sha256`, `scope`, `color`, `provenance`, and `processing`. `scope` MUST be `material`, `aligned_scene`, `regional`, or `visual`.
- `color` MUST select an explicit declaration or an embedded ICC profile with its hash. Unknown color interpretation MUST make color measurements unavailable.

`provenance` MUST identify source, attribution, redistribution permission, capture conditions, and development and digitization conditions. Unknown conditions MUST be represented explicitly, not fabricated. `processing` MUST identify known grading, histogram matching, compression, sharpening, noise reduction, and resampling, including unknown entries.

- An `aligned_scene` reference MUST provide a frozen transform or already aligned derivative, its correspondence-review record, and a common-area mask. A `regional` reference MUST provide corresponding reference-region geometries.
- A `material` reference MUST provide the measurement domain, instrument, units, and conditions required by its selected metrics. A `visual` reference MUST NOT select full-reference scalar measurements.

A `visual` reference MAY provide explicit reference-region geometries for crop inspection. The comparison MUST retain these geometries without calculating reference distances. A report MUST NOT borrow input-to-simulation alignment to locate a region in an unaligned reference.

Reference transforms, masks, review records, and derivatives MUST be hashed dependencies. Their geometry and purpose MUST be explicit. The evaluator MUST NOT fit a new transform during `compare`.
## Prepared-image review boundary

The instrument does not define or accept a prepared-image review manifest. `review` is not a document-registry kind, and `schema --kind review` and `validate --kind review` MUST return a stable unsupported-kind error. The instrument's report source is a retained comparison only.

The benchmark product MAY retain a separate prepared-image viewer for local visual inspection. Its image inventory, per-image geometry, default pair, captions, display conversion, and report builder belong to the benchmark command and its own contract. The instrument MUST NOT import that manifest, derive regions from it, shape-dispatch on it, or expose aliases for it.


## Target document

- A target MUST contain `schema_version`, `id`, `renderer`, `data_dir`, `backend`, and `changes`. `renderer` MUST contain an executable path and its SHA-256.
- `backend` MUST be `cpu` or `gpu`. `data_dir` MUST be explicit.
- Renderer identity validation MUST precede execution.
- `changes` MUST contain arrays `set_files` and `set`, which MAY be empty. They MUST apply after each case's configuration using the existing sparse-file and inline-assignment semantics.
- They MUST NOT edit seed, input identity, or geometry outside the permitted renderer fields. The comparison MUST expose the resolved differences rather than trusting a change description.
- A target MAY contain `case_changes`, keyed by exact case identity. Each value MUST use the same `set_files` and `set` fields and MUST apply after target-wide changes.
- Unknown case identities MUST fail. This surface MUST NOT support conditions or wildcard case selection.
- Explicit renderer and data identities MUST support executable or profile/data revisions without parameter edits. The run MUST hash the complete selected data directory with a deterministic relative-path inventory.
- The resolved inventory MUST identify file bytes, not modification times. Before any rendering, the runner MUST check the actual film and paper profile names selected by the renderer's dry run, including preset-provided references, and reject profile paths that resolve outside the target's data directory.
- Target changes MUST NOT silently substitute a profile file from another directory. A data directory that cannot be inventoried, including one containing symlinks, MUST NOT abort the run without a manifest.
- The run MUST still publish `run.json`. It MUST fail every case that did not already fail for a more specific reason such as profile confinement, and MUST remain ineligible as a qualified baseline.
- The target MUST NOT own output container settings, input geometry, or seed selection. The runner MUST control artifact paths, request floating TIFF masters, preserve scanner output color and transfer state, and request execution diagnostics.
- Source declarations MUST remain case-owned. Before image rendering, the runner MUST compare target-resolved input geometry and `settings.preview_mode` / `settings.preview_max_size` against the case-only dry resolution.
- Target-wide and case-specific changes, whether inline assignments or sparse files, MUST fail if these case-owned resolved fields differ.
- The runner MUST isolate user preset configuration and remove inherited simulation-control environment values before rendering. An allowlist of environment variables that influence loading, numerical execution, or device selection MUST be recorded with the instrument implementation.
- Allowed values MUST contribute to run identity. Experimental environment controls MUST require an explicit target extension in a new interface version; unrecorded experimental values MUST NOT affect a qualified run.

## Policy document

- A policy MUST contain `schema_version`, `id`, `protocol_digest`, `calibration`, and `rules`. It MUST bind to the resolved measurement protocol.
- `calibration` MUST identify hashed evidence, its case population, reviewed labels, observation scale, and scope. An uncalibrated exploratory policy MUST identify that state and MUST NOT produce `passed` qualification.
- The resolved protocol digest MUST include case source declarations and hashes, selected measurement families, probe definitions, region geometry and mask hashes, reference dependencies, and seed populations. It MUST also include the instrument implementation revision, metric and color-transform versions, relevant numerical dependency versions, and the LittleCMS version and transform settings for ICC references.
- `validate --kind suite` MUST expose this digest for policy authors. Cross-implementation evidence and policy handling MUST follow [Rust delivery](../benchmark/rust-delivery/spec.md#existing-evidence-and-protocol-identity).
- Each rule MUST contain `id`, `purpose`, `selector`, `metric`, and its constraints. `purpose` MUST be `protect`, `expect`, or `reference`.
- `selector` MUST identify exact case and region identities or the explicit full-image region. Wildcards, expressions, and implicit aggregate selectors MUST NOT be accepted.

Before computing measurements, `compare` MUST reject a rule whose metric identity is unknown or cannot be produced by the selected case and region measurement families. A supported selected metric that lacks usable evidence MUST remain unavailable under the existing evaluation rules. The implementation MUST reuse measurement definitions for this check rather than maintain a second policy metric registry.

- A protected rule MUST contain an inclusive finite `min` or `max`, or both, in the metric's units. A reference rule MUST constrain candidate-to-reference distance or candidate-minus-baseline reference distance with an explicitly named metric.
- An expected-change rule MUST state the direction or interval expected for one signed metric. Expected changes MUST NOT mask protected failures.
- Every rule MUST declare `required`. An unavailable required rule MUST prevent a pass.
- An optional unavailable rule MUST remain visible and MUST NOT count as satisfied. Evaluation MUST NOT infer thresholds from a candidate's measurements.

A human approval MAY be attached to a comparison as a separate review artifact with reviewer, comparison digest, decision, and rationale. It MUST NOT modify the numerical decision or claim a new measured value.

## Run record

A run directory MUST contain `run.json`, the suite and target snapshots, resolved dependency inventories, per-case resolved configuration, per-seed floating masters, diagnostics, and captured renderer output. The record MUST distinguish source documents from resolved configuration.

`run.json` MUST contain `schema_version`, `id`, `execution_status`, `suite`, `target`, `protocol`, `selection`, `dependencies`, `cases`, and `artifacts`. Document and dependency identities MUST use SHA-256. `selection` MUST state whether it covers the full suite.

Each case result MUST identify case identity, seed, terminal status, input and output dimensions, configuration artifact, master artifact, diagnostics artifact, and errors when present. Actual backend, execution path, host precision, GPU precision, and adapter facts MUST come from observed diagnostics. Missing observations MUST remain unavailable.

Each completed case MUST retain the full executed simulation command, including numerical-output and diagnostics arguments, with no optional native-capture argument. The source-only command MUST remain separate. Required source evidence MUST remain required even when both source-master fields are absent; a valid manifest digest does not establish semantic completeness.

- Completed runs MUST retain a floating `source_raw` with `source_raw_metadata` from the real renderer's input route, including source color interpretation and observed dimensions. The input route MUST use the same RAW decoding options and case-owned geometry as simulation, force output to linear ACES2065-1, disable the independent CMYK ink style, and force `taps.inject=rgb_in` and `taps.collect=rgb_out`.
- Source configuration and observed diagnostics MUST remain retained dependencies of an input comparison. Original source dimensions and preparation metadata MUST remain separately identified.
- Input output geometry MUST be checked before declaring pixelwise correspondence with simulation.

Completed case artifact references MUST be objects with an indexed `path`. A malformed required reference MUST return a structured command failure.

- The artifact index MUST identify relative path, content hash, media type, role, and dimensions or sample layout when applicable. All retained paths MUST resolve.
- Partial work MUST NOT be indexed as a complete master. A failed run MUST retain successfully completed cases and explicit failures, but MUST NOT be reused as a qualified baseline.
- New ordinary suite/image rows MUST explicitly identify native observation as absent (`requested: false`, `availability: "unavailable"`, `reason: "not_requested"`) with no manifest or buffer references. Their exposure and prepared print-normalization observations MUST likewise remain unavailable when not retained; neither authored controls nor a synthetic rerender MAY fill them.
- Historical supported rows that retained a native bundle keep their case `stage_evidence` reference and artifact index. Readers MUST validate the producer schema version, terminal state, numerical layout, byte length, finite samples, hashes, stage identities, geometry, and availability before use.
- The [native observation contract](native-observations.md) owns buffer and preparation semantics, not the process CLI or a resumable stage artifact.

### Chain runs

A chain run writes the same `run` kind. It MUST carry `kind` set to `run` or `sample`, so `verify` can read a record without being told what it is.

- `declaration`: the owning replayable `measure run` invocation, chain options, entry boundary, requested read stages, observer configuration, and declared stimulus document with hashes — everything needed to resolve and observe the same chain again. Observer configuration includes selected taps, backend, host precision, bundle destination, and native schema version.
- It MUST NOT contain fabricated process capture argv.
- `stimulus`: the stimulus manifest path and the array hashes. A sampled source records its own `source` block instead, because no chain was resolved to read it.
- `identity` for an executed run includes the renderer's path, content hash, and version, the selected data inventory digest, and the profile identities. It also includes the declared parameter sources and the content hash of every parameter file they name, the stock-specific application, the effective parameter document, the execution precision, and the seed.
- `verify` diffs that document leaf by leaf, so a re-verification names the parameter that moved instead of only its digest.
- `stages`: one record per read stage with its identity, domain, units, channel order, the declared color space and transfer state, the buffer hash, and its availability together with a stable reason when the stage did not run.
- `groups` and `summaries`: the group identities and their per-channel statistics.
- `executor`: the backend and the precision of an executed run.
- `evidence/`: the retained stage buffers of the executed run under the instrument-owned [native observation contract](native-observations.md), not a `.sfstage` resume state.
- `readings.csv`: one row per sample and stage — `sample_id`, `source`, `partition`, `stage`, `domain`, `units`, `channel_order`, the channel values, and `valid`. A value that is not finite is an empty field with `valid` false; a placeholder number MUST never be written in its place.

A declaration MUST be replayable from any working directory: a declared parameter or preset file is recorded with its canonical path and content hash. Identity fields reuse the vocabulary the renderer and the image-domain records already use (`renderer`, `data_dir`, the backend) rather than inventing a competing name.

Where a record restates a shared convention — the interpolated percentile estimator, the bounded-display color distance — the specification of the shared owner is authoritative and the record names it.

## Comparison record

A comparison directory MUST contain `comparison.json`, input run identities, the policy snapshot when selected, scalar measurements, map and curve artifacts, and selected detail-region records. Every finding MUST resolve to retained artifacts or verified source run artifacts.

- `comparison.json` MUST contain `schema_version`, `kind` equal to `comparison`, `id`, `execution_status`, `evaluation_status`, `protocol`, `selection`, `condition_differences`, `cases`, `findings`, `summary`, and `artifacts`. Version comparisons MUST contain `baseline` and `candidate`, and MUST contain `policy` when selected.
- Single-run comparisons MUST contain `mode` equal to `single_run`, `run`, and `default_pair` equal to `input` or `reference`. Each case MUST retain a `pairs` array with unique `id`, `left_role`, `right_role`, `pixel_aligned`, `status`, pair-owned measurements and review artifacts, and reference applicability when selected.
- Input pairs MUST use roles `input` and `simulation`; reference pairs MUST use `reference` and `simulation`. Unavailable image evidence MUST remain labeled and MUST NOT fall back to another role.
- Pair ids MUST also identify scalar measurements and findings in single-run results.
- Each completed case MUST retain `run_evidence` keyed by `baseline` and `candidate` for a version comparison, or `simulation` for a single-run comparison. It MUST preserve run, target, input, data and configuration identities, declared preparation, actual command and observed stage context.
- Referenced native bundles MUST be copied with their relative buffer dependencies into the comparison artifact index. Report and inspection consumers MUST NOT require the original run directory to recover this evidence.
- New comparisons MUST retain `region_readouts` on version case records or single-run pair records, and `stage_readouts` once per case. The [comparison schema](../../crates/spektrafilm-benchmark/assets/comparison.schema.json) owns their field structure.
- The [measurement protocol](../benchmark/metrics/spec.md#retained-region-coordinates) owns estimators and eligibility. Historical version-1 records MAY omit these arrays.
- Consumers MUST NOT interpret omission as an empty measured population or a zero result. Available observations MUST contain finite values; unavailable observations MUST contain a reason and no value.
- Color readouts MUST identify `pair_id`; native-stage readouts MUST identify `run_role`. Artifact references MUST resolve inside the retained comparison.
- `evaluation_status` MUST be `not_evaluated`, `passed`, `review_required`, `failed`, or `insufficient_evidence`. No policy MUST select `not_evaluated`.
- A protected or expected-change rule violation MUST select `failed`. A missing required rule measurement or an uncalibrated policy MUST select `insufficient_evidence` unless an observed violation already selects `failed`.
- An unresolved compatibility difference or required human decision MUST select `review_required` unless a stronger failure applies. `passed` MUST require all required rules and qualification conditions to be satisfied.
- Each scalar measurement MUST identify case, seed population, region, metric identity, domain, units, estimator, sample count, availability, and artifact references. Available measurements MUST contain `value`.
- Other states MUST contain `reason` and no value. Curves, spectra, and maps MUST retain their sample coordinates, units, dimensions, and hashes.

Each finding MUST contain `id`, `case_id`, `region_id`, `metric`, `decision`, `evidence`, and the relevant rule identity when policy-derived. Automatic detail records MUST retain their rectangle, ranking metric, ranking value, and merge/selection method. A prose explanation MAY supplement but MUST NOT replace these fields.

The summary MUST retain selected and completed case counts, required and unavailable rule counts, protected failures, and worst-case findings. It MUST NOT contain a universal appearance score.

### Reading comparisons

- A comparison of two retained readings matches rows by composite sample and stage identity. Two sides that disagree on the domain, unit, or channel order of a shared key are an incompatible identity set: the command MUST fail and name the mismatch instead of truncating the join.
- Ambiguous identities, a repeated key inside one side, MUST fail as well. Keys that only one side carries are reported as unmatched with the side that carries them.

Channel deltas are published only for pairs whose color space and transfer state agree, including two absent declarations. A pair whose declarations differ reports no channel deltas and an unavailable reason. The color-distance metric converts through linear sRGB into the bounded-display Lab domain, states that domain in its records, and marks a reading it had to bound.

Availability is decided per pair: the baseline/candidate, candidate/reference, and baseline/reference pairs each decide from their own two sides and their own metric. A decision MUST never be borrowed from another pair.

Each comparison carries its own name, so a record cannot be read as another comparison: the primary comparison is `candidate_minus_baseline`, and a reference adds `candidate_minus_reference` and `baseline_minus_reference`.

The group labels of a reading (`source`, `partition`) are provenance, not identity: two sides that label one shared key differently MUST report the label difference as an identity difference and still compare the values. Only the value's own declaration (domain, units, channel order) makes a shared key incomparable.

An identity difference between the two runs is evidence, not an error. The command MUST report the difference together with the numeric deltas. It MUST NOT refuse a comparison because the two chains differ; detecting that difference is the purpose of a retained reference reading.

- `--reference-run` adds a third retained reading. Its similarities and differences MUST be reported under its own role, not relabelled as the baseline or the candidate.
- A reference that shares no sample and stage with the pair, or whose readings are not eligible for the metric, MUST report an unavailable distance with a stable reason, and the comparison MUST still publish its other measurements. An unavailable measurement MUST never serialize as a distance, as zero, or as an absent row without a reason.

The command MUST compute per-sample deltas and group summaries through the shared metric implementation. It MUST NOT add a second color-difference, equal-lightness, or region-aggregation algorithm. It MUST read the artifacts the measurement wrote, including their quoting: a sample identity with a comma, a quote, or a line break MUST survive the round trip.

- A reading comparison writes `deltas.csv` with `pair`, `sample_id`, `source`, `partition`, `stage`, `domain`, `units`, the minuend side's channels, the subtrahend side's channels, the channel deltas, the color distance, `valid`, and `clipped`. One row per joined sample and stage per comparison, so reference comparisons keep their per-sample rows.
- `d` is the minuend side minus the subtrahend side of that comparison. The distance column stays empty when the stage declares no color space; `clipped` counts the channels the distance had to bound.
- A pair whose two sides declare different color interpretations writes no channel deltas, with empty delta fields and `valid` false, so a file-level aggregation cannot mix a pair the record itself calls incomparable.

## Publication and examples

- `run`, `compare`, and `report` MUST require a destination that does not exist. They MUST NOT replace completed evidence.
- They MUST publish a terminal manifest only after referenced local artifacts have been finalized and hashed. A handled operation failure MAY publish a failed terminal manifest and valid partial evidence.
- Forced termination before the terminal manifest MUST leave an incomplete directory that cannot be treated as a completed run.
- A gate failure MUST preserve the completed comparison. A report-generation failure MUST NOT modify its source comparison.
- Publication failures MUST NOT produce a success result. This contract does not promise a transaction across different destination directories or power-loss durability.
- The implementation MUST ship schema-validated examples for every authored document kind. Command examples MUST execute with packaged fixture assets and the real renderer.
- Documented instrument usage examples MUST be checked against the implemented command and flag surface by a permanent test. A documented command or flag the executable does not accept MUST fail that test.
- The schema, parser, examples, and validator MUST change together. The interface MUST NOT claim executable examples or a validated schema before these artifacts are exercised.
- An executable example MUST use fixture generation, `validate`, `run`, `compare`, and `report` to complete a comparison without source-code inspection. The example MUST read metric identities, units, availability, and evidence links from the resulting measurements before authoring a policy.
- It MUST identify the exploratory policy's qualification limit. This workflow MUST use the existing commands and document fields; it MUST NOT require a new discovery language.

## Standard configuration generation

The `fixtures` command MUST accept this grammar without a new selector or configuration language:
```text
spektrafilm-lab fixtures --renderer FILE --data-dir DIRECTORY --out DIRECTORY [--natural-input FILE | --download-natural]
```

`--natural-input` and `--download-natural` MUST be mutually exclusive. The generator MUST preserve the existing pinned natural-source validation and attribution rules.

- Every invocation MUST generate the existing probe assets, suites, targets, exploratory policy, and provenance. It MUST also generate `look.toml`, `color-tone.json`, and `appearance.json`.
- `look.toml` MUST use the renderer's portable preset schema. The two JSON files MUST contain the existing standalone `configuration` object, not a new document envelope.
- The [standard configuration product contract](../benchmark/standard-configurations/spec.md) defines their rendering purposes and suite coverage.
- Both configurations MUST reference `look.toml` through `presets: ["look.toml"]`. They MUST use existing `route`, `set_files`, and `set` carriers.
- Their paths MUST resolve relative to the containing configuration file. Generated suite cases MUST carry the equivalent configuration objects because the suite schema has no configuration include field.

Both configuration objects MUST set `route` to `input > film > print > scan` and `set_files` to an empty array. Their ordered `set` arrays MUST begin with these shared assignments:
```json
[
  "camera.auto_exposure=false",
  "camera.exposure_compensation_ev=0",
  "camera.film_format_mm=35",
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

- `color-tone.json` MUST append `debug.deactivate_spatial_effects=true`, `debug.deactivate_stochastic_effects=true`, and `film_render.grain.active=false`. `appearance.json` MUST append `debug.deactivate_spatial_effects=false` and `debug.deactivate_stochastic_effects=false`.
- Neither object MAY add `film` or `paper` selections that compete with the exported look. The generator MUST use the renderer's resolution to validate the effective settings; standalone shape validation is not proof of supported renderer parameters.
- The generator MUST obtain the look through the selected renderer's public `preset show built-in:Kodak Gold 200 --format toml --data-dir DIRECTORY` command. It MUST capture child output and diagnostics separately.
- It MUST NOT copy a preset from the checkout or resolve a bare user-library name. Preset export or validation failure MUST fail generation without selecting a substitute look.
- Profile references MUST permit intentional runtime profile revisions rather than require baseline content hashes.

The generator MUST validate both configuration documents and all generated suites and targets before publishing success. `provenance.json` MUST record the standard configuration revision `standard-print-scan-v1`, selected built-in preset selector, renderer SHA-256, and relative paths and SHA-256 values for the three new files:
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

Existing natural-source provenance MUST remain intact. A change to authored standard settings or appearance-case coverage MUST update this revision; runtime dependency hashes remain authoritative for file identity.

- The successful stdout result MUST include `schema_version`, `command: fixtures`, `execution_status: completed`, the output directory as `artifact`, the existing `suites` list, `preset: look.toml`, and `configurations` with `color_tone: color-tone.json` and `appearance: appearance.json`. Returned file paths MUST be relative to `artifact`.
- Failure MUST use the existing structured error and exit-status contract. A partial directory without a successful final provenance record MUST NOT count as a generated bundle.
- The generator MUST refuse any existing destination. It MUST NOT invoke rendering or device discovery.
- It MUST NOT create runs, comparisons, or calibrated policy evidence. Generated baseline and candidate target files remain examples, not completed baseline or candidate runs.
- The packaged usage example MUST generate once, use `color-tone.json` and `appearance.json` with a declared local image, and select seeds `0`, `42`, and `99` explicitly for full appearance. It MUST show how to construct a second target and reuse the same bundle for a version comparison.
- It MUST use the existing `run`, `compare`, `inspect`, and `report` commands. These examples MUST be executed during delivery acceptance rather than presented as already supported.

## Acceptance

- `schema --kind` MUST cover every kind in the [document registry](#document-registry), and `validate --kind` MUST accept every authored kind, including the kinds a suite embeds and the experiment document.
- A command that fails MUST name a stable error code, the file the offending value lives in, and its JSON Pointer; a command that reads more than one document MUST name which document failed.
- `validate` MUST decide every locally decidable condition, including a probe's point count and ordering and a policy rule's metric identity against the selected families, and MUST list every other check in `unchecked`.
- Repeated runs on identical inputs MUST produce identical artifacts.
- On the `log_e_film` stage, a measurement reading MUST match the `log_e_film` native observation of a dedicated `experiment run` within a relative tolerance of 1e-9, with the operations the guard names for that range off. Both producers MUST observe the same shared core rather than request process capture.
- On a default chain, `measure run` MUST fail before executing and name every active operation in the measured range with its remediation, and a `--read log_e_film` range MUST list only the operations that reach `log_e_film`.
- With a frozen chart image and a frozen regions document, `sample` MUST reproduce the frozen values, and sampling a retained encoded stage MUST record a linear reading beside the source's own encoded declaration.
- A reference that is unmatched or ineligible MUST report an unavailable reason per pair, and the comparison MUST still publish its other measurements.
- A route that skips a boundary MUST NOT list that boundary's operations, and a range that executes them MUST NOT rely on the route. A chain measured through `input > convert-film > scan` with the camera and development controls left active MUST complete when it covers the whole chain.
- The same chain on a route that runs them MUST fail and name them. A narrower range on a skipping route (`input` with `--enter rgb_in --read log_e_film`, `input > convert-film > scan` with `--read cmy_film`) MUST fail and name the stages its tap topology fires.
- A comparison MUST publish each comparison's own rows and own availability. Every `deltas.csv` row MUST name its comparison.
- A shared key whose group labels differ MUST still compare with the label difference reported, and a key whose units or channel order differ MUST fail.
- A run that fails after executing, and a sampling of a bundle whose buffer disagrees with its manifest, MUST leave no artifact behind.
- A record MUST carry `kind`, and `verify --record` MUST read a run or a comparison without being told which one it is.
- Every artifact MUST survive a round trip: a command MUST read back what it wrote, including identities that carry separators.
- Ordinary suite and single-image runs, including preflight, expose no optional native capture or implicit replacement. New rows explicitly retain absence, and sampling/inspection/reporting never rerender or fabricate exposure, normalization, or native-stage readings.
- Dedicated experiment and measurement records retain the owning replayable invocation and actual observer configuration; existing supported `sample --from-evidence` and `sample --from-run` inputs remain readable without restoring a process capture option.
- `experiment export INPUT --out DIRECTORY` consumes a native observation bundle and never treats a `.sfstage` resume state or debug text as scientific observations.

## Laws

1. One fact, one name: a fact shared by a command, a document, and a record is spelled the same way in all three.
2. A retained record is immutable and self-describing: it names its kind, its schema version, and the identity that produced it.
3. An unavailable measurement is never zero and never an absent row without a reason.
4. Validation never claims more than it checked.
5. No second implementation of a metric, a parameter language, a record format, or a chain-entry mechanism.
6. Publication is all-or-nothing per destination: a reader MAY read the presence of a record as the presence of a complete result.
7. Stability is bound to a kind and its schema version, never to a command name.
8. The instrument decides nothing about quality: it computes values, publishes evidence, and applies only the policy a document declares.
