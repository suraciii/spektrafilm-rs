# Research machine interface

The research surface ships in two binaries: `spektrafilm-lab`, and the f64 reference build `spektrafilm-f64`. The lab quantifies delivered images, compares two quantified images, evaluates completed runs, and publishes reports. The research commands of `spektrafilm-f64` render a declaration into retained evidence beside the exporter commands of the same build.

They share one document family and one machine-output contract.

Normal image editing and delivery do not need either binary. They compute and publish evidence; interpretation belongs to the caller and to the policies a document declares.

- This document owns the command grammar of both binaries, the [document registry](#document-registry), the machine-output contract, and the serialized documents. It does not change the renderer's [CLI parameter language](../cli/spec.md).
- The [lab product specification](../lab/spec.md) owns the lab boundary, its laws, `compare`, analysis profiles, and the report content contract.
- A command name in this document denotes the endpoint of its owning binary, as the [command ownership table](#command-ownership) states. The owning binary is part of the endpoint identity.
- The [lab specification](../lab/spec.md) owns measurement meaning, the [measurement protocol](../lab/metrics/spec.md) owns metric domains and algorithms, and the [profile authoring contract](../profile-authoring/spec.md) owns profile fitting.
- Ordinary rendering MUST NOT provide optional native capture. The measurement endpoint MUST use the common core observer.
[Stage artifacts](../cli/stage-artifacts.md) owns resumable film/print states, and [debug diagnostics](../cli/spec.md#debug-diagnostics) owns human process diagnostics.

## Authority

- Owning modules: the `spektrafilm-lab` binary and the research commands of the `spektrafilm-f64` build, with their command plumbing. The measurement library they orchestrate stays renderer-independent and depends only on the shared mathematics crate.
- Consumes: delivered rasters, authored regions, the shared metric implementations, and the shared document loader.
- MUST NOT own: rendering mathematics, a second parameter registry, a second metric implementation, a second record format, or a second chain-entry mechanism.

In this document, **the research binaries** means `spektrafilm-lab` and the research commands of `spektrafilm-f64`. A rule that names only one endpoint applies to the binary that owns it. The exporter commands that share the `spektrafilm-f64` build are outside this document's surface.

## Unix endpoint model

The research binaries are a Unix-style set of smart endpoints connected by dumb file and process boundaries. Each endpoint has one semantic operation, reads declared inputs, and publishes one immutable record or artifact. It MUST NOT inspect human output, invoke a downstream endpoint, or decide the next research step.

The boundary between endpoints is an authored document, a retained record directory, or a machine-readable command result. A binary stage buffer remains a referenced artifact rather than an implicit shell value. Paths are explicit whenever provenance, relative dependencies, hashes, or artifact identity matter; stdin is not an undeclared alternate document source.

An endpoint MAY be used in a shell pipeline through its stdout result, stderr diagnostics, and named artifacts. stdout carries exactly one machine-readable result object, stderr carries human messages and progress, and neither stream is a hidden control channel. A future streaming form MUST define its carrier and identity explicitly instead of changing the meaning of an existing command.

The research command surface is not a workflow language. It has no includes, environment interpolation, shell commands, expressions, conditionals, loops, parameter arithmetic, wildcard selectors, or optimizer instructions. A declaration selects and describes one endpoint invocation; it does not describe a graph of invocations.

- The renderer parameter language remains the only parameter DSL. `--preset`, `--params`, and `--set` retain the renderer's syntax and precedence.
- Research documents carry typed process options and research identities, but they do not reinterpret or extend renderer parameter paths. Region geometry, stage selection, seed identity, and artifact destinations are research declarations, not runtime parameters.

The supported command families have distinct endpoint roles:

| Family | Endpoint role | MAY consume | MUST NOT do |
|---|---|---|---|
| `sample` | read declared windows from one delivered image | one raster plus `regions` | render, resize, infer geometry, or score |
| `analyze` | quantify one delivered image set | one raster image set plus `regions` and a protocol | compare, invoke a renderer, or report |
| `compare` | quantify two delivered image sets and join them | two raster image sets plus `regions` and a protocol | invoke a renderer or report |
| `compare readings` | compare two retained reading records | retained reading records | invoke a renderer or select a candidate |
| `inspect` / `report comparison` | project retained evidence | completed comparison | recalculate metrics |

### Command ownership

| Binary | Owns |
|---|---|
| `spektrafilm-lab` | `analyze`, `compare`, `compare readings`, `sample`, `inspect`, `report comparison`, `report photos`, `regions`, `document` |
| `spektrafilm-f64` | `spectral evaluate`, `fit` |

An endpoint MUST NOT be accepted by the binary that does not own it. A rejected invocation MUST return `argument_syntax` and name the owning binary and the endpoint, so that a caller reading a script's failure learns where the endpoint lives.

`compare` compares delivered images; `compare readings` joins retained reading records. The subcommand is the discriminator, and the flags required without one are not required with one.

The `inspect` that this table assigns to the lab projects a retained comparison; the `inspect` that inspects one delivered image is an exporter command outside this surface.

A record stays readable after the endpoint that wrote it changes binary. Record compatibility is bound to the kind and its schema version, not to an executable name.

A parent MUST NOT own execution semantics. Each leaf has one input kind, one core operation, one publication boundary, and one handler.

The canonical research pipe is:
```text
declaration → spektrafilm-f64 → images and retained evidence → spektrafilm-lab compare → spektrafilm-lab report comparison
```

Each edge is replaceable by a file, a retained directory, or an external tool. A research binary MUST NOT collapse this pipe into a command that renders, samples, scores, selects a candidate, and publishes a report as one hidden operation.

The endpoint name is authoritative. A retained document kind and schema version are validated by the selected endpoint; they MUST NOT select another endpoint from field presence or directory shape.

The supported command families carry the compatibility promise of this document. `spectral evaluate` and `fit` remain experimental endpoints under their own specifications. An experimental endpoint MAY change or disappear between revisions, but its result MUST still satisfy the document, artifact, and machine-output contracts below.

## Commands

The executables MUST accept the following canonical command grammar:
```text
spektrafilm-lab --help
spektrafilm-lab --version
spektrafilm-lab document --kind KIND
spektrafilm-lab document --kind KIND --check FILE
spektrafilm-lab sample --source IMAGE --input-color-space NAME --input-cctf-decoding true|false --regions FILE --out DIRECTORY
spektrafilm-lab analyze --source FILE_OR_DIR --declaration SPACE:encoded|decoded --out DIRECTORY [IMAGE COMPARISON OPTIONS]
spektrafilm-lab compare --baseline FILE --candidate FILE --declaration SPACE:encoded|decoded --out DIRECTORY [IMAGE COMPARISON OPTIONS]
spektrafilm-lab compare --baseline DIRECTORY --candidate DIRECTORY --declaration SPACE:encoded|decoded --out DIRECTORY [IMAGE COMPARISON OPTIONS]
spektrafilm-lab compare readings --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [--reference DIRECTORY]
spektrafilm-lab inspect --comparison DIRECTORY [--case ID ...] [--region ID ...] [--metric ID ...]
spektrafilm-lab report comparison --comparison DIRECTORY --out DIRECTORY
spektrafilm-lab report photos --manifest FILE --out DIRECTORY
spektrafilm-lab regions --rows N --columns N --patch-size WxH --role ROLE --out FILE [--id ID] [--origin X,Y] [--gap X,Y]
```
```text
spektrafilm-f64 --help
spektrafilm-f64 --version
spektrafilm-f64 spectral evaluate --spectra FILE {--preset NAME ... | --profile FILE} [--set PATH=VALUE|FILE ...] [--data-dir DIRECTORY] --out DIRECTORY
spektrafilm-f64 fit [FIT OPTIONS]
```

`IMAGE COMPARISON OPTIONS` are defined by the [`compare` section](../lab/spec.md#compare) of the lab product specification.

Each canonical leaf has one typed argument group and one semantic handler. The handler MUST NOT select a different contract from optional field presence or from the shape of a retained document.

- The former flat forms MAY remain only as deprecated compatibility aliases during one migration revision. An alias MUST map to one canonical handler before endpoint execution, MUST accept no new options, and MUST reject mixed endpoint options with `argument_syntax`.
- The alias layer MUST NOT contain document-shape dispatch. New documentation and examples MUST use the canonical grammar.
- A later compatibility revision MAY remove the aliases.

The command family names are semantic, not workflow steps. `compare readings` reads only retained records and never invokes the next family. A shell pipeline MAY connect endpoints by paths and result objects, but the research binaries do not own that control flow.

- `KIND` is a [document registry](#document-registry) kind.
- Every option of both binaries MUST have a nonempty help description.
- Version MUST identify the binary implementation and supported document versions. Every binary's help MUST identify the endpoint families it owns.
- `document` without `--check` MUST emit the authoritative JSON Schema for one document kind. For the `metrics` vocabulary it MUST emit the metric-identity vocabulary instead of a JSON Schema.
- It MUST describe the required nested records and explain fields whose units, coordinate space, or applicability are not evident from their type. `document --check` MUST decide the document and resolve the local dependencies needed for that kind.
- It MUST NOT render images.

- Native stages, observed camera exposure, and prepared print normalization that are not independently retained remain explicitly unavailable; authored controls and ordinary diagnostics are not substitute numerical observations. No sampling, comparison, inspection, or reporting endpoint MAY rerender or reconstruct those absent values.
- `compare` MUST quantify the two delivered image sets the [lab product specification](../lab/spec.md#compare) declares. It MUST NOT load another document kind or an executed command.
- Each case MUST retain the measurements, regions, automatic regions, source-image artifacts, and difference maps of its one pair. The pair identity MUST be `baseline/candidate` wherever a region readout or a finding names it.
- An image-source case has no seed: the comparison MUST NOT carry a seed field, and MUST NOT publish a cross-realisation statistic.
- The comparison MUST use the version shape and MUST NOT carry a `mode` field. Its `baseline` and `candidate` blocks MUST use `kind` equal to `images`, the declared color space, the transfer state, one file per case with its relative artifact path, SHA-256, and dimensions, and the digest of that declaration.
- Those blocks MUST NOT contain another document kind or an executed command.
- A delivered image has no executed command. The comparison MUST NOT fabricate one, and a reader MUST NOT reconstruct one.
- A region, a protocol, or a mask that a comparison consumed MUST be copied into that comparison and hashed. A reader MUST NOT require the caller's original file.
- Every file an image source block names MUST be hashed in that block, so a reader can re-hash it against the bytes in front of it.
- Each case MUST carry `baseline_source` and `candidate_source` entries, written by the shared artifact writer, with the retained raster path, SHA-256, dimensions, and declared color interpretation. Reporting MUST resolve a case image from those entries.
- The comparison MUST carry `evaluation_status` equal to `not_evaluated` and `findings` as an empty array, because it reports no verdict. `selection` MUST name its case identities.
- A region that selects a family whose probe is absent MUST retain an unavailable row with the family's own reason. The other selected families MUST still measure.
- `compare readings` MUST load two retained reading records and join them by `(sample_id, stage)`. Duplicate keys fail.
- Missing keys remain unmatched with an explicit reason. Shared keys with incompatible domain, units, channel order, or coordinate space fail as an identity mismatch.
- It MUST NOT read image-domain masters or invoke a renderer. An optional retained reference is input evidence.
- The comparison source MUST derive its views from the comparison and cited artifacts. It MUST NOT recompute metrics.
- Its output MUST remain usable without the source directories by copying required review artifacts and retaining their provenance. An external original that cannot be redistributed MUST remain excluded from a distributable report; the report MUST identify the omission rather than substitute another image.
- `regions` MUST expand one finite grid declaration into a normal `regions`
  document without reading an image, invoking a renderer, computing
  measurements, or generating a page.
- It MUST write `render` geometry, the only coordinate space a delivered raster
  has, rather than offer a choice whose other values it would refuse to measure.
  It MUST require the role, because a generator does not decide what a region is
  for. Origin, gap, and identity MUST default to the pixel origin, no gap, and
  the output file stem.
- It MUST validate dimensions, origin and gap arithmetic before publishing. A
  region names no measurement family, because the analysis profile owns the
  table from role to families.
- The generated document is the only input geometry consumed by later `sample` and `compare` endpoints; downstream endpoints MUST NOT interpret a grid declaration.

Irregular geometry MUST be authored directly as a `regions` document, or declared with `--roi` on a comparison. `--roi` declares windows on one source and names no image or default pair.

- `inspect` MUST validate and read a retained comparison without invoking a renderer or computing measurements. It MUST emit the retained comparison identity, execution state, conditions, summary, failures, findings, and compact measurements.
- The compact measurements MUST be one top-level `measurements` array. Rows MUST carry the same fields in every comparison.
- Per-seed observations and cross-seed `seed_measurements` MUST remain separate. Measurement identity, domain, units, estimator, sample count, availability, value or reason, and artifact references MUST remain intact.
- Exact repeatable case, region, and metric filters MUST reject unknown selections with available choices; they MUST NOT silently return an empty result. Referenced evidence MUST resolve relative to the emitted `artifact_base`.
- A valid read of a failed comparison MUST complete successfully while exposing its failed comparison state. Inspection MUST NOT modify evidence, rank unlike units, or infer acceptance.
- Inspection MUST project retained `region_readouts` arrays with case identities. Case and region selectors MUST apply to these arrays; metric selectors MUST apply only to scalar measurements and findings.
- `readout_status` MUST distinguish retained arrays from `not_retained` fields in historical comparisons. Inspection MUST NOT backfill historical values.
- Report locations and page navigation belong to the [lab report](../lab/report/spec.md). The command grammar does not define page fragments.
- `sample` MUST validate coordinate-space compatibility before reading any region. Equal dimensions do not establish compatibility.
- A `source` region is compatible with a declared raster or `source_pixels`; a `render` region is compatible with `output_pixels`; a native stage region is compatible only with the same native stage name. The sampler MUST NOT resize, crop, pad, translate, or silently map a mismatch.
- `document --check` MUST decide every condition that is decidable from the document, its declared dependencies, and the local filesystem. It MUST list in `unchecked` every check it did not perform, including checks that need another document, a renderer, a compute device, or a measurement.
- A check the validator did not perform MUST NOT be omitted from `unchecked`, and a condition it could have decided MUST NOT appear there. Passing `document --check` MUST never be presented as proof that an execution will succeed.

## Machine output

- Every command result MUST identify `schema_version`, `command`, `execution_status`, and the terminal artifact when one exists. `document --kind` without `--check` emits one schema object without a result wrapper.
- A result MUST name the binary and the endpoint that produced it, so a reader can tell a lab result from a render result without inspecting the payload.
- An execution failure MUST identify `execution_status` as `failed` and MUST contain structured errors. A published failed manifest MUST remain linked from the result; its detailed errors MUST NOT replace the command's structured error summary.
- Human error text MUST NOT be the only machine discriminator.
- A requested gate failure after successful computation MUST retain `execution_status` as `completed` and MUST expose its non-pass `evaluation_status`.
- `compare` MUST report progress on stderr as line-oriented text. Every selected case MUST produce a start line and a completion line.
- A completion line MUST identify the item and its elapsed seconds. The last progress output before the command result MUST state the completed and failed item counts and the total elapsed seconds.
- Progress MUST NOT require a terminal.
- The executable MUST return status `2` for argument syntax errors. It MUST return status `1` for validation, rendering, measurement, publication, or a requested gate failure.
- It MUST return status `0` when the requested operation completed without those failures. The difference between execution and evaluation MUST remain visible in JSON even when both use status `1`.
- A comparison with a failed case MUST have `execution_status` equal to `failed`.
- A comparison with an unavailable measurement caused by an unsupported reference scope MAY complete execution while keeping that row unavailable.
- A numerical computation failure MUST remain an execution failure.

## Carrier and validation

All documents MUST use UTF-8 JSON and MUST carry `schema_version` equal to `1`. Authored documents MUST reject duplicate keys and unknown fields. They MUST NOT support includes, environment interpolation, expressions, shell commands, or executable plugins.

- Schemas MUST describe document structure, types, ranges, and field applicability. The validator MUST also check duplicate identities, nonfinite numbers, unsafe artifact paths, and identifiers resolvable from the supplied document.
- Checks that require file contents MUST use the declared dependencies. Invalid authored probe point counts, coordinate ordering, mutually exclusive fields, and auxiliary reference document structure MUST fail validation before rendering.
- Runtime parameter and source-declaration semantics MUST use the renderer's shared definitions rather than a second parameter registry.
- Generated comparison readers MAY ignore unknown fields within a supported schema version. Required semantic fields MUST still be validated.
- Unsupported versions MUST fail rather than select defaults. Generated documents MUST preserve full unsigned seed and counter values.
- Authored file paths MUST resolve relative to the document that contains them. A document that names a chain carries parameter values, not a parameter language: it MUST use the `process` surface's names and precedence, and resolving it MUST be the resolution the user binaries perform.
- A parameter file a document names MUST resolve inside that document's directory tree; a path that escapes it MUST fail. Executable paths MUST be explicit paths, not commands resolved from `PATH`.
- Comparison artifact paths MUST be relative to their containing directory. Artifact paths MUST NOT contain an absolute root, parent traversal, or a symbolic link that escapes that directory.
- File dependencies outside the artifact directory MAY be consumed through authored paths but MUST be hashed and identified.
- A reader MUST retain bytes from the filesystem-resolved dependency path. It MUST preserve the absolute authored path separately for offline lookup, including parent components after symbolic links.
- A comparison MUST match that authored identity before looking up a lexically normalized identity from a historical record. Current-directory components MUST NOT distinguish equivalent authored paths.
- Neither lookup MAY resolve the original filesystem.

## Document registry

Every JSON document a research binary reads or writes has one kind and one owning section. Every document carries `schema_version`.

`document --kind KIND` MUST emit the authoritative JSON Schema for every kind, including the nested records a consumer MUST read to diagnose a result: measurement rows, findings, summaries, failure records, and artifact references.

`document --kind KIND --check FILE` MUST accept every authored kind and resolve the local dependencies that kind needs. A kind this binary prints but cannot decide MUST be refused with `unvalidatable_document_kind`, not reported as unknown.

| Kind | State | Owner |
|---|---|---|
| `regions` | authored | [Regions document](#regions-document) |
| `comparison` | retained | [Comparison record](#comparison-record) |
| `analysis` | retained | [Comparison record](#comparison-record) |
| `report` | retained | [Commands](#commands) |
| `metrics` | vocabulary | [Measurement protocol](../lab/metrics/spec.md) |

- A kind's state decides which commands accept it. An authored kind is written by a person or another tool and MAY be validated.
- A retained kind is published by a research binary, is immutable, and is consumed by `compare`, `inspect`, and `report`. `metrics` is not a document: it is the closed vocabulary of metric identities a record MAY name, and the same vocabulary the measurement protocol owns.

## Regions document

A regions document declares reading positions. One kind serves the sampling command and a comparison's declared windows.

- A regions document MUST contain `schema_version`, an `id`, and a nonempty `regions` array. A region MUST contain `id`, `geometry`, and `role`, MAY contain `label`, `source`, and `partition`, and MUST NOT declare measurement families, because an analysis profile owns the one table from role to families and a second declaration could disagree with it.
- `role` MUST identify `skin`, `neutral`, `shadow`, `highlight`, `color`,
  `flat_field`, `edge`, `texture`, or `other`. A regions document MUST NOT
  declare the families a region measures, because the analysis profile owns
  that table.
- `source` and `partition` are group labels for reporting. Identities MUST be unique within the document, MUST match `^[A-Za-z0-9][A-Za-z0-9_.-]*$`, and `full_image` stays reserved for the whole declared source.
- `geometry` MUST be either a half-open rectangle with the named fields `x`, `y`, `width`, `height`, or a lossless mask with its `path`, `sha256`, and declared `width` and `height`. Widths and heights MUST be positive integers.
- Every geometry MUST declare its coordinate space. `source` names the declared source image's pixels; `render` names a completed simulation's output pixels; native stage names are `source_pixels`, `working_pixels`, `film_frame_pixels`, `grain_aperture_pixels`, or `output_pixels`.
- Mask paths MUST resolve relative to the document that contains them and enter the retained dependency inventory as immutable evidence.
- Regions MAY overlap: a reading carries its own identity, and a narrower window inside a wider one is a declared intent, not a conflict. Correspondence is never inferred from equal dimensions, labels, or shared region identities.
- `regions` is the only standalone geometry generator. It writes a normal `regions` document and does not read images or produce a review page.
- `compare --roi` declares windows inline and materializes the same `regions` shape as its retained geometry authority. It is a declaration carrier, not a second geometry language: it names no image, pair, inventory, or default region.
- Non-grid geometry is authored directly. The sampling command and a comparison consume the same regions shape through one validator.
- Image-domain consumers accept only `source` and `render`. Native stage values require a producer mapping and are rejected there with `unsupported_coordinate_space` until one exists.

## Prepared-image review boundary

`review` is not a document-registry kind, and `document --kind review` MUST
return a stable unsupported-kind error. The removed manifest source MUST NOT be
represented as a compatibility alias or a second result kind. `report
comparison`'s source is a retained comparison only.

`report photos` MAY present a prepared-image inventory for local visual
inspection. Its image inventory, per-image geometry, default pair, captions,
display conversion, and report builder belong to the photo report contract. The
measurement endpoints MUST NOT import that manifest, derive regions from it, or
shape-dispatch on it.


## Comparison record

A comparison directory MUST contain `comparison.json`, scalar measurements, map and curve artifacts, and selected detail-region records. Every finding MUST resolve to a retained artifact.

- `comparison.json` MUST contain `schema_version`, `kind` equal to `comparison`, `id`, `execution_status`, `protocol`, `protocol_digest`, `map_scale`, `selection`, `condition_differences`, `baseline`, `candidate`, `cases`, `summary`, and `artifacts`. A new comparison also carries `evaluation_status` equal to `not_evaluated` and an empty `findings` array; a historical record MAY omit them, and a reader MUST treat the omission as the same fact.
- An image comparison retains `evaluation_status` equal to `not_evaluated`: it reports a difference, never a verdict.
- A source block MUST declare `kind` equal to `images` and retain its `id`, the declared color space, the transfer state, one file per case with its relative artifact path, SHA-256, and dimensions, and the digest of that declaration.
- A source block MUST NOT carry another document kind or an executed command.
- A `condition_differences` entry MUST report a changed color space, a changed transfer state, a changed dimension, or a changed analysis identity.
- Each case MUST retain its `case_id`, `regions`, `automatic_regions`, `region_readouts`, scalar measurements, review artifacts, and difference maps. The pair identity MUST be `baseline/candidate` wherever a region readout or a finding names it.
- An image-source case has no seed: the comparison MUST NOT carry a seed field, and MUST NOT publish a cross-realisation statistic.
- New comparisons MUST retain `region_readouts` on case records. The [comparison schema](../../crates/spektrafilm-lab/assets/comparison.schema.json) owns their field structure.
- The [measurement protocol](../lab/metrics/spec.md#retained-region-coordinates) owns estimators and eligibility. Historical version-1 records MAY omit these arrays.
- Consumers MUST NOT interpret omission as an empty measured population or a zero result. Available observations MUST contain finite values; unavailable observations MUST contain a reason and no value.
- Color readouts MUST identify `pair_id`. Artifact references MUST resolve inside the retained comparison.
- Each scalar measurement MUST identify case, region, metric identity, domain, units, estimator, sample count, availability, and artifact references. Available measurements MUST contain `value`.
- Other states MUST contain `reason` and no value. Curves, spectra, and maps MUST retain their sample coordinates, units, dimensions, and hashes.

Each finding MUST contain `id`, `case_id`, `region_id`, `metric`, `decision`, and `evidence`. Automatic detail records MUST retain their rectangle, ranking metric, ranking value, and merge/selection method. A prose explanation MAY supplement but MUST NOT replace these fields.

The summary MUST retain selected and completed case counts and worst-case findings. It MUST NOT contain a universal appearance score.

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

- `compare` and `report` MUST require a destination that does not exist. They MUST NOT replace completed evidence.
- They MUST publish a terminal manifest only after referenced local artifacts have been finalized and hashed. A handled operation failure MAY publish a failed terminal manifest and valid partial evidence.
- Forced termination before the terminal manifest MUST leave an incomplete directory that cannot be treated as a completed run.
- A gate failure MUST preserve the completed comparison. A report-generation failure MUST NOT modify its source comparison.
- Publication failures MUST NOT produce a success result. This contract does not promise a transaction across different destination directories or power-loss durability.
- The implementation MUST ship schema-validated examples for every authored document kind. Command examples MUST execute with packaged fixture assets and the real renderer.
- Documented research usage examples MUST be checked against the implemented command and flag surface by a permanent test. A documented command or flag the executable does not accept MUST fail that test.
- The schema, parser, examples, and validator MUST change together. The interface MUST NOT claim executable examples or a validated schema before these artifacts are exercised.
- An executable example MUST use `document`, `compare`, and `report` to complete a comparison without source-code inspection. It MUST read metric identities, units, availability, and evidence links from the resulting measurements.

## Acceptance

- `document --kind` MUST cover every kind in the [document registry](#document-registry), and `document --kind KIND --check` MUST accept every authored kind.
- A command that fails MUST name a stable error code, the file the offending value lives in, and its JSON Pointer; a command that reads more than one document MUST name which document failed.
- `document --check` MUST decide every locally decidable condition, including a probe's point count and ordering and a metric identity against the selected families, and MUST list every other check in `unchecked`.
- Repeated runs on identical inputs MUST produce identical artifacts.
- With a frozen chart image and a frozen regions document, `sample` MUST reproduce the frozen values, and sampling a retained encoded stage MUST record a linear reading beside the source's own encoded declaration.
- A reference that is unmatched or ineligible MUST report an unavailable reason per pair, and the comparison MUST still publish its other measurements.
- A route that skips a boundary MUST NOT list that boundary's operations, and a range that executes them MUST NOT rely on the route. A chain measured through `input > convert-film > scan` with the camera and development controls left active MUST complete when it covers the whole chain.
- The same chain on a route that runs them MUST fail and name them. A narrower range on a skipping route (`input` with `--enter rgb_in --read log_e_film`, `input > convert-film > scan` with `--read cmy_film`) MUST fail and name the stages its tap topology fires.
- A comparison MUST publish each comparison's own rows and own availability. Every `deltas.csv` row MUST name its comparison.
- A shared key whose group labels differ MUST still compare with the label difference reported, and a key whose units or channel order differ MUST fail.
- A command that fails after executing, and a sampling of a bundle whose buffer disagrees with its manifest, MUST leave no artifact behind.
- A record MUST carry `kind`, so a reader can read a record without being told which one it is.
- Every artifact MUST survive a round trip: a command MUST read back what it wrote, including identities that carry separators.
- Ordinary rendering exposes no optional native capture or implicit replacement. New rows explicitly retain absence, and sampling/inspection/reporting never rerender or fabricate exposure, normalization, or native-stage readings.
- Measurement records retain the owning replayable invocation and the observer configuration they used.
- `spektrafilm-lab` MUST reject every endpoint the [command ownership table](#command-ownership) assigns to `spektrafilm-f64`, and `spektrafilm-f64` MUST reject every lab endpoint. Each rejection MUST return `argument_syntax` and name the owning binary.
- `spektrafilm-lab report comparison` MUST succeed on a host with no renderer executable and no graphics device.
- A comparison MUST carry one case per declared pair, source blocks of `kind: images`, and no command record.
- Two identical images MUST produce a comparison whose available deltas are zero and whose unavailable rows carry a reason.

## Laws

1. One fact, one name: a fact shared by a command, a document, and a record is spelled the same way in all three.
2. A retained record is immutable and self-describing: it names its kind, its schema version, and the identity that produced it.
3. An unavailable measurement is never zero and never an absent row without a reason.
4. Validation never claims more than it checked.
5. No second implementation of a metric, a parameter language, a record format, or a chain-entry mechanism.
6. Publication is all-or-nothing per destination: a reader MAY read the presence of a record as the presence of a complete result.
7. Stability is bound to a kind and its schema version, never to a command name.
8. The research binaries decide nothing about quality: they compute values and publish evidence.
