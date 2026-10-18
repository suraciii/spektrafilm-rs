# SpektraFilm lab CLI

## Purpose and authority

`spektrafilm-lab` is the Unix-style research command surface for declared rendering, retained evidence, point-wise sampling, and evidence comparison. It publishes immutable machine-readable records and named artifacts. It does not decide which candidate is better unless the caller selects an explicit policy.

This specification owns the lab command grammar, endpoint boundaries, regions generation, and lab-specific migration rules. The [instrument machine interface](../instrument/spec.md) owns shared document fields, artifact publication, error status, schema discovery, and stdout/stderr behavior. The [benchmark specification](../benchmark/spec.md) owns metric meaning, reference scope, and policy evaluation. The renderer's [CLI specification](../cli/spec.md) owns renderer parameters and native stage evidence.

The lab product does not use a review manifest. A `review` document is not an instrument document kind, is not accepted by `spektrafilm-lab validate`, and is not a source for `sample` or `report`. A separately maintained benchmark viewer may retain a prepared-image input contract, but that contract is outside the lab product and must not be passed through this interface.

## Product pipe

The canonical pipe is:

```text
authored declaration → run → retained evidence → sample → retained readings → compare → inspect/report
```

Each arrow is a file, a retained directory, or a machine-readable result object. An endpoint performs one semantic operation. It must not invoke the next endpoint, parse another command's stderr, or infer a downstream action from a result shape.

| Endpoint | Owns | Does not do |
|---|---|---|
| `run suite` | execute one authored image-domain suite | execute an image-only adapter, measure stimuli, sample, compare, select, or report |
| `run image` | adapt one local image into one image-domain run | execute a suite, measure stimuli, sample, compare, select, or report |
| `measure run` | execute one declared point-wise stimulus chain and retain stage evidence | execute an image-domain run, sample, compare, select, or report |
| `regions grid` | expand one finite grid declaration into explicit regions | read images, render, sample, compare, or create a review page |
| `sample` | read declared windows from one source | render, resize, infer geometry, or score |
| `compare runs` | compare two completed image-domain runs | compare readings, select a candidate, invoke a renderer, or report |
| `compare readings` | join and compare two retained reading records | compare image-domain runs, invoke a renderer, or report |
| `compare pairs` | create explicitly requested pairs from one retained run | compare two runs, infer a default pair, invoke a renderer, or report |
| `inspect` | project retained comparison evidence | recalculate metrics, evaluate policy, or alter decisions |
| `report comparison` | build an offline page from one retained comparison | accept a manifest, render, or recalculate metrics |
| `verify` | re-hash one retained record and its dependencies | rewrite, render, compare, or judge |

The parent names `run`, `measure`, and `compare` group leaf endpoints. A parent must not own execution semantics. Each leaf has one input kind, one core operation, one publication boundary, and one handler.

The instrument must keep leaf endpoints independently replaceable. A shell script or another program may control the pipe. The lab does not become a workflow language.

The following flow is the only product-owned composition:

```text
authored declaration → run suite|image or measure run → retained evidence → sample → retained readings → compare runs|readings|pairs → inspect/report comparison
```

An arrow is a file, retained directory, or machine-readable result object. A leaf endpoint must not invoke the next leaf, parse another command's stderr, or infer a semantic handler from a document's field presence.

The endpoint name is authoritative. A retained document kind and schema version are validated by the selected endpoint; they do not select a different endpoint.

## Command surface

The canonical grammar is:

