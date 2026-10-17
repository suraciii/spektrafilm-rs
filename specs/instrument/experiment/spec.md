# Experiment runs

`experiment run` publishes one native stage-evidence bundle for one declared experiment invocation, and `experiment export` converts an existing bundle's raw f64 buffers into `.npy` arrays. Together they are one endpoint in the Unix-style research pipe; they do not execute a downstream sample, compare, report, or optimization step.

Both commands are experimental: they carry no compatibility promise. See [command tiers](../spec.md#command-tiers).

The experiment document is a typed declaration carrier, not a second parameter language or workflow DSL. Renderer parameter sources retain the [process surface](../../cli/spec.md) syntax and precedence. Instrument fields describe the endpoint input and retained evidence only.

## Command surface

- `spektrafilm-lab experiment run` — render one declared experiment and retain its boundary values.
- `spektrafilm-lab experiment export` — convert a bundle's buffers into `.npy` arrays.

## Experiment document

`experiment run` reads one experiment document. It is an instrument-owned document: it names a case, declares a chart grid, and carries parameter values. It is not a benchmark suite and not a process job file.

- `schema_version` — `1`.
- `id` — a nonempty identifier matching `^[A-Za-z0-9][A-Za-z0-9_.-]*$`.
- `input` — the source image. `path` is a file path relative to the document or an absolute path; `sha256` is the SHA-256 of the file's bytes. A missing or changed file is a command failure.
- `chain` — the parameter chain. `--film`, `--paper`, `--preset`, `--params`, `--set`, `--route`, and `--scan-output` use the [process surface](../../cli/spec.md) names and precedence. The document is a source of these values, not a new parameter language. An omitted field takes the process default.
- `case` — the case whose regions the grid describes. `id` must name a case in the suite that `suite` names; `suite` is that suite's path, resolved relative to the document. The case must declare a nonempty `regions` array.
- `grid` — the patch grid the case's regions must reproduce. `rows` and `cols` are positive integers; `patch_px` and `gap_px` are nonnegative integers; `origin` is `[x, y]`; `patch` maps a region `id` to its `[row, col]` position, zero-based. A region matches when its `geometry` equals `origin + [col * (patch_px + gap_px), row * (patch_px + gap_px), patch_px, patch_px]` for the `[row, col]` that `patch` names. Every case region must appear in `patch`, and `patch` must not name an id the case does not declare. `rows` and `cols` must equal the largest declared row and column plus one.
- `seeds` — one or more unsigned 32-bit seeds. The default is `[0]`. `experiment run` must execute and retain one result for every declared seed. Until that multi-seed publication contract is implemented, the command must reject more than one seed rather than execute only the first seed while reporting the whole population.

The document must reject duplicate keys and unknown fields. A `--set` file path must resolve inside the document's directory tree; a path that escapes it is a command failure.

The case reference keeps the region vocabulary in the [benchmark](../../benchmark/spec.md) owner: the suite declares region ids, roles, and measurement families, and the grid check is what surfaces a disagreement between the document and the case.
`experiment run` is one endpoint invocation and currently publishes one retained result. Its `seeds` declaration must therefore contain exactly one seed; a document with more than one seed is rejected before rendering rather than executing only the first seed while reporting the whole population. A retained record must identify the seed that produced it; a report must not imply observations for a seed that was not executed.

The case and grid fields are validation inputs for this endpoint. They must not become a second region language. The suite case remains authoritative for region identity, role, measurement family, and coordinate-space semantics. `grid` only checks that a declared experiment arrangement agrees with the referenced case; it does not license geometry inference from image dimensions, labels, or colors. A future endpoint may derive a standalone `regions` document from an authored review grid, but `experiment` must not add another geometry expression or silently replace the case's regions.

The first implementation slice for this document must not publish one result with a report that lists unexecuted seeds.

## Invocation

```bash
spektrafilm-lab experiment run --experiment EXPERIMENT.json --out DIR \
    [--data-dir DIR] [--output-format FORMAT] [--force]
```

- `--experiment <PATH>` — the experiment document.
- `--out <DIR>` — the report destination. `run` writes the bundle, `resolved.json`, `command.json`, and `report.json` into it. `run` refuses to replace an existing report unless `--force` is given.
- `--data-dir` — otherwise `SPEKTRAFILM_DATA_DIR` or automatic discovery, as on the [process surface](../../cli/spec.md#data-directory-selection).
- `--output-format` — the image container written beside the bundle, as on the [process surface](../../cli/spec.md#export-and-execution-options). The default is `tiff`. The command writes the image into `--out` under the same replacement rule as the report.

`run` resolves the chain with the shared `process_params::resolve`, renders through the shared `process` entry with the CPU f64 backend, and publishes the bundle.

- `resolved.json` is the resolved chain of the executed run: profiles, effective parameters, input declaration, and edit feedback. It is the single record of what the run actually used.
- `command.json` is the full `process` argument vector, including the `--stage-evidence` destination and every `--set` source, so a retained run is reproducible.
- `report.json` records the experiment identity, the resolved chain's profile names and input declaration, the bundle manifest, the grid and case references, and the tool version.

`run` must not accept `--dry-run`: a resolved report without a bundle is not evidence. It must not render on a GPU backend, in preview mode, or through a route the bundle cannot describe; those are configuration-level failures before rendering under the [stage-evidence contract](../../cli/spec.md#stage-evidence).

## Bundle validation

`experiment export` validates a bundle against the [stage-evidence contract](../../cli/spec.md#stage-evidence) and rejects one it cannot consume:

- `schema_version` is `1` and `execution_status` is `completed`.
- `host_precision` is `f64`. The instrument is f64-only; an `f32` bundle is not convertible to the `.npy` the caller expects.
- Every `available` stage record names a buffer that exists, whose SHA-256 matches the record, and whose byte length equals `width * height * channels * 8`.
- Every sample is finite. A non-finite sample is a command failure, not a value to pass through.

The manifest is the authority for stage identity and layout. `export` must not infer a stage from a filename, a buffer length, or a default shape.

## Buffer conversion

```bash
spektrafilm-lab experiment export --stage-evidence DIR --out DIR [--stage ID]... [--force]
```

- `--stage-evidence <DIR>` — the bundle directory.
- `--out <DIR>` — the array destination. `export` refuses to replace an existing array unless `--force` is given.
- `--stage <ID>` — a stage to convert. Repeats. The default is every `available` stage in manifest order. An `ID` that the manifest does not list as `available` is a command failure naming the available stages.

Each converted array is written as `<stage-id>.npy`: C-order, little-endian `f64`, shape `[height, width, channels]`, one row per pixel, channels interleaved. The values are the buffer's bytes exactly — no scaling, no channel permutation, no clipping.

The record's `channel_order` is the axis declaration: `rgb_in` and `rgb_out` are `"RGB"`; `log_e_film`, `cmy_film`, `log_e_print`, and `cmy_print` are `"CMY"`. The converted array's header carries `'<f8'` and the shape only; the axis meaning stays the bundle's to state. `export` must not reorder channels to match a consumer's expectation, because a consumer that reads a `"CMY"` buffer as RGB misreads the data and a converter that "fixed" it would hide that.

`export` writes `export.json` beside the arrays, recording the bundle's manifest path, the converted stage IDs, each array's SHA-256 and shape, and the tool version.

## Determinism

A repeated `run` with identical inputs produces byte-identical `report.json`, `resolved.json`, and `command.json`, and a bundle whose buffers are byte-identical. A repeated `export` with identical inputs produces byte-identical arrays.

A bundle's buffers are the render's own values. `export` does not re-render, does not re-derive a stage from another stage, and does not average or resample.

## Boundaries

`experiment` does not measure, score, fit, or render a new image. Region means, color differences, and tail statistics belong to [benchmark](../../benchmark/spec.md). Review pages belong to the benchmark report and prepared-image review. Profile fitting belongs to [`fit`](../spec.md#commands).

`experiment` reads the one case it references and the grid it declares. It does not restate the region list, the roles, or the measurement families, and it does not read the rest of the suite.

## Acceptance

1. Run one experiment whose case grid matches. Confirm the bundle publishes, `resolved.json` carries the resolved chain, `command.json` carries the full argument vector, and `report.json` names the case and grid.
2. Repeat the same run. Confirm `report.json`, `resolved.json`, and `command.json` are byte-identical and every bundle buffer is byte-identical.
3. Run with a case region absent from `patch`, and with `patch` naming an id the case does not declare. Confirm both fail before rendering and leave no manifest.
4. Run with a region geometry that does not match the declared grid. Confirm it fails before rendering and leaves no manifest.
5. Run with `input.sha256` not matching the file. Confirm it fails before rendering.
6. Run with a `--set` file path outside the document's directory tree. Confirm it fails with an actionable message.
7. Run with a document that repeats a key or carries an unknown field. Confirm it fails before rendering.
8. Run with a configuration the bundle cannot describe, on each of: a GPU backend, preview mode, and a route outside the executed tap range. Confirm each fails before rendering and leaves no manifest.
9. Run with `--dry-run`. Confirm it is rejected as an unknown option and writes nothing.
10. Export a completed f64 bundle. Confirm every `available` stage becomes a `.npy` with shape `[height, width, 3]`, little-endian `<f8`, and bytes equal to the bundle buffer, and that `export.json` records each array's SHA-256.
11. Export a bundle whose `host_precision` is `f32`. Confirm it fails naming `f64` and writes no array.
12. Export a bundle whose buffer SHA-256 does not match its record. Confirm it fails and writes no array.
13. Export with `--stage` naming a stage the manifest does not carry at all. Confirm it fails naming the available stages.
14. Export with `--force` over a destination holding a byte-for-byte sentinel array. Confirm the sentinel is replaced and the new array is readable.
15. Export a bundle whose manifest marks one stage `unavailable`. Confirm the bundle is rejected as incomplete, no array is written, and the reason is reported.
16. Read an exported `rgb_out` array and a `log_e_film` array from the same bundle. Confirm the first is `"RGB"` and the second is `"CMY"` in the manifest, and that neither array's channel order was rewritten.
