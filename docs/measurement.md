# Measurement

`spektrafilm-lab` reads the executed chain and reports what it produced. Four commands carry
the measurement surface: `measure run` measures the chain on a declared stimulus set,
`sample` reads declared regions from an image or a retained stage capture, `compare readings`
matches two retained measurements, and `verify` re-resolves a recorded run. The commands exist
only in the instrument binary and run at f64 reference precision. They compute values and write
evidence; they do not fit parameters, render images, or judge a result. See the [measurement
contract](../specs/instrument/spec.md#measurement) for the requirements.

The [native observation contract](../specs/instrument/native-observations.md) separates dedicated measurement and experiment endpoints from ordinary rendering. They attach the common core observer to their actual execution and own observation retention; they do not request capture through `process` flags. Measurement records retain their replayable declaration and actual observer entry, collection boundary, named taps, schema, and CPU f64 precision. Ordinary `process`, `run suite`, and `run image` offer no optional native capture or implicit replacement. Scientific bundles remain explicit research inputs, not resumable `.sfstage` states.

## The point-wise requirement

A reading is defined only for a point-wise chain. An injected numeric sample has no
neighborhood, so a stage that mixes samples, measures the whole image, or draws stochastic
values after the entry would describe a different signal than the measured one. Halation,
glare, lens blur, automatic camera exposure, and grain are such stages.

`measure run` resolves the chain first and fails before executing when one of them executes
inside the measured range: at or after the boundary `--enter` injects the stimulus at, and at
or before the last stage `--read` asks for. An operation the range never reaches cannot change
a reading, and a stage that does not execute at all cannot either, so neither is named.

Which stages execute depends on the path the range takes. The whole chain — the stimulus
entering at `rgb_in` and the last reading at `rgb_out` — runs the route's own stage list:
`input` runs neither the film nor the print stage, the convert routes build the film density by
inverting the scan model instead of metering and developing it, `io.scan_film` scans the
developed film directly instead of printing it first, and the scanner runs on every route but
`input`. A narrower range walks the chain from the entry to the last reading instead, and the
stages on that path execute whatever the route says; only the metering stays skipped on the
convert routes, and `io.scan_film` still drops the print stage. The failure list is the
resolved chain's own state either way, so it names what the run would actually execute.

The failure prints one line per offender — `path=value  ->  pass --set path=fixed`, the
resolved value on the left and the value that disables it on the right. The
[worked example](#worked-example) shows the whole report. The list is the chain's own state
after resolution, so it names the stages the preset, the profile, and your `--set` edits left
active.

The automatic `standard` negative conversion also measures the frame. A range that
reaches its RGB output must select a declared fixed conversion style instead.
Earlier density and exposure boundaries remain measurable. The instrument does
not treat its point array as a photograph for automatic setup.

The command does not disable a stage on your behalf. Pass the `--set` edits it names.

## `measure run`

```bash
./target/release/spektrafilm-lab measure run \
    --stimuli points.json --enter log_e_film \
    --read log_e_film,cmy_film,rgb_out \
    --preset "Kodak Portra 800" \
    --set film_render.halation.active=false \
    --set film_render.dir_couplers.active=false \
    --set film_render.grain.active=false \
    --set print_render.glare.active=false \
    --set scanner.unsharp_mask=[0,0] \
    --data-dir data --out run-1
```

`--enter` declares the chain boundary the stimulus enters at: `spectra`, `rgb_in`,
`log_e_film`, or `cmy_film`. `--read` selects stages from the tap vocabulary (`rgb_in`,
`log_e_film`, `cmy_film`, `log_e_print`, `cmy_print`, `rgb_out`); omit it to read every
available stage. A stimulus enters through the same `taps.inject` control the user binaries
expose, and the run records the chain it resolved, the values it read, and the stage
evidence it read them from. The chain options are the ones `process` takes: `--preset`, `--film`,
`--paper`, `--set`, `--data-dir`, plus `--route` for the workflow route and `--scan-output` for
the film scanner mode. A chain run resolves its own seed from the parameter surface, so the
instrument records that seed in the measurement's identity instead of accepting a `--seed`
option on this form. `--dry-run` resolves the chain and writes `run.json` with the identity
record only.

### The stimulus document

`--stimuli` names a JSON manifest. The arrays live beside it as `.npy` or `.npz` files.

A `spectra` entry reads a measured-spectra manifest. It declares the wavelength grid, the
valid band, the per-sample reflectance arrays, the sample labels, and the scene illuminant
pair. The grid must match the film profile's own wavelength grid, and the manifest itself
declares the scene color space, the transfer state of the input values, and the illuminant
pair as `condition`:

```json
{
  "schema_version": 1,
  "name": "portra-800-spectra",
  "color_space": "sRGB",
  "cctf_decoding": true,
  "condition": {"reference": "D65", "target": "Tungsten 3200K"},
  "grid_nm": {"start": 380.0, "end": 780.0, "step": 5.0},
  "valid_band": [380.0, 780.0],
  "reflectances": {"path": "reflectances.npy"},
  "samples": [
    {"id": "patch-01", "source": "it8", "partition": "color", "row": 0}
  ]
}
```

`color_space` names the scene color space the values are read in and `cctf_decoding` declares
whether the chain must decode them, exactly as `--input-cctf-decoding` declares it for a raster.
`condition.reference` names the film reference illuminant and `condition.target` the scene
illuminant that produced the input.

Every other entry reads a points manifest: an `N` by 3 array of values at that boundary. The
`domain` must match the entry (`scene_rgb` for `rgb_in`, `film_log_e` for `log_e_film`,
`film_density` for `cmy_film`).

A `scene_rgb` manifest declares its `color_space` and its `cctf_decoding` state, exactly as a
raster input to `process` does. Declare `true` when the array carries transfer-encoded values
that the chain must decode, and `false` when it already carries transfer-decoded values; the
chain then treats them exactly as it treats a raster:

```json
{
  "schema_version": 1,
  "domain": "scene_rgb",
  "color_space": "sRGB",
  "cctf_decoding": true,
  "points": {"path": "scene.npy"},
  "samples": [
    {"id": "p00", "source": "chart", "partition": "all", "row": 0}
  ]
}
```

A film-domain manifest declares neither, because its values are already film-domain numbers:

```json
{
  "schema_version": 1,
  "domain": "film_log_e",
  "points": {"path": "log_e.npy"},
  "samples": [
    {"id": "patch-01", "source": "it8", "partition": "color", "row": 0}
  ]
}
```

Each sample carries a stable `id`, a `source` and `partition` label, and an optional `row`
that defaults to its position in the `samples` array. Those labels become the groups the
report summarizes.

### Artifacts

A run writes `run.json` and `readings.csv` into `--out`. It writes nothing when the
measurement does not complete, and it never replaces an existing artifact. An executed run
also retains the stage evidence it read under `evidence/`.

A measurement executes its stimulus as a one-row image, and a point-wise chain's values do
not depend on that shape. What they do depend on is which spatial stages execute: the DIR
couplers diffuse between neighbouring samples, so a reading at or after `cmy_film` is not
point-wise while `film_render.dir_couplers.active` is true, and `log_e_film`, taken before
development, is unaffected by them. That is why the guard is measured per stage rather than
named after a flag.

`run.json` carries the declared inputs and their hashes, the resolved chain identity,
the entry boundary, the read stages, the stimulus identity, and the group summaries.
The `command` record retains the owning replayable `spektrafilm-lab measure run`
argv, executable identity, executed seed, and the dependency identities already used by
`verify`: parameter-source hashes, stimulus manifest/array hashes, profile identities,
and the data inventory. It reuses the canonical `declaration`, rather than serializing
a resolved snapshot as a new input. The `observer` record identifies the actual named
taps, entry and collection boundary, CPU f64 execution, native schema version, and
`directory: "evidence"` relative to `run.json`. Replay may change `--out` to a fresh
destination; canonical declaration paths allow another working directory. There is
no fabricated `process` capture argv.
The measurement seed remains instrument-owned and defaults to zero; `random_seed`
is not a settable parameter source and measurement adds no seed flag. Named user
presets are retained as the selected file's canonical path and content hash, while
built-in selections are qualified with `built-in:` so a different user library
cannot shadow them.
`readings.csv` carries one row per sample per available stage: the sample identity, its group,
the stage, channel order, the three channel values, the unit, and a validity flag. A sample
whose values include a non-finite number is invalid and reports an empty field; the command
does not clip, extrapolate, or repair it.

### The command result

Every successful measurement command prints one JSON object on stdout with `schema_version`,
`command`, `execution_status`, and `artifact`, the record the command published. Human
messages, progress, and errors use stderr, so the two streams stay separable. A failed
command prints its human text on stderr and a result object on stdout whose
`execution_status` is `failed` and whose `errors` name the failure.

## `sample`

```bash
./target/release/spektrafilm-lab sample \
    --source chart.tif --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --regions regions.json --out sample-1
```

`--source` declares a raster image and requires the same input declaration as `process`: a source color space and a transfer state. `--from-evidence DIRECTORY --stage STAGE` remains a research reader for one retained native observation bundle; its manifest supplies coordinate space, channel order, units, color space, and transfer state. `--from-run DIRECTORY|RUN.JSON --case ID --seed UINT32 --stage STAGE` remains a reader for an exact completed historical case/seed row with indexed available observations. The artifact index and case manifest reference must agree on path and digest; unsafe paths, missing or failed rows, absent observations, missing stages, malformed buffers, and cross-seed references fail without fallback. New ordinary suite/image rows explicitly retain observation absence. Sampling never renders, reconstructs a missing stage, enables capture, or resumes a `.sfstage` state to fill that absence.

An image source reports its readings under the stage name `source`, with the domain
`source_rgb` and the declared color space. An evidence source reports the bundle's own stage
identifier and declaration. When the bundle stage declares transfer-encoded values
(`cctf_decoding: true`), the reading decodes them with that stage's own declared color space
and records `cctf_decoding: false` in its stage record, because the values the reading stores
are decoded; the `source` block keeps the bundle's own declaration verbatim, so
`source.cctf_decoding` still reads `true`. A stage that declares no color space or transfer
state — the density and exposure stages do not, since their values carry no transfer encoding
— records `null` for both rather than a state the bundle never declared.

### The regions document

`--regions` declares the sample geometry with stable identities:

```json
{
  "schema_version": 1,
  "id": "chart-regions",
  "name": "chart-plan",
  "regions": [
    {"id": "patch-01", "label": "skin", "source": "it8", "partition": "color",
     "role": "skin", "measurements": ["color", "tone"],
     "geometry": {"x": 64, "y": 32, "width": 8, "height": 8, "space": "source"}}
  ]
}
```

Each region carries a stable `id`, an optional `label`, optional `source` and `partition`
group labels, a `role`, and the `measurements` families it selects. The role is one of `skin`,
`neutral`, `shadow`, `highlight`, `color`, `flat_field`, `edge`, `texture`, or `other`. The
`geometry` names a half-open rectangle with required `x`, `y`, `width`, `height`, and `space` fields. `width` and `height` are positive. `space` must be `source`, `render`, `source_pixels`, `working_pixels`, `film_frame_pixels`, `grain_aperture_pixels`, or `output_pixels`. `source` names the declared source image's pixels and `render` names the coordinates of a completed simulation; a native stage value names that exact stage coordinate grid. Sampling resolves the producer mapping exactly before reading and rejects a mismatch; it does not resize, crop, pad, translate, or clamp. Identities are unique within the document, and `full_image` stays reserved for the whole declared source. Regions may overlap: a reading carries its own identity.

The same explicit regions document can be used for standalone retained-stage sampling, while suite and reference image consumers currently accept only `source` and `render` geometry. Native spaces remain valid in the shared schema and for standalone sampling only when the selected producer publishes the exact matching coordinate space; embedding native geometry in a suite or reference is rejected with `unsupported_coordinate_space` until that consumer mapping exists. To create ordinary rectangular geometry independently, use `spektrafilm-lab regions grid`; downstream commands consume the resulting explicit regions without interpreting a grid declaration.
A region window that leaves the source bounds fails with the region identity. The command does not clamp, pad, extrapolate, or repair a region, and it does not resize, re-encode, or convert the source beyond the declared input declaration.

## `compare readings`

```bash
./target/release/spektrafilm-lab compare readings \
    --baseline run-1 --candidate run-2 \
    --reference run-ref --out compare-1
```

Readings match by sample identity and stage identity. An incompatible identity set is
rejected, and the mismatch is named; a sample or stage that only one side carries is
reported as unmatched, and the rest still publishes.

An identity difference between the two runs is evidence rather than an error: the command
reports it beside the numeric deltas and does not refuse the pair. That is how a retained
reference reading makes chain drift visible.

`--reference-run` adds a third retained reading and compares it under its own role: the
primary comparison is `candidate_minus_baseline`, and the reference adds
`candidate_minus_reference` and `baseline_minus_reference`. Each comparison decides its own
availability from its own two sides, so a stage that joins for one comparison is not
reported for another on its behalf. Differences that name only a value's group label
(`source`, `partition`) are reported as identity differences and the comparison still
publishes; a difference in the value's own declaration (domain, units, channel order) is an
incompatible identity set and fails.

A stage that declares no color space reports its color distance as unavailable with a
reason instead of a number, and an unavailable distance is never written as zero:

```json
{"pair": "candidate_minus_baseline", "stage": "log_e_film",
 "metric": "delta_e_2000", "reason": "no declared color space"}
```

`comparison.json` carries the side records, the identity differences, the unmatched readings,
the joined stages, one summary group per comparison per channel, the color-distance summaries
with their clipped count, and the unavailable records. `deltas.csv` carries one row per joined
sample and stage per comparison, with the comparison in its `pair` column, the subtrahend
side in `b0..b2` and the minuend side in `c0..c2`, the deltas, the color distance, the
validity flag, and the count of channels that fell outside the display gamut. A color
distance is written only where both sides declare a color space, so a missing value is an
empty field rather than a zero, and a pair whose two sides declare different color
interpretations reports its channel deltas empty and its validity `false`, exactly as the same
comparison's record reports that metric unavailable.

## `verify`

```bash
./target/release/spektrafilm-lab verify --record run-1 --data-dir data
```

The command re-resolves the declaration recorded in `run-1/run.json` against the data
directory you name, then re-hashes the renderer executable, the data inventory, the
profiles, and the retained stage buffers. It reports every difference from the recorded
identity. It renders nothing, rewrites nothing, and judges nothing: it exits 0 whenever it
could read the record and re-resolve the chain, because a difference is evidence rather than
a failure.

The command writes no artifact. It prints its full report object on stdout and then the usual
command result, so a verification cannot overwrite the record it verifies.

This is how a guard run stays useful. Retain a measurement of a frozen stimulus set, then
verify it later to see whether the executable, the data, the profiles, or the parameters

## Worked example

Measure Kodak Portra 800 through the default print-and-scan route. The preset enables
halation, DIR couplers, grain, print glare, and scanner unsharp masking, so the first attempt
fails before it executes:

```bash
./target/release/spektrafilm-lab measure run \
    --stimuli log_e.json --enter log_e_film \
    --preset "Kodak Portra 800" \
    --data-dir data --out run-1
```

```text
the resolved chain is not point-wise, so a reading would describe a different signal than the measured one:
film_render.dir_couplers.active=true  ->  pass --set film_render.dir_couplers.active=false
film_render.grain.active=true  ->  pass --set film_render.grain.active=false
print_render.glare.active=true  ->  pass --set print_render.glare.active=false
scanner.unsharp_mask=[0.7,0.7]  ->  pass --set scanner.unsharp_mask=[0,0]
```

Halation is enabled and is not named: the stimulus enters at `log_e_film`, which the halation
stage has already produced, so the injected values never pass through it. The DIR couplers,
the grain, the viewing glare, and the scanner's unsharp mask all sit after that boundary and
do. The list names exactly the operations the measured range can reach.

Disable the stages the failure names, and the same run completes:

```bash
./target/release/spektrafilm-lab measure run \
    --stimuli log_e.json --enter log_e_film \
    --preset "Kodak Portra 800" \
    --set film_render.grain.active=false \
    --set print_render.glare.active=false \
    --set scanner.unsharp_mask=[0,0] \
    --data-dir data --out run-1
```

```text
Measurement: run-1
Samples: 24 (0 invalid)
Entry: log_e_film
rgb_in: not_applicable (outside_executed_tap_range)
log_e_film: available
cmy_film: available
log_e_print: available
cmy_print: available
rgb_out: available
```

The entry stage is `available`, because the run executes the range the entry declares; the
stages before it, which the route never executed for this measurement, report
`not_applicable`. Retain the run, then verify it after a rebuild or a data change:

```bash
./target/release/spektrafilm-lab verify --record run-1 --data-dir data
```

## See also

- [Measurement contract](../specs/instrument/spec.md#measurement)
- [Lab CLI design](../specs/instrument/design.md)
- [CLI processing](cli.md)
- [Native observations](../specs/instrument/native-observations.md)
- [Dedicated experiment and positional bundle export](../specs/instrument/experiment/spec.md)
- [Resumable film and print artifacts](../specs/cli/stage-artifacts.md)
- [Human debug diagnostics](../specs/cli/spec.md#debug-diagnostics)
- [Profile fitting](profile-fitting.md)
- [Building](building.md)
