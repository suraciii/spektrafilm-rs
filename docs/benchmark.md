# Appearance benchmark

The `spektrafilm-lab` instrument renders immutable evidence, compares versions, and creates an offline review. It reports color, tone, official LDR-FLIP, grain, halation, edge response, and numerical health by case and region. It preserves floating masters, full-resolution maps, curves, spectra, and per-seed observations.

- Compare versions: generate one bundle, run baseline and candidate targets, then use `compare`, `inspect`, and `report comparison --comparison DIRECTORY --out DIRECTORY`.
- Generate measurement geometry independently: use `spektrafilm-lab regions grid --id ID --rows N --columns N --origin X,Y --patch-size WIDTHxHEIGHT --gap X,Y --out REGIONS.json`. It does not read images or create a visual report.
- Prepared-image visual review, if used, is benchmark-owned. It is not registered as an instrument document and is not accepted by lab `sample`, `compare`, `inspect`, or `report comparison`.
- Review a small set of photos visually: build one offline page with `spektrafilm-bench report photos --manifest FILE --out DIRECTORY`. It shows no measurements, no scores, and no baseline, candidate, or reference judgment. See [Review a small set of photos](#review-a-small-set-of-photos).
- Check a new configuration: use `run --dry-run` before rendering. Read its grouped errors and retained renderer logs before changing the configuration.

The agent prepares the evidence and reads the measurements. The reviewer opens `index.html` to inspect full images and native crops. Baseline selection and calibrated policy budgets remain explicit decisions.

```bash
./target/release/spektrafilm-lab regions grid \
    --id chart-regions --rows 6 --columns 4 \
    --origin 0,0 --patch-size 8x8 --gap 2,2 \
    --out /tmp/spektrafilm-benchmark-regions.json
```

## Generate and compare suite evidence

Build the instrument. Generate fixtures outside the repository. The optional pinned Kodak DC50 RAW has its own attribution and license in the generated `provenance.json`; the repository does not distribute that original.

The generator exports the explicit built-in `Kodak Gold 200` look as portable `look.toml`. `color-tone.json` isolates color and tone by disabling spatial effects, stochastic effects, and grain. `appearance.json` preserves the look's effect controls and enables spatial and stochastic processing. Both configurations use manual exposure, a 35 mm film-format scale, the product readout illuminant with direct scan output, and encoded sRGB on `input > film > print > scan`. They preserve the supplied input resolution without preview reduction, automatic crop, or resize. Their preset paths resolve relative to each configuration file. The generated `provenance.json` retains the renderer and bundle file hashes.

The generated grain probe selects `procedural` in custom Noise mode with three explicit seeds. Suites generated before the engine rename still contain the rejected `v2` selector. Generating a new bundle must remain an explicit setup action. A version comparison must reuse the same exported preset and configuration bytes on both sides.

```bash
just build-f64
just build-lab
./target/release/spektrafilm-lab fixtures \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --out /tmp/spektrafilm-benchmark-fixtures --download-natural

./target/release/spektrafilm-lab validate --kind suite \
    --file /tmp/spektrafilm-benchmark-fixtures/complete.json
./target/release/spektrafilm-lab run suite \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --dry-run --out /tmp/spektrafilm-benchmark-preflight
./target/release/spektrafilm-lab run suite \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --out /tmp/spektrafilm-benchmark-baseline
./target/release/spektrafilm-lab run suite \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/candidate.json \
    --out /tmp/spektrafilm-benchmark-candidate
./target/release/spektrafilm-lab compare runs \
    --baseline /tmp/spektrafilm-benchmark-baseline \
    --candidate /tmp/spektrafilm-benchmark-candidate \
    --out /tmp/spektrafilm-benchmark-comparison
./target/release/spektrafilm-lab inspect \
    --comparison /tmp/spektrafilm-benchmark-comparison \
    --case color_chart --region skin --metric delta_e00.p95
./target/release/spektrafilm-lab report comparison \
    --comparison /tmp/spektrafilm-benchmark-comparison \
    --out /tmp/spektrafilm-benchmark-review
just benchmark-test
```

## Benchmark a local image

For a local image, supply a configuration JSON with the same fields as a suite case's `configuration`; its preset and parameter-file paths resolve relative to that JSON. Generate a target independently of fixtures to pin the renderer executable SHA-256, data directory, and backend. Input files, prepared crops, runs, and reports may live anywhere outside the repository. This executable example reuses the generated chart; replace its path and source declaration with your image's actual color interpretation.

```bash
cat > /tmp/spektrafilm-local-configuration.json <<'JSON'
{
  "film": "kodak_gold_200",
  "paper": "kodak_portra_endura",
  "route": "input > film > print > scan",
  "set": ["camera.auto_exposure=false"]
}
JSON
./target/release/spektrafilm-lab target \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-local-target.json
./target/release/spektrafilm-lab validate --kind configuration \
    --file /tmp/spektrafilm-local-configuration.json
./target/release/spektrafilm-lab run image \
    --input /tmp/spektrafilm-benchmark-fixtures/chart.png \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-local-configuration.json \
    --target /tmp/spektrafilm-local-target.json \
    --out /tmp/spektrafilm-local-run
./target/release/spektrafilm-lab compare pairs \
    --run /tmp/spektrafilm-local-run --against input \
    --out /tmp/spektrafilm-local-comparison
./target/release/spektrafilm-lab report comparison \
    --comparison /tmp/spektrafilm-local-comparison \
    --out /tmp/spektrafilm-local-review
```

## Use the standard bundle

Use the standard bundle to run a local raster without authoring simulation settings. This example generates one bundle in a new directory outside the repository and reuses its chart as a declared sRGB raster. For a photograph, replace `LOCAL_IMAGE` and both source declaration flags with the image's actual color interpretation. Set `CANDIDATE_RENDERER` to the path of an already-built renderer for the version to compare. Both targets use the same data directory; an intentional data revision must remain a declared comparison condition. Every output destination must be new.

The exported look is a preset document of the renderer that generated it. A candidate renderer must be able to read that document. A renderer that predates a preset-schema field rejects the bundle with `invalid look`; the observed message for such a mismatch is `unknown field`, followed by the field name. Generate the bundle with the renderer that will read it, and treat a regenerated bundle as a declared comparison condition rather than a silent substitution.

```bash
just build-f64
just build-lab
: "${CANDIDATE_RENDERER:?Set CANDIDATE_RENDERER to the candidate renderer executable}"
./target/release/spektrafilm-lab fixtures \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --out /tmp/spektrafilm-standard-bundle

./target/release/spektrafilm-lab validate --kind configuration \
    --file /tmp/spektrafilm-standard-bundle/color-tone.json
./target/release/spektrafilm-lab validate --kind configuration \
    --file /tmp/spektrafilm-standard-bundle/appearance.json
./target/release/spektrafilm-lab target \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-standard-baseline-target.json

LOCAL_IMAGE=/tmp/spektrafilm-standard-bundle/chart.png
./target/release/spektrafilm-lab run image \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/color-tone.json \
    --target /tmp/spektrafilm-standard-baseline-target.json \
    --seed 0 --out /tmp/spektrafilm-standard-color-tone
./target/release/spektrafilm-lab run image \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/appearance.json \
    --target /tmp/spektrafilm-standard-baseline-target.json \
    --seed 0 --seed 42 --seed 99 \
    --out /tmp/spektrafilm-standard-appearance-baseline

./target/release/spektrafilm-lab target \
    --renderer "$CANDIDATE_RENDERER" --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-standard-candidate-target.json
./target/release/spektrafilm-lab run image \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/appearance.json \
    --target /tmp/spektrafilm-standard-candidate-target.json \
    --seed 0 --seed 42 --seed 99 \
    --out /tmp/spektrafilm-standard-appearance-candidate
./target/release/spektrafilm-lab compare runs \
    --baseline /tmp/spektrafilm-standard-appearance-baseline \
    --candidate /tmp/spektrafilm-standard-appearance-candidate \
    --out /tmp/spektrafilm-standard-comparison
./target/release/spektrafilm-lab inspect \
    --comparison /tmp/spektrafilm-standard-comparison --case image
./target/release/spektrafilm-lab report comparison \
    --comparison /tmp/spektrafilm-standard-comparison \
    --out /tmp/spektrafilm-standard-review
```

Open `/tmp/spektrafilm-standard-review/index.html` to review full images and native detail crops. A single seed may provide image-review evidence. It must not establish statistical grain acceptance. The standard look is an end-to-end rendering convention. It must not claim physical film fidelity.

## Preflight a run

Add `--dry-run` to either run form with a new destination to resolve every selected case and seed without decoding or rendering. It retains `preflight.json`, configurations, and renderer logs; successful preflight reports `execution_status: validated`. Image decoding, device availability, observed geometry, and numerical results remain unchecked. Preflight is separate from completed render evidence and cannot be compared as a run. Ordinary execution repeats the same configuration checks.

## Inspect a comparison

Use `inspect` to read retained measurements and `report comparison` to publish the offline
retained-comparison page; neither command recalculates metrics.

`case_conditions` exposes each selected case and seed's retained `run_evidence` by role. Read its input and target identities, actual commands, preparation declaration, resolved configuration links, and any retained observed exposure before interpreting color differences. Ordinary suite/image runs have no native observation capture: exposure and prepared print normalization remain unavailable unless already retained in supported historical evidence. An enabled configuration flag is not a measured gain, and inspection must not rerender to obtain one.

For each selected region, `region_readouts` exposes both sides' Lab/LCh centers and a signed three-component Lab displacement, ΔL*, ΔC*, Δh, ΔE76, ΔE00, and equal-L ΔE00 where eligible. Read `estimator` before comparing values: `bounded_lab_channel_median` and `unclipped_linear_srgb_mean` are different statistics. Repeat region selectors, for example `--region A2 --region C4 --region F2 --region D4`, to read the four TE226 focus patches when those IDs are declared. Metric selectors narrow scalar rows, not these regional records. Historical comparisons report `not_retained`; create a new comparison from retained runs to obtain the new readouts without rerendering.

For a replay across renderer versions, compare the resolved values as well as the parameter-file hashes. Legacy `--params` supplies a baseline; it does not protect profile-owned values from stock initialization. Pin required inherited values with explicit `--set` sources under the [CLI composition rules](../specs/cli/spec.md#composition). Check native output or stage agreement before treating a changed result as a material-model difference.

## Native-stage research boundary

The [native observation contract](../specs/instrument/native-observations.md) separates dedicated research from ordinary benchmark rendering. Ordinary `run suite` and `run image`, including preflight and source-only execution, expose no optional native capture or implicit replacement. New runs retain explicit absence. Sampling, comparison, inspection, and reporting do not rerender, reconstruct missing stages, or infer applied exposure and prepared print normalization from authored settings.

Dedicated [`experiment run`](../specs/instrument/experiment/spec.md) and [`measure run`](measurement.md#measure-run) retain native observations by attaching the common core observer to their actual CPU f64 execution, not by passing capture flags to `process`. Their replay records identify the owning instrument invocation, dependencies, seed, and observer configuration, never fabricated process argv. Experimental export uses positional bundle input:

```bash
spektrafilm-lab experiment run --experiment experiment.json --out research-run
spektrafilm-lab experiment export research-run/bundle --out research-arrays
```

An experiment document declares input identity, parameter sources, and matching case/grid. Its shared process handler attaches a private instrument observer to the same render that supplies the image. `command.json` records the owning experiment invocation, executable and declaration hashes, seed, and native observer configuration; it is not a process command with a retired capture flag. A bundle contains the native buffers actually produced by that execution, with geometry, domain, channel axis, precision, hashes, and explicit skipped-stage reasons. The scientific wire filename remains `stage-evidence.json`. Experiment observation requires the complete film-and-print scan route; measurement may deliberately observe narrower ranges. GPU and preview requests fail rather than changing execution or metering semantics. Human Boolean `--debug` does not capture arrays. A `.sfstage` film/print artifact is a resumable state, not an observation bundle.

Existing `sample --from-evidence` and `sample --from-run` remain research readers for supported retained bundles and historical captured runs. They reject absent observations without rendering or resuming a stage artifact. A newly rendered ordinary run is not a source of native buffers merely because its final image exists.

The report opens the selected image pair, a short condition summary, and the available regional evidence. Selecting a region keeps other regions, failed cases, and worst regressions reachable. The page never calculates measurements or matches exposure. A visual pair without measured evidence shows that limitation instead of borrowing another pair's score. Unknown reference processing remains visual-only; stage provenance does not qualify that reference for numerical distances.

`stage_readouts` exposes retained native channel means, minima, and maxima with channel order, units, precision, and buffer links when supported evidence and a proven region mapping exist. For new ordinary suite/image runs, native observations remain explicitly absent; neither a later comparison nor a report obtains them by rerendering. Historical uncaptured stages and grids without a proven mapping remain unavailable. The report places retained color coordinates, signed displacements, and native-stage tables inside region details, keeping full precision in a disclosure. CLI and report project the same retained comparison without recomputing measurements or adjusting exposure; native buffers are downloadable evidence, not page imagery.

Provide reference `regions` when a visual-only reference needs patch crops. These coordinates describe the reference image, not the simulation. Without reference geometry, its patch crop remains unavailable. The report folds the complete measurement table below the image review.

For a processing-qualified regional reference, author case `region_pairs` to name two existing regions and protocol `color.hue_chroma_floor` to select hue diagnostics. `inspect` exposes ordinary patch DE00, the separately named equal-L diagnostic, signed hue shift, absolute hue error, and pair separation. A low-chroma patch has an unavailable hue value. Color, neutral, and endpoint summaries keep their member regions and tails separate. Exact field syntax is in the [suite interface](../specs/instrument/spec.md#suite-document); metric identities and interpretation limits are in the [regional protocol](../specs/benchmark/metrics/spec.md#regional-color-diagnosis).

## Use camera RAW and prepared inputs

Use `--input-kind raw` for camera RAW and omit the raster color flags. Set dedicated RAW options in the configuration's `raw` object. The run retains the real renderer's decoded input with the same crop and resize as simulation, along with the original source dimensions. Set simulation crop and resize through the configuration's existing `set_files` or `set` carriers; fixed review regions use completed-render coordinates. An externally prepared crop is a separate input; optional `--preparation FILE` records its original path, SHA-256, and descriptive operations without executing them.

## Retain a local reference

Add `--reference FILE --reference-info FILE` to retain a local reference. Reference info uses the existing reference declaration without `path` and `sha256`; it must declare color, provenance, processing, and scope. Then select `spektrafilm-lab compare pairs --run DIRECTORY --against reference` or `--against both`. Input/simulation reports describe processing changes; reference/simulation reports describe applicable distances to the reference. Visual and regional references remain separate native views, with explicit limits on pixelwise measurements. Both-pair reports open reference/simulation and provide a selector for input/simulation. No single-run comparison claims acceptance.

## Preserve evidence integrity

The decoded input route fixes its taps to RGB input/output, so simulation diagnostic taps cannot relabel film density as source RGB.

## Review evidence offline

The benchmark report reads retained comparison artifacts only. It does not accept a prepared-image
review manifest; any benchmark-owned prepared-image viewer must use its own command and contract.

Case, pair, and region links preserve exact identities in the report fragment, for example `#case=color_chart&seed=0&pair=baseline%2Fcandidate&region=skin`. Opening or reloading a link selects the evidence and expands its requested detail offline. Detail buttons expose their names and expanded state to keyboard and browser agents. Invalid locations display an error instead of unrelated imagery.

## Prepared-image visual review

Prepared-image visual review is benchmark-owned. Its image inventory, pair selection, display conversion, captions, and browser state are not instrument documents and are not accepted by lab measurement or retained-comparison endpoints.

The benchmark may retain a separate viewer implementation during migration. Its command and validator are benchmark-owned; do not pass its manifest to lab `sample`, `compare`, `inspect`, or `report comparison`.

## Review a small set of photos

Use `spektrafilm-bench report photos --manifest FILE --out DIRECTORY` to build one offline visual report from a small photo manifest. The command is benchmark-owned. It is not a `spektrafilm-lab` endpoint, and the manifest is not an instrument document. Photo Report shows photographs and authored rectangular regions of interest (ROIs). It does not measure, score, rank, or recommend. It never labels one side a baseline, candidate, or reference.

Build the binary first:

```bash
cargo build --release -p spektrafilm-benchmark
```

Author a UTF-8 JSON manifest with `schema_version: 1`. It needs a nonempty `title`, a nonempty `photos` array, and for each photo a unique `id`, a relative `path`, and a nonempty `label`. `description`, `note`, `color_space`, `cctf_decoding`, and `rois` are optional. A photo `path` is relative to the manifest file, must not be absolute, and must not resolve outside the manifest directory through `..` or a symlink. The command rejects unknown fields, duplicate JSON keys, duplicate photo or ROI IDs, unsafe paths, unsupported color declarations, and ROI rectangles outside the decoded photo.

Each ROI is one named rectangle in the photo's native pixel coordinates: a unique `id`, a nonempty `label`, and `rect` as `[x, y, width, height]`. ROI IDs are scoped to their photo. A ROI is a detail view, not a measurement region, and it does not register one photo against another.

Declare `color_space` and `cctf_decoding` when a source is not encoded sRGB. Omitted declarations select the fixed display default `sRGB` with `cctf_decoding: true`, and the report shows that default. The supported values are `sRGB`, `Display P3`, `DCI-P3`, `Adobe RGB (1998)`, `ProPhoto RGB`, `ITU-R BT.2020`, `ACEScg`, and `ACES2065-1`. The generator reads only the manifest declaration and never a neighboring sidecar.

The number of photos selects the report mode. One photo produces a single-photo report. Two photos produce one comparison pair in manifest order. Three or more photos require a nonempty `comparisons` array, and each entry names two different photo IDs with `left` and `right`. A manifest with one or two photos must not provide `comparisons`. The generator creates only the declared pairs.

Publish the report to a new directory:

```bash
./target/release/spektrafilm-bench report photos \
    --manifest /tmp/photos/manifest.json \
    --out /tmp/photo-report
```

The published directory contains `index.html`, `report.json`, `viewer.js`, and one 16-bit sRGB display PNG per photo under `images/`. Open `index.html` to review the report. The complete directory is the shareable artifact, and the page works from the local filesystem, from static hosting, and after the original photos are removed. The page makes no network request. An existing destination is rejected without modification.

The command emits one JSON result on stdout. Progress and diagnostics use stderr. Status `2` reports an argument syntax error, status `1` reports a validation, decoding, conversion, publication, or source-integrity failure, and status `0` reports successful publication.

This example manifest declares one comparison pair with one shared ROI identity and one one-sided ROI:

```json
{
  "schema_version": 1,
  "title": "800T outdoor comparison",
  "description": "Original scan and simulation output.",
  "photos": [
    {
      "id": "scan",
      "path": "scan.jpg",
      "label": "Original scan",
      "note": "No additional sharpening.",
      "color_space": "sRGB",
      "cctf_decoding": true,
      "rois": [
        {"id": "face", "label": "Face", "rect": [100, 120, 200, 280], "note": "Face and forehead."}
      ]
    },
    {
      "id": "simulation",
      "path": "simulation.png",
      "label": "Simulation output",
      "note": "800T, seed 0.",
      "rois": [
        {"id": "face", "label": "Face", "rect": [90, 110, 190, 270]},
        {"id": "sky", "label": "Sky", "rect": [0, 0, 640, 240]}
      ]
    }
  ]
}
```

The page shows the full-image pair in manifest order with side-by-side and A/B alternation, plus fit, native-pixel, zoom, and fullscreen controls. For the selected pair it lists the union of both photos' ROI IDs in deterministic input order. An ROI identity declared on both photos is shared and shows two-sided crops; the report matches ROIs only by identity, never by label, rectangle, size, position, or filename. An identity declared on one photo only shows the available crop and an explicit `unavailable` counterpart. A shared ROI keeps each side's own rectangle and native dimensions, and the report never resizes, crops, registers, or exposure- or color-matches either photo. Selecting one ROI keeps the full-image view, the other pair, and the other ROIs available.

A single-photo report shows the full photo, its ROI list and selected crop when it declares ROIs, and no comparison, swap, or A/B controls. A multi-pair report adds a selector for exactly the declared pairs, and changing the pair replaces the complete pair view. The page keeps its current photo, pair, and ROI in a URL fragment, restores that state after reload, and shows an accessible error with a recovery action for an unknown or malformed location instead of displaying an unrelated photo or pair. All photo, pair, and ROI selectors are reachable from the keyboard, and narrow layouts need no horizontal scrolling.

`report.json` is the terminal machine-readable record. It identifies the report `mode` (`single_photo`, `two_photo`, or `requested_comparisons`), the photos in authored order, the generated pairs in authored or derived order, resolved ROI availability per pair, source path and SHA-256, decoded dimensions, the color and display declarations, and every output artifact with its SHA-256.

## Read command results and gates

Each parsed command emits one JSON result on stdout. Diagnostic messages use stderr. Status `2` indicates argument syntax errors; status `1` indicates validation, execution, publication, or requested gate failure. Use `schema --kind suite|target|policy|run|comparison` to discover carriers. `compare --policy FILE --gate` publishes the comparison before returning failure for a non-pass evaluation. The supplied exploratory policy must not qualify acceptance. A calibrated policy must bind the digest emitted by suite validation and supply reviewed calibration evidence.

Every result is complete JSON on stdout, and the retained artifact is its authoritative detail: read the fields you need from it, and use the `inspect` filters for exact case, region, and metric evidence; unknown selections report the available identities. Policy binding is mechanical: `validate --kind suite` prints the protocol digest that the policy must carry. A non-pass evaluation status can be read from the retained summary counts and the policy snapshot.

`run` and `compare` write each case-seed start and completion to stderr. Completion includes its status and elapsed seconds; the final summary reports completed and failed items and total elapsed seconds. Keep stderr separate when reading JSON stdout.

Every publication destination must be new. If a destination already exists, choose another destination or remove the existing one after reviewing its contents. A directory left without its terminal manifest is incomplete. A published failed manifest retains its completed items and errors, but does not qualify as a baseline. A gate failure preserves the completed comparison, so inspect it before retrying.

## Understand fixture coverage

The generated complete suite adds `final_appearance`, which renders the chart with the full-appearance configuration and seeds `[0,42,99]`. Acquiring the pinned natural scene also adds `natural_appearance`, which uses that scene's RAW declaration and review regions with the full-appearance configuration and the same seeds. Both appearance cases select numeric, color, tone, and FLIP measurements. Dedicated grain, halation, and edge probes retain their effect-specific settings. The quick suite and `natural.json` retain isolated color-and-tone scene coverage when the natural scene is acquired. Without `--natural-input` or `--download-natural`, the generated suites contain controlled probes only. The `complete.json` filename must not imply comprehensive stock coverage, calibrated acceptance, or full-resolution photographic review. The packaged scene covers shadows and texture; broader skin, foliage, mixed-light, and real film-scan coverage requires explicitly declared assets and regions. A caller must use representative local photographs for native-resolution spatial review. Synthetic probes and rendered reference fixtures must not establish physical film fidelity.

## See also

- [Benchmark specification](../specs/benchmark/spec.md)
- [Photo Report specification](../specs/benchmark/photo-report/spec.md)
- [Building](building.md)
- [CLI](cli.md)
- [RAW input](raw.md)
- [Parity evidence](parity.md)
