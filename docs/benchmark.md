# Appearance benchmark

The `spektrafilm-lab` instrument renders immutable evidence, compares versions, and creates an offline review. It reports color, tone, official LDR-FLIP, grain, halation, edge response, and numerical health by case and region. It preserves floating masters, full-resolution maps, curves, spectra, and per-seed observations.

The [benchmark specification](../specs/benchmark/spec.md) defines reference scopes and qualification rules.

Choose the path for the task:

- Compare versions: generate one bundle, run baseline and candidate targets with that bundle, then use `compare`, `inspect`, and `report --comparison`.
- Review one image: use `run --input`, then `compare --run --against input` and `report --comparison`. Add a declared reference to review against it.
- Review prepared images without a simulation run: use `report --manifest` with a set manifest. It accepts Chinese captions and a default pair; a shared grid supports linked patch inspection while explicit regions select independently per side. `regions` turns a grid set's declared measurement intent into a region array.
- Check a new configuration: use `run --dry-run` before rendering. Read its grouped errors and retained renderer logs before changing the configuration.

The agent prepares the evidence and reads the measurements. The reviewer opens `index.html` to inspect full images and native crops. Baseline selection and calibrated policy budgets remain explicit decisions.

## Generate and compare suite evidence

Build the instrument. Generate fixtures outside the repository. The optional pinned Kodak DC50 RAW has its own attribution and license in the generated `provenance.json`; the repository does not distribute that original.

The generator exports the explicit built-in `Kodak Gold 200` look as portable `look.toml`. `color-tone.json` isolates color and tone by disabling spatial effects, stochastic effects, and grain. `appearance.json` preserves the look's effect controls and enables spatial and stochastic processing. Both configurations use manual exposure, a 35 mm film-format scale, neutral scanner finishing, and encoded sRGB on `input > film > print > scan`. They preserve the supplied input resolution without preview reduction, automatic crop, or resize. Their preset paths resolve relative to each configuration file. The generated `provenance.json` retains the renderer and bundle file hashes.

The generated grain probe selects `procedural` in custom Noise mode with three explicit seeds. Suites generated before the engine rename still contain the rejected `v2` selector. Generating a new bundle must remain an explicit setup action. A version comparison must reuse the same exported preset and configuration bytes on both sides.

```bash
just build-f64
just build-lab
./target/release/spektrafilm-lab fixtures \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --out /tmp/spektrafilm-benchmark-fixtures --download-natural

./target/release/spektrafilm-lab validate --kind suite \
    --file /tmp/spektrafilm-benchmark-fixtures/complete.json
./target/release/spektrafilm-lab run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --dry-run --out /tmp/spektrafilm-benchmark-preflight
./target/release/spektrafilm-lab run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --out /tmp/spektrafilm-benchmark-baseline
./target/release/spektrafilm-lab run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/candidate.json \
    --out /tmp/spektrafilm-benchmark-candidate
./target/release/spektrafilm-lab compare \
    --baseline /tmp/spektrafilm-benchmark-baseline \
    --candidate /tmp/spektrafilm-benchmark-candidate \
    --out /tmp/spektrafilm-benchmark-comparison
./target/release/spektrafilm-lab inspect \
    --comparison /tmp/spektrafilm-benchmark-comparison \
    --case color_chart --region skin --metric delta_e00.p95
./target/release/spektrafilm-lab report \
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
./target/release/spektrafilm-lab run \
    --input /tmp/spektrafilm-benchmark-fixtures/chart.png \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-local-configuration.json \
    --target /tmp/spektrafilm-local-target.json \
    --out /tmp/spektrafilm-local-run
./target/release/spektrafilm-lab compare \
    --run /tmp/spektrafilm-local-run --against input \
    --out /tmp/spektrafilm-local-comparison
./target/release/spektrafilm-lab report \
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
./target/release/spektrafilm-lab run \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/color-tone.json \
    --target /tmp/spektrafilm-standard-baseline-target.json \
    --seed 0 --out /tmp/spektrafilm-standard-color-tone
./target/release/spektrafilm-lab run \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/appearance.json \
    --target /tmp/spektrafilm-standard-baseline-target.json \
    --seed 0 --seed 42 --seed 99 \
    --out /tmp/spektrafilm-standard-appearance-baseline

./target/release/spektrafilm-lab target \
    --renderer "$CANDIDATE_RENDERER" --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-standard-candidate-target.json
./target/release/spektrafilm-lab run \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/appearance.json \
    --target /tmp/spektrafilm-standard-candidate-target.json \
    --seed 0 --seed 42 --seed 99 \
    --out /tmp/spektrafilm-standard-appearance-candidate
./target/release/spektrafilm-lab compare \
    --baseline /tmp/spektrafilm-standard-appearance-baseline \
    --candidate /tmp/spektrafilm-standard-appearance-candidate \
    --out /tmp/spektrafilm-standard-comparison
./target/release/spektrafilm-lab inspect \
    --comparison /tmp/spektrafilm-standard-comparison --case image
./target/release/spektrafilm-lab report \
    --comparison /tmp/spektrafilm-standard-comparison \
    --out /tmp/spektrafilm-standard-review
```

