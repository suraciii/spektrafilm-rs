# Appearance benchmark

The external `spektrafilm-bench` CLI renders immutable evidence, compares versions, and creates an offline review. It reports color, tone, official LDR-FLIP, grain, halation, edge response, and numerical health by case and region. It preserves floating masters, full-resolution maps, curves, spectra, and per-seed observations.

The [benchmark specification](../specs/benchmark/spec.md) defines reference scopes and qualification rules.

## Generate and compare suite evidence

Build the native benchmark tools. Generate fixtures outside the repository. The optional pinned Kodak DC50 RAW has its own attribution and license in the generated `provenance.json`; the repository does not distribute that original.

The generator exports the explicit built-in `Kodak Gold 200` look as portable `look.toml`. `color-tone.json` isolates color and tone by disabling spatial effects, stochastic effects, and grain. `appearance.json` preserves the look's effect controls and enables spatial and stochastic processing. Both configurations use manual exposure, a 35 mm film-format scale, neutral scanner finishing, and encoded sRGB on `input > film > print > scan`. They preserve the supplied input resolution without preview reduction, automatic crop, or resize. Their preset paths resolve relative to each configuration file. The generated `provenance.json` retains the renderer and bundle file hashes.

The generated grain probe selects `procedural` in custom Noise mode with three explicit seeds. Suites generated before the engine rename still contain the rejected `v2` selector. Generating a new bundle must remain an explicit setup action. A version comparison must reuse the same exported preset and configuration bytes on both sides.

```bash
just build-f64
just build-benchmark
./target/release/spektrafilm-bench-fixtures \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --out /tmp/spektrafilm-benchmark-fixtures --download-natural

./target/release/spektrafilm-bench validate --kind suite \
    --file /tmp/spektrafilm-benchmark-fixtures/complete.json
./target/release/spektrafilm-bench run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --dry-run --out /tmp/spektrafilm-benchmark-preflight
./target/release/spektrafilm-bench run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --out /tmp/spektrafilm-benchmark-baseline
./target/release/spektrafilm-bench run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/candidate.json \
    --out /tmp/spektrafilm-benchmark-candidate
./target/release/spektrafilm-bench compare \
    --baseline /tmp/spektrafilm-benchmark-baseline \
    --candidate /tmp/spektrafilm-benchmark-candidate \
    --out /tmp/spektrafilm-benchmark-comparison
./target/release/spektrafilm-bench inspect \
    --comparison /tmp/spektrafilm-benchmark-comparison \
    --case color_chart --region skin --metric delta_e00.p95
./target/release/spektrafilm-bench report \
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
./target/release/spektrafilm-bench target \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-local-target.json
./target/release/spektrafilm-bench validate --kind configuration \
    --file /tmp/spektrafilm-local-configuration.json
./target/release/spektrafilm-bench run \
    --input /tmp/spektrafilm-benchmark-fixtures/chart.png \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-local-configuration.json \
    --target /tmp/spektrafilm-local-target.json \
    --out /tmp/spektrafilm-local-run
./target/release/spektrafilm-bench compare \
    --run /tmp/spektrafilm-local-run --against input \
    --out /tmp/spektrafilm-local-comparison
./target/release/spektrafilm-bench report \
    --comparison /tmp/spektrafilm-local-comparison \
    --out /tmp/spektrafilm-local-review
```

## Use the standard bundle

Use the standard bundle to run a local raster without authoring simulation settings. This example generates one bundle in a new directory outside the repository and reuses its chart as a declared sRGB raster. For a photograph, replace `LOCAL_IMAGE` and both source declaration flags with the image's actual color interpretation. Set `CANDIDATE_RENDERER` to the path of an already-built renderer for the version to compare. Both targets use the same data directory; an intentional data revision must remain a declared comparison condition. Every output destination must be new.