```text
spektrafilm-lab schema --kind KIND
spektrafilm-lab validate --kind KIND --file FILE
spektrafilm-lab target --renderer FILE --data-dir DIRECTORY --backend cpu|gpu --out FILE
spektrafilm-lab run suite --suite FILE --target FILE --out DIRECTORY [RUN OPTIONS]
spektrafilm-lab run image --input FILE --configuration FILE --target FILE --out DIRECTORY [RUN OPTIONS]
spektrafilm-lab measure run --stimuli FILE --enter BOUNDARY --out DIRECTORY [CHAIN OPTIONS]
spektrafilm-lab regions grid --id ID --rows N --columns N --origin X,Y --patch-size WxH --gap X,Y [--sample WxH] [--space SPACE] [--role ROLE] [--measurements FAMILY,...] --out FILE
spektrafilm-lab sample --source IMAGE --input-color-space NAME --input-cctf-decoding true|false --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-evidence DIRECTORY --stage STAGE --regions FILE --out DIRECTORY
spektrafilm-lab sample --from-run DIRECTORY|RUN.JSON --case ID --seed UINT32 --stage STAGE --regions FILE --out DIRECTORY
spektrafilm-lab compare runs --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [COMPARISON OPTIONS]
spektrafilm-lab compare readings --baseline DIRECTORY --candidate DIRECTORY --out DIRECTORY [--reference DIRECTORY]
spektrafilm-lab compare pairs --run DIRECTORY --against input|reference|both --out DIRECTORY
spektrafilm-lab inspect --comparison DIRECTORY [--case ID ...] [--region ID ...] [--metric ID ...]
spektrafilm-lab report comparison --comparison DIRECTORY --out DIRECTORY
spektrafilm-lab verify --record DIRECTORY [--data-dir DIRECTORY]
spektrafilm-lab fixtures --renderer FILE --data-dir DIRECTORY --out DIRECTORY [--natural-input FILE | --download-natural]
```

`run suite`, `run image`, `measure run`, `compare runs`, `compare readings`, and `compare pairs` are distinct command contracts. They must have distinct typed argument groups and distinct semantic handlers. A handler must not choose among these contracts from optional field presence or from the shape of `run.json`.

The former flat forms may remain only as deprecated compatibility aliases during one migration revision. An alias must map to one canonical handler before endpoint execution, must accept no new options, and must reject mixed endpoint options with `argument_syntax`. The alias layer must not contain document-shape dispatch. New documentation and examples must use the canonical grammar. A later compatibility revision may remove the aliases.

`experiment run`, `experiment export`, `spectral evaluate`, and `fit` remain experimental endpoints under their own specifications. They use the same document, artifact, and machine-output contracts. They must not add a review input, workflow graph, sweep language, optimizer language, or second renderer parameter DSL.

Every command option must have a nonempty help description. Help, version, and schema discovery must not require a renderer, image decoder, input asset, or graphics device. `--help` must identify each canonical leaf operation.

## Documents and artifacts

The lab consumes and publishes the document kinds in the instrument registry except `review`. The relevant kinds are:

| Kind | State | Lab use |
|---|---|---|
| `target`, `suite`, `configuration`, `regions`, `stimuli`, `policy`, `reference`, `preparation` | authored | declarations and dependencies |
| `run`, `comparison`, `preflight`, `report`, `evidence` | retained | endpoint results and cited evidence |
| `metrics` | vocabulary | metric identity owned by the benchmark protocol |

A `regions` document is a measurement geometry document. It is not an image inventory, a comparison pair, a caption store, a browser state document, or a report manifest. It has one owner and one purpose: declare stable reading identities and the geometry each reading uses.

A retained reading record owns sampled values, source provenance, stage metadata, coordinate-space metadata, availability, and artifact references. A retained comparison owns joins, deltas, summaries, findings, and copied evidence needed by inspection and reporting. A report owns presentation artifacts only. No command may move facts between these owners to avoid declaring them.

## Regions document

A regions document contains `schema_version`, `id`, and a nonempty `regions` array. Each region contains `id`, `geometry`, `role`, and `measurements`, and may contain `label`, `source`, and `partition`. Region identities are unique, stable, and match `^[A-Za-z0-9][A-Za-z0-9_.-]*$`. `full_image` is reserved for the complete declared image.