Open `/tmp/spektrafilm-standard-review/index.html` to review full images and native detail crops. A single seed may provide image-review evidence. It must not establish statistical grain acceptance. The standard look is an end-to-end rendering convention. It must not claim physical film fidelity.

## Preflight a run

Add `--dry-run` to either run form with a new destination to resolve every selected case and seed without decoding or rendering. It retains `preflight.json`, configurations, and renderer logs; successful preflight reports `execution_status: validated`. Image decoding, device availability, observed geometry, and numerical results remain unchecked. Preflight is separate from completed render evidence and cannot be compared as a run. Ordinary execution repeats the same configuration checks.

## Inspect a comparison

Use `inspect` to read measurements instead of parsing `comparison.json`. Version comparisons store measurements on case records; single-run comparisons store them on pair records. `inspect` exposes both as one top-level `measurements` array with the same fields. Single-run rows also carry `pair_id`.

Repeat `--case`, `--region`, and `--metric` for exact selection; unknown selections report available identities. The compact records preserve units, domains, unavailable reasons, and evidence links relative to `artifact_base`. Per-seed/pair `measurements` and cross-seed `seed_measurements` remain separate. The retained comparison's execution and evaluation states remain visible, including failures; inspection never produces a universal appearance score.

`case_conditions` exposes each selected case and seed's retained `run_evidence` by role. Read its input and target identities, actual commands, preparation declaration, resolved configuration links, and observed exposure before interpreting color differences. Without requested stage capture, observed exposure and print normalization remain unavailable; an enabled configuration flag is not a measured gain.

For each selected region, `region_readouts` exposes both sides' Lab/LCh centers and a signed three-component Lab displacement, ΔL*, ΔC*, Δh, ΔE76, ΔE00, and equal-L ΔE00 where eligible. Read `estimator` before comparing values: `bounded_lab_channel_median` and `unclipped_linear_srgb_mean` are different statistics. Repeat region selectors, for example `--region A2 --region C4 --region F2 --region D4`, to read the four TE226 focus patches when those IDs are declared. Metric selectors narrow scalar rows, not these regional records. Historical comparisons report `not_retained`; create a new comparison from retained runs to obtain the new readouts without rerendering.