The exported look is a preset document of the renderer that generated it. A candidate renderer must be able to read that document. A renderer that predates a preset-schema field rejects the bundle with `invalid look`; the observed message for such a mismatch is `unknown field`, followed by the field name. Generate the bundle with the renderer that will read it, and treat a regenerated bundle as a declared comparison condition rather than a silent substitution.

```bash
just build-f64
just build-benchmark
: "${CANDIDATE_RENDERER:?Set CANDIDATE_RENDERER to the candidate renderer executable}"
./target/release/spektrafilm-bench-fixtures \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --out /tmp/spektrafilm-standard-bundle

./target/release/spektrafilm-bench validate --kind configuration \
    --file /tmp/spektrafilm-standard-bundle/color-tone.json
./target/release/spektrafilm-bench validate --kind configuration \
    --file /tmp/spektrafilm-standard-bundle/appearance.json
./target/release/spektrafilm-bench target \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-standard-baseline-target.json

LOCAL_IMAGE=/tmp/spektrafilm-standard-bundle/chart.png
./target/release/spektrafilm-bench run \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/color-tone.json \
    --target /tmp/spektrafilm-standard-baseline-target.json \
    --seed 0 --out /tmp/spektrafilm-standard-color-tone
./target/release/spektrafilm-bench run \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/appearance.json \
    --target /tmp/spektrafilm-standard-baseline-target.json \
    --seed 0 --seed 42 --seed 99 \
    --out /tmp/spektrafilm-standard-appearance-baseline

./target/release/spektrafilm-bench target \
    --renderer "$CANDIDATE_RENDERER" --data-dir ./data \
    --backend cpu --out /tmp/spektrafilm-standard-candidate-target.json
./target/release/spektrafilm-bench run \
    --input "$LOCAL_IMAGE" \
    --input-color-space sRGB --input-cctf-decoding true \
    --configuration /tmp/spektrafilm-standard-bundle/appearance.json \
    --target /tmp/spektrafilm-standard-candidate-target.json \
    --seed 0 --seed 42 --seed 99 \
    --out /tmp/spektrafilm-standard-appearance-candidate
./target/release/spektrafilm-bench compare \
    --baseline /tmp/spektrafilm-standard-appearance-baseline \
    --candidate /tmp/spektrafilm-standard-appearance-candidate \
    --out /tmp/spektrafilm-standard-comparison
./target/release/spektrafilm-bench inspect \
    --comparison /tmp/spektrafilm-standard-comparison --case image
./target/release/spektrafilm-bench report \
    --comparison /tmp/spektrafilm-standard-comparison \
    --out /tmp/spektrafilm-standard-review
```

Open `/tmp/spektrafilm-standard-review/index.html` to review full images and native detail crops. A single seed may provide image-review evidence. It must not establish statistical grain acceptance. The standard look is an end-to-end rendering convention. It must not claim physical film fidelity.

## Preflight a run

Add `--dry-run` to either run form with a new destination to resolve every selected case and seed without decoding or rendering. It retains `preflight.json`, configurations, and renderer logs; successful preflight reports `execution_status: validated`. Image decoding, device availability, observed geometry, and numerical results remain unchecked. Preflight is separate from completed render evidence and cannot be compared as a run. Ordinary execution repeats the same configuration checks.

## Inspect a comparison

`inspect` reads a validated comparison without computing metrics. Repeat `--case`, `--region`, and `--metric` for exact selection; unknown selections report available identities. Its compact records preserve units, domains, unavailable reasons, and evidence links relative to `artifact_base`. Per-seed/pair `measurements` and cross-seed `seed_measurements` remain separate. The retained comparison's execution and evaluation states remain visible, including failures; inspection never produces a universal appearance score.

## Use camera RAW and prepared inputs

Use `--input-kind raw` for camera RAW and omit the raster color flags. Set dedicated RAW options in the configuration's `raw` object. The run retains the real renderer's decoded input with the same crop and resize as simulation, along with the original source dimensions. Set simulation crop and resize through the configuration's existing `set_files` or `set` carriers; fixed review regions use completed-render coordinates. An externally prepared crop is a separate input; optional `--preparation FILE` records its original path, SHA-256, and descriptive operations without executing them.