`geometry` is a half-open rectangle or a hashed mask. Rectangles use nonnegative `x` and `y` and positive `width` and `height`. Masks carry a path, SHA-256, width, height, and coordinate-space declaration. Paths resolve relative to the regions document and become retained dependencies when the document enters a run or reading record.

The geometry's `space` identifies the producer coordinate grid. The accepted authoring values are `source` and `render`; native stage values are `source_pixels`, `working_pixels`, `film_frame_pixels`, `grain_aperture_pixels`, and `output_pixels`. Compatibility is exact:

| Region `space` | Compatible producer declaration |
|---|---|
| `source` | a declared raster source or stage `source_pixels` |
| `render` | a completed render or stage `output_pixels` |
| any native stage value | a stage whose `geometry.coordinate_space` has the same value |

The `source` and `render` values are authoring names retained for image-domain cases. A producer must publish its native stage name. The sampler must resolve the mapping above before reading any pixel. Equal width and height do not establish compatibility. The sampler must reject a mismatch before reading, and must not resize, crop, pad, translate, or otherwise repair it.

A regions document may contain overlapping regions. Overlap is declared intent, not an error. A region's `role`, `source`, `partition`, and `measurements` are reporting metadata; they do not establish correspondence between two images or two runs. A shared region identity is not proof of spectral, geometric, or pixel registration.
Standalone `regions` and retained-stage sampling may use native spaces only when the selected producer publishes the exact matching coordinate space. Suite, reference, and benchmark image consumers do not yet implement those mappings and must reject native-space geometry before consuming it; the current validator reports `unsupported_coordinate_space` rather than treating native values as source or render coordinates.

## `regions grid`

`regions grid` is a deterministic document generator:

```text
spektrafilm-lab regions grid \
  --id ID --rows N --columns N --origin X,Y \
  --patch-size WIDTHxHEIGHT --gap X,Y \
  [--sample WIDTHxHEIGHT] [--space source|render|source_pixels|working_pixels|film_frame_pixels|grain_aperture_pixels|output_pixels] \
  [--role ROLE] [--measurements FAMILY,...] --out REGIONS.json
```

The endpoint expands one row-major grid into an ordinary `regions` document. It generates spreadsheet-style column labels (`A` through `Z`, then `AA` and onward) and one-based row labels. The region identity is the column label followed by the row number. The generated order is stable and is part of the document identity.

`--id` is the document identity. `--rows` and `--columns` are positive. `--origin` and `--gap` are nonnegative. `--patch-size` is positive. `--sample`, when present, is positive and must fit inside every patch. A sample window is centered in its patch using integer arithmetic. `--space` defaults to `source`; `--role` defaults to `other`; omitted `--measurements` produces an empty array. The accepted roles and measurement families are the vocabularies in the regions schema and benchmark metric protocol.

The endpoint must check every addition and multiplication for overflow. It must reject a sample that does not fit. It must not inspect an image, decode a file, read dimensions, render a patch, or infer a correspondence. Bounds against a producer are checked later by `sample` or by the consuming run validator. The output is a normal regions document and can be authored, reviewed, hashed, and reused without the grid declaration.

`regions grid` must refuse an existing destination, validate the complete output before publication, and emit one result object naming the output artifact and region count. It must not emit a review manifest or an HTML file.

Irregular geometry is authored directly as a `regions` document. The lab must not add repeated `--region`, `--image`, `--pair`, `--default-region`, or equivalent options that move a manifest into an argument list.

## Sampling

`sample` reads exactly one source and exactly one regions document. It accepts a declared raster source, one retained evidence directory and stage, or one retained image-domain run selected by `(case, seed, stage)`.

For a raster source, the caller must provide the same explicit color-space and transfer-state declaration required by `process`. For retained evidence, the stage bundle is authoritative for domain, units, channel order, transfer state, coordinate space, layout, and buffer digest. For a retained run, the selected case must be completed, stage evidence must be requested and available, and the run artifact index, case reference, manifest path, and digest must agree. Unsafe paths, cross-seed references, missing stages, failed cases, unavailable capture, malformed bundles, and digest mismatches fail the operation without fallback.