For a replay across renderer versions, compare the resolved values as well as the parameter-file hashes. Legacy `--params` supplies a baseline; it does not protect profile-owned values from stock initialization. Pin required inherited values with explicit `--set` sources under the [CLI composition rules](../specs/cli/spec.md#composition). Check native output or stage agreement before treating a changed result as a material-model difference.

## Collect native-stage diagnosis

Add `--stage-evidence` to a CPU run with `settings.preview_mode=false` to retain the renderer's native stage bundle. Capture rejects preview mode instead of changing its rendering or metering behavior. Ordinary crop and resize remain supported and carry their own stage geometry. This example reuses the generated quick suite and target:

```bash
./target/release/spektrafilm-lab run \
    --suite /tmp/spektrafilm-benchmark-fixtures/quick.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --case color_chart --stage-evidence \
    --out /tmp/spektrafilm-diagnostic-run
./target/release/spektrafilm-lab compare \
    --run /tmp/spektrafilm-diagnostic-run --against input \
    --out /tmp/spektrafilm-diagnostic-comparison
./target/release/spektrafilm-lab inspect \
    --comparison /tmp/spektrafilm-diagnostic-comparison --case color_chart
./target/release/spektrafilm-lab report \
    --comparison /tmp/spektrafilm-diagnostic-comparison \
    --out /tmp/spektrafilm-diagnostic-review
```

The bundle retains native RGB, film-layer exposure and developed channels, print-layer exposure and developed channels, and final RGB where the selected route executes those stages. Each buffer identifies its geometry, domain, storage layout, host precision, and hash. Skipped stages carry an explicit reason. Actual exposure and prepared print normalization come from the same renderer execution; the benchmark does not reconstruct them. This capture is opt-in, separate from ordinary telemetry, and does not change image parameters or output. A GPU target is not silently replaced by CPU execution. `run --dry-run --stage-evidence` records the request but cannot observe exposure or stage values.

The report opens the selected image pair, a short condition summary, and the available regional evidence. Selecting a region keeps other regions, failed cases, and worst regressions reachable. The page never calculates measurements or matches exposure. A visual pair without measured evidence shows that limitation instead of borrowing another pair's score. Unknown reference processing remains visual-only; stage provenance does not qualify that reference for numerical distances.

`stage_readouts` exposes each declared region's native channel mean, minimum, and maximum, with channel order, units, precision, and buffer links. Uncaptured stages and grids without a proven region mapping remain unavailable. The report places color coordinates, signed displacements, and native-stage tables inside each region's detail panel. Rounded tables keep full-precision records in a disclosure. Both CLI and report read the same retained comparison; neither recomputes measurements or adjusts exposure. Native buffers remain downloadable evidence, not page imagery.

Provide reference `regions` when a visual-only reference needs patch crops. These coordinates describe the reference image, not the simulation. Without reference geometry, its patch crop remains unavailable. The report folds the complete measurement table below the image review.

For a processing-qualified regional reference, author case `region_pairs` to name two existing regions and protocol `color.hue_chroma_floor` to select hue diagnostics. `inspect` exposes ordinary patch DE00, the separately named equal-L diagnostic, signed hue shift, absolute hue error, and pair separation. A low-chroma patch has an unavailable hue value. Color, neutral, and endpoint summaries keep their member regions and tails separate. Exact field syntax is in the [suite interface](../specs/instrument/spec.md#suite-document); metric identities and interpretation limits are in the [regional protocol](../specs/benchmark/metrics/spec.md#regional-color-diagnosis).

## Use camera RAW and prepared inputs

Use `--input-kind raw` for camera RAW and omit the raster color flags. Set dedicated RAW options in the configuration's `raw` object. The run retains the real renderer's decoded input with the same crop and resize as simulation, along with the original source dimensions. Set simulation crop and resize through the configuration's existing `set_files` or `set` carriers; fixed review regions use completed-render coordinates. An externally prepared crop is a separate input; optional `--preparation FILE` records its original path, SHA-256, and descriptive operations without executing them.

## Retain a local reference

Add `--reference FILE --reference-info FILE` to retain a local reference. Reference info uses the existing reference declaration without `path` and `sha256`; it must declare color, provenance, processing, and scope. Then select `compare --run DIRECTORY --against reference` or `--against both`. Input/simulation reports describe processing changes; reference/simulation reports describe applicable distances to the reference. Visual and regional references remain separate native views, with explicit limits on pixelwise measurements. Both-pair reports open reference/simulation and provide a selector for input/simulation. No single-run comparison claims acceptance.

## Preserve evidence integrity

The decoded input route fixes its taps to RGB input/output, so simulation diagnostic taps cannot relabel film density as source RGB.

## Review evidence offline

Open `/tmp/spektrafilm-benchmark-review/index.html`. The human review opens one case and seed at a time in request order and keeps every case, seed, failure, and artifact reachable through the selector and folded evidence. The first view shows the selected comparison pair, a short retained condition summary, and a small rounded full-image metric summary; sources, references, difference maps, curves, full measurements, execution settings, findings, and detail regions stay collapsed until needed. Fixed regions, automatic hotspots, lossless crops, synchronized pan and zoom, manual A/B alternation, and split view remain available for aligned pairs. Detail zoom supports 100%, 200%, and 400% in device pixels. Report generation reads retained evidence and needs neither the original input files nor the renderer.

The report opens in split mode: drag the divider on the image, use the range control, or move the focused divider with arrow keys (Shift for larger steps; Home/End for endpoints). Display copies use 16-bit RGB sRGB PNG without dither. Fit views resize floating samples before 16-bit quantization; native 100%, 200%, and 400% views switch to the full-size image, and detail crops preserve its 16-bit samples. The report retains the lossless native detail images and numerical masters. Browser and screen precision still limit the displayed result. Preview resampling never feeds measurements.

Case, pair, and region links preserve exact identities in the report fragment, for example `#case=color_chart&seed=0&pair=baseline%2Fcandidate&region=skin`. Opening or reloading a link selects the evidence and expands its requested detail offline. Detail buttons expose their names and expanded state to keyboard and browser agents. Invalid locations display an error instead of unrelated imagery.

## Review prepared images

Use `report --manifest` for prepared images grouped into sets. It does not need a renderer, run directory, or comparison record, and it does not calculate color differences or acceptance scores. Keep measured findings in the existing comparison workflow; this command provides reusable visual inspection.

Each set declares exactly one form. A `grid` set gives every image the same row-major patch windows derived from `origin`, `patch_size`, and `gap`. A `regions` set declares explicit half-open `[x, y, width, height]` rectangles per image in `geometry`. Region identities are scoped to their set, and different sets never acquire correspondence from equal sizes, labels, or patch names.

A newly generated fixture bundle contains `review.json`. Its synthetic probes demonstrate a shared grid, a scaled grid copy, and explicit per-image regions without any cross-set correspondence:

```bash
./target/release/spektrafilm-lab validate --kind review \
    --file /tmp/spektrafilm-benchmark-fixtures/review.json
./target/release/spektrafilm-lab report --manifest \
    /tmp/spektrafilm-benchmark-fixtures/review.json \
    --out /tmp/spektrafilm-image-review
```

For your own images, save this manifest beside the named prepared files. Replace the names and descriptions with their actual provenance. Paths resolve relative to the manifest, not the command's working directory. The following grid describes 768 × 432 TE226 images; explicit regions declare one rectangle per named image and need no shared layout. Neither form detects or rectifies a photographed chart.

```json
{
  "schema_version": 1,
  "title": "800T 图像对照",
  "description": "输入来源分别标注；没有曝光对齐或颜色拟合。",
  "sets": [
    {"id": "te226", "label": "TE226", "grid": {
      "rows": 5, "columns": 9, "origin": [12, 12],
      "patch_size": [72, 72], "gap": [12, 12]
    },
    "measurement": {"sample": [30, 30], "role": "color", "measurements": ["color", "tone"]}},
    {"id": "blue1", "label": "蓝光打印色卡1", "grid": {
      "rows": 3, "columns": 8, "origin": [12, 12],
      "patch_size": [72, 72], "gap": [24, 24]
    }},
    {"id": "blue2", "label": "蓝光打印色卡2", "grid": {
      "rows": 3, "columns": 8, "origin": [12, 12],
      "patch_size": [72, 72], "gap": [24, 24]
    }},
    {"id": "street", "label": "实景对照", "regions": [
      {"id": "face", "label": "面部",
       "geometry": {"scan-6170": [1832, 1044, 640, 800],
                    "sim-800t": [1608, 978, 512, 640]}}
    ]}
  ],
  "images": [
    {"id": "synthetic", "set": "te226", "path": "synthetic.tif",
     "label": "厂家示例合成图", "description": "不是被拍实体色卡的实测值。",
     "color_space": "ProPhoto RGB", "cctf_decoding": false},
    {"id": "native", "set": "te226", "path": "raw-chart.tif",
     "label": "原生 RAW 应用输入", "description": "as-shot 解码并按既存几何裁切，不是场景真值。",
     "color_space": "ACES2065-1", "cctf_decoding": false},
    {"id": "blue1-input", "set": "blue1", "path": "blue1.png",
     "label": "蓝光打印1", "description": "已准备的 sRGB 显示图。",
     "color_space": "sRGB", "cctf_decoding": true},
    {"id": "blue2-input", "set": "blue2", "path": "blue2.png",
     "label": "蓝光打印2", "description": "与第一组不同的色块序列。",
     "color_space": "sRGB", "cctf_decoding": true},
    {"id": "scan-6170", "set": "street", "path": "scan.png",
     "label": "打印扫描", "description": "逐图声明的面部矩形，不与其它集合对应。",
     "color_space": "sRGB", "cctf_decoding": true},
    {"id": "sim-800t", "set": "street", "path": "sim.png",
     "label": "模拟输出", "description": "同一标识按自己的几何声明解释。",
     "color_space": "sRGB", "cctf_decoding": true}
  ],
  "default_pair": {"left": "synthetic", "right": "native"}
}
```

Add simulations and scans as additional `images` records. Use the same grid set only when they share the declared ordered patch identities. Equal grid sizes or equal region labels do not make different sets corresponding. A region may cover a subset of its set's images and must not overlap another region on the same image. To inspect only central 30 × 30 windows, declare their actual origin, window size, and intervening gap instead of changing the source image.

A grid set may declare a `measurement` object. `sample` is the centered window `regions` writes (default: the whole patch window), `role` is the derived region role (default `other`), and `measurements` selects the derived families (default none). The page always shows the declared patch windows; the object only describes the derived regions.

Camera RAW must first use the [native decoder](raw.md), then an explicitly established chart preparation. Declare its prepared linear TIFF as `ACES2065-1` with `cctf_decoding: false`. Declare ordinary encoded sRGB PNGs as `sRGB` with `cctf_decoding: true`. Embedded profiles and neighboring sidecars do not replace the manifest declaration. Do not label ACES data as ProPhoto or use a camera preview as measured color truth.

Open the output `index.html` directly. Select any left and right image, swap them, or inspect individual windows. A shared grid set provides linked selection and a complete paired contact sheet; whole-image inspection also offers split view and manual A/B alternation there. Explicit-region images select independently per side with a label-only correspondence notice and per-side window dimensions. Different sets keep independent selectors and a visual-only warning. Use the zoom selector for fit, native-device-pixel, or enlarged inspection; fit scaling is not native 100% inspection. Captions, source declarations, original hashes, and display artifacts remain available offline. Source images are not edited, and an existing output directory is never replaced.

Click a contact-sheet cell to open that window, or use the arrow keys while a shared-grid image viewport has focus. The page URL fragment retains the image IDs, viewing mode, zoom, and selected windows. Copy the URL to share that exact view; invalid identities produce a visible error rather than silently changing the requested pair.

### Derive measurement regions

Derive a case or reference region array from one grid set when the comparison should measure the same windows the review page shows:

```bash
./target/release/spektrafilm-lab regions \
    --manifest /tmp/spektrafilm-benchmark-fixtures/review.json \
    --set color-probe \
    --out /tmp/color-probe-regions.json
```

The command reads the set's declared `measurement` object and reuses the review page's window arithmetic. Generated records keep the review patch names (`A1`, `A2`, …); without a declared object they keep each full patch window and default to role `other` with no families. They validate as the existing `regions` kind, so they fit a case `regions` array, a reference, or `run --regions`. A set without a grid, an unknown set identity, a `measurement` object on an explicit-regions set, and windows or samples that leave a decoded image are rejected; an existing output file is never replaced.

Use `schema --kind review` for the complete authored schema. Validation reports checks that still require decoding. The generated `report.json` identifies the actual images, set geometry, selected default pair, and output artifacts.

Share the complete output directory, including `viewer.js` and `images/`, rather than only `index.html`. The same directory supports direct file opening and static hosting with a same-origin script policy; no network library or report server is required.

## Read command results and gates

Each parsed command emits one JSON result on stdout. Diagnostic messages use stderr. Status `2` indicates argument syntax errors; status `1` indicates validation, execution, publication, or requested gate failure. Use `schema --kind suite|target|policy|run|comparison` to discover carriers. `compare --policy FILE --gate` publishes the comparison before returning failure for a non-pass evaluation. The supplied exploratory policy must not qualify acceptance. A calibrated policy must bind the digest emitted by suite validation and supply reviewed calibration evidence.

Every result is complete JSON on stdout, and the retained artifact is its authoritative detail: read the fields you need from it, and use the `inspect` filters for exact case, region, and metric evidence; unknown selections report the available identities. Policy binding is mechanical: `validate --kind suite` prints the protocol digest that the policy must carry. A non-pass evaluation status can be read from the retained summary counts and the policy snapshot.

`run` and `compare` write each case-seed start and completion to stderr. Completion includes its status and elapsed seconds; the final summary reports completed and failed items and total elapsed seconds. Keep stderr separate when reading JSON stdout.

Every publication destination must be new. If a destination already exists, choose another destination or remove the existing one after reviewing its contents. A directory left without its terminal manifest is incomplete. A published failed manifest retains its completed items and errors, but does not qualify as a baseline. A gate failure preserves the completed comparison, so inspect it before retrying.

## Understand fixture coverage

The generated complete suite adds `final_appearance`, which renders the chart with the full-appearance configuration and seeds `[0,42,99]`. Acquiring the pinned natural scene also adds `natural_appearance`, which uses that scene's RAW declaration and review regions with the full-appearance configuration and the same seeds. Both appearance cases select numeric, color, tone, and FLIP measurements. Dedicated grain, halation, and edge probes retain their effect-specific settings. The quick suite and `natural.json` retain isolated color-and-tone scene coverage when the natural scene is acquired. Without `--natural-input` or `--download-natural`, the generated suites contain controlled probes only. The `complete.json` filename must not imply comprehensive stock coverage, calibrated acceptance, or full-resolution photographic review. The packaged scene covers shadows and texture; broader skin, foliage, mixed-light, and real film-scan coverage requires explicitly declared assets and regions. A caller must use representative local photographs for native-resolution spatial review. Synthetic probes and rendered reference fixtures must not establish physical film fidelity.

## See also

- [Benchmark specification](../specs/benchmark/spec.md)
- [Building](building.md)
- [CLI](cli.md)
- [RAW input](raw.md)
- [Parity evidence](parity.md)