## Retain a local reference

Add `--reference FILE --reference-info FILE` to retain a local reference. Reference info uses the existing reference declaration without `path` and `sha256`; it must declare color, provenance, processing, and scope. Then select `compare --run DIRECTORY --against reference` or `--against both`. Input/simulation reports describe processing changes; reference/simulation reports describe applicable distances to the reference. Visual and regional references remain separate native views, with explicit limits on pixelwise measurements. Both-pair reports open reference/simulation and provide a selector for input/simulation. No single-run comparison claims acceptance.

## Preserve evidence integrity

The decoded input route fixes its taps to RGB input/output, so simulation diagnostic taps cannot relabel film density as source RGB.

## Review evidence offline

Open `/tmp/spektrafilm-benchmark-review/index.html`. The human review opens one case and seed at a time in request order and keeps every case, seed, failure, and artifact reachable through the selector and folded evidence. The first view shows the selected comparison pair, a short retained condition summary, and a small rounded full-image metric summary; sources, references, difference maps, curves, full measurements, execution settings, findings, and detail regions stay collapsed until needed. Fixed regions, automatic hotspots, lossless crops, synchronized pan and zoom, manual A/B alternation, and split view remain available for aligned pairs. Detail zoom supports 100%, 200%, and 400% in device pixels. Report generation reads retained evidence and needs neither the original input files nor the renderer.

The viewer opens in split mode: drag the divider on the image, use the range control, or move the focused divider with arrow keys (Shift for larger steps; Home/End for endpoints). Display copies use 16-bit RGB sRGB PNG without dither. Fit views resize floating samples before 16-bit quantization; native 100%, 200%, and 400% views switch to the full-size image, and detail crops preserve its 16-bit samples. Browser and screen precision still limit the displayed result. Preview resampling never feeds measurements.

Case, pair, and region links preserve exact identities in the report fragment, for example `#case=color_chart&seed=0&pair=baseline%2Fcandidate&region=skin`. Opening or reloading a link selects the evidence and expands its requested detail offline. Detail buttons expose their names and expanded state to keyboard and browser agents. Invalid locations display an error instead of unrelated imagery.

## Read command results and gates

Each parsed command emits one JSON result on stdout. Diagnostic messages use stderr. Status `2` indicates argument syntax errors; status `1` indicates validation, execution, publication, or requested gate failure. Use `schema --kind suite|target|policy|run|comparison` to discover carriers. `compare --policy FILE --gate` publishes the comparison before returning failure for a non-pass evaluation. The supplied exploratory policy must not qualify acceptance. A calibrated policy must bind the digest emitted by suite validation and supply reviewed calibration evidence.

## Understand fixture coverage

The generated complete suite adds `final_appearance`, which renders the chart with the full-appearance configuration and seeds `[0,42,99]`. Acquiring the pinned natural scene also adds `natural_appearance`, which uses that scene's RAW declaration and review regions with the full-appearance configuration and the same seeds. Both appearance cases select numeric, color, tone, and FLIP measurements. Dedicated grain, halation, and edge probes retain their effect-specific settings. The quick suite and `natural.json` retain isolated color-and-tone scene coverage when the natural scene is acquired. Without `--natural-input` or `--download-natural`, the generated suites contain controlled probes only. The `complete.json` filename must not imply comprehensive stock coverage, calibrated acceptance, or full-resolution photographic review. The packaged scene covers shadows and texture; broader skin, foliage, mixed-light, and real film-scan coverage requires explicitly declared assets and regions. A caller must use representative local photographs for native-resolution spatial review. Synthetic probes and rendered reference fixtures must not establish physical film fidelity.

## See also

- [Benchmark specification](../specs/benchmark/spec.md)
- [Building](building.md)
- [CLI](cli.md)
- [RAW input](raw.md)
- [Parity evidence](parity.md)