Before reading a value, `sample` must verify that every region is compatible with the source coordinate space and lies within the source bounds. It must report the region identity in a bounds failure. It must never clamp, pad, extrapolate, resize, crop, or infer a region from labels or image dimensions.

The sampler may apply only the declared transfer decoding required to express the reading. It must preserve the source's original declaration beside the reading declaration. It must retain the producer coordinate space, domain, units, channel order, source identity, stage identity, and hashes. A sample never executes a chain and must record `executed: false`.

A successful sample publishes one retained reading directory and its terminal result. The record must contain one row for every declared region and stage, including an explicit unavailable reason when a value cannot be produced. Unavailable is never zero and never an omitted row without a reason.

## Comparison

`compare runs` compares completed image-domain runs. `compare readings` compares retained reading records. `compare pairs` creates the explicitly requested input/simulation or reference/simulation pairs from one retained run. Each form has one handler and one input contract.

Reading comparison joins by the composite identity `(sample_id, stage)`. Duplicate keys fail. A missing key is retained as unmatched. Shared keys with different domain, units, channel order, or incompatible coordinate space fail as an identity mismatch; the command must not truncate the join or silently transform a value. Group labels are provenance and may differ without preventing a value comparison; the difference remains visible.

The comparison engine uses the shared metric implementation. It publishes per-reading deltas, group summaries, scalar measurements, availability reasons, and provenance. It must preserve each comparison pair's own availability. A missing or ineligible reference must not make another pair unavailable, and no unavailable value may be serialized as zero.

A comparison must not invoke a renderer, read a review manifest, select a candidate, infer a default pair, or compute a score outside the selected metric and policy contracts. A requested policy gate may change the command status after publication, but it must not delete or rewrite the completed comparison.

## Inspection and reporting

`inspect` reads a retained comparison and projects its identity, conditions, summary, findings, compact measurements, readouts, availability, and artifact references. Filters select exact case, region, and metric identities. Inspection does not recalculate metrics and does not backfill omitted historical values.

`report comparison` builds one offline inspection page from one retained comparison:

```text
spektrafilm-lab report comparison --comparison DIRECTORY --out DIRECTORY
```

The report source is always a retained comparison. The command must copy or link only indexed, permitted artifacts and must retain their provenance. It must show the selected pair's images, measurements, detail artifacts, coordinate spaces, unavailable reasons, and conditions without assigning a new measurement. It must work when the renderer and original run directories are unavailable, subject to redistribution restrictions recorded by the comparison.

The lab report command does not accept `--manifest`, `review.json`, `--default-pair`, `--left-image`, `--right-image`, or any equivalent image-inventory input. A prepared-image page, if retained by the benchmark product, is generated by that product's own command and is not a lab report. The lab must not expose a second report builder for it.

## Machine output and failures

The shared instrument contract applies:

- `schema` emits one schema object without a result wrapper.
- Every other successfully parsed command emits one UTF-8 JSON result object on stdout.
- Progress, child-process text, and human diagnostics use stderr.
- Argument syntax failure returns `2`; validation, execution, measurement, and publication failure return `1`; successful completion returns `0`.
- A result identifies `schema_version`, `command`, `execution_status`, and the terminal artifact when one exists.
- A failure names a stable error code, failed boundary, input document, file, and exact JSON Pointer when applicable.
- A failed operation never publishes a successful terminal record.

The stdout result is the only command-to-caller control record. A caller may pass its named artifact paths to another endpoint, but an endpoint must not parse another endpoint's stderr, progress text, or result prose. Named artifacts must carry their own document kind, schema version, relative paths, digests, and producer identity.

`run suite`, `run image`, `measure run`, `sample`, `compare runs`, `compare readings`, `compare pairs`, `inspect`, and `report comparison` must not change their machine-output contract based on terminal detection. They must be usable with redirected stdout and stderr and in a pipeline that treats stdout as JSON.

The implementation migration must proceed in this order:

1. Define separate typed argument groups and handlers for `run suite`, `run image`, and `measure run`.
2. Route each existing flat invocation to exactly one handler before execution. Preserve artifact bytes, result identity, exit status, publication behavior, and stderr semantics.
3. Define separate typed argument groups and handlers for `compare runs`, `compare readings`, and `compare pairs`.
4. Remove directory-shape dispatch and reject options that belong to another comparison contract.
5. Add canonical-form coverage before changing or removing compatibility aliases.

The migration must not add a fourth run mode, a fourth comparison mode, a workflow document, a sweep language, an optimizer language, a second renderer parameter DSL, or an implicit stdin document carrier.

## Schema and compatibility policy

The instrument document registry must remove `review`. `schema --kind review` and `validate --kind review` must return a stable unsupported-kind error from `spektrafilm-lab`. The `regions` schema remains the authoritative schema for both authored case geometry and standalone sampling plans. Its coordinate-space vocabulary must include every producer space that the sampler accepts.

The removal is a deliberate lab interface revision. Existing retained `run`, `evidence`, `reading`, and `comparison` records remain readable when their schema versions are supported. Existing regions documents remain usable when they satisfy the regions schema. A review manifest is not converted automatically into regions because conversion would lose image inventory, per-image geometry, default-pair intent, or visual-review semantics. The caller must author a regions document or invoke `regions grid` with an explicit declaration.

The benchmark may maintain a separate prepared-image review contract during migration. If it does, its manifest kind, validator, report builder, and command names belong to the benchmark product and must not be registered, linked, or shape-dispatched by the lab. No lab fixture, example, or acceptance test may require that separate manifest.

## Acceptance

1. `regions grid` produces the same byte-level regions document for identical arguments and refuses arithmetic overflow, invalid samples, and existing destinations.
2. A generated grid document contains only explicit regions. It contains no image paths, image identities, pair, default-pair, caption, HTML, or browser state.
3. An authored irregular regions document can be reused unchanged by two samples and a later comparison.
4. Sampling rejects a region/source coordinate-space mismatch before reading pixels, even when dimensions are equal.
5. Sampling a retained stage reads domain, units, channel order, coordinate space, and digest from the stage bundle rather than command-line duplicates.
6. `sample --from-run` selects exactly one completed `(case, seed, stage)` and rejects traversal, cross-seed, unavailable, malformed, and digest-mismatched artifacts.
7. A comparison preserves unmatched and unavailable rows with reasons, never writes an unavailable value as zero, and does not borrow availability from another pair.
8. `report comparison` works from retained comparison artifacts without a renderer and does not recompute a metric.
9. `report --manifest`, `regions --manifest`, `--default-pair`, and image-list substitutes are rejected by the lab parser or validator with stable errors.
10. Capturing stdout separately from stderr yields one parseable result object and does not remove progress or alter endpoint behavior.
11. A document containing includes, environment substitution, expressions, shell commands, wildcard selectors, workflow edges, or optimizer instructions is rejected before endpoint execution.
12. The fixture set contains one ordinary 45-region document and one irregular explicit regions document. Neither depends on a review manifest or a universal appearance score.

## Laws

1. `regions.json` is the only geometry authority for `sample`.
2. A grid declaration is expanded once into ordinary regions; downstream endpoints do not interpret grids.
3. A source coordinate is never silently treated as a render coordinate.
4. A retained record is self-describing and immutable.
5. An unavailable measurement is never zero and never an absent row without a reason.
6. `run`, `sample`, `compare`, `inspect`, and `report` never call a downstream endpoint.
7. Lab reporting consumes comparisons; it does not consume review manifests.
8. The renderer parameter language remains the only renderer parameter DSL.
