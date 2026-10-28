# Scanning and negative conversion

This page explains the scan model, scan output modes, negative conversion, and film-scan controls. It also describes the experimental CineStill 800T Cs41 material and look.

## Neutral scan model

A scan is one colorimetric capture. The scanner has one sensor, modeled on the CIE 1931 color-matching functions, and one readout light. There is no receiver profile to choose, no scanner calibration matrix, and no scanner finish.

The readout light is a product constant selected by the material category: `BB5800` for colour cine print materials (ISO 2910:2018) and `D50` for every other material (ISO 3664). It is not a runtime parameter and not a profile field, so there is no `scanner.lamp` to set; a document that carries that field fails validation.

The white reference of a scan is the clear film base of the scanned medium. A clear area of that base reads as the output color space's white, and the scan chromatically adapts from that clear-film white to the output white. The scan matrix is the identity. The readout light is part of the final colour transform, not an acquisition control.

The scanner controls include `scanner.scan_output`, `scanner.conversion_style`, `scanner.experimental_setup`, `scanner.operator`, `scanner.lens_blur`, and `scanner.unsharp_mask`. Lens blur models readout optics on linear transmitted light and runs before any density interpretation. Unsharp mask is digital sharpening and runs after interpretation and output gamut processing. Legacy white/black correction, white/black levels, film-scan black and white points, receiver profiles, negative conversion methods, the retired `scanner.auto_setup` control, scanner finish, and Digital Reference remain removed; their field names fail validation at the current schema version.

`settings.use_scanner_lut` selects a PCHIP lattice of normalized `log_xyz`, not a direct spectral scan or an RGB lattice. After interpolation, the scanner applies `10^log_xyz`, chromatic adaptation, and XYZ-to-RGB in that order. On the GPU route, lattice generation, slope and cell-hull preparation, interpolation, and the color transforms remain on the device. Storage limits split the lattice into overlapping source halos without changing its domain or resolution. A limit below the minimum halo or spectral coefficient binding returns an error; it does not select a CPU implementation.

## Experimental workflow routes and output modes

The GUI exposes passthrough, film-scan, film-print-scan, and the three convert-film routes. The CLI also accepts material-producing and continuation routes described in [stage artifacts](../specs/cli/stage-artifacts.md). Direct negative-film scans expose `scanner.scan_output=direct_scan|negative_to_positive`.

`direct_scan` preserves the medium's polarity. Calibrated `negative_to_positive` supports direct `input > film > scan`, resumed `film > scan`, and real negative-capture `input > convert-film > scan`. It rejects paper, positive-film, print-paper, scan-minus-base, and active-Grain-Procedural combinations.

`scanner.scan_output` accepts `direct_scan` and `negative_to_positive`. Replace the removed `positive_scan` value in CLI commands, parameter files, presets, recipes, and GUI states. Old values fail validation. The GUI displays **Preserve medium polarity** and **Negative to positive**. See [scanner language](../specs/film-chain/spec.md#scanner-language).

## Negative conversion

The following command selects the scan output explicitly.

```bash
# Negative conversion
./target/release/spektrafilm-f64 process input.tif -o positive.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --scan-output negative_to_positive --data-dir data

# Inspect effective scanner selections without decoding or writing output
./target/release/spektrafilm process input.tif -o positive.tif --film kodak_portra_400 --scan-film \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --scan-output negative_to_positive --dry-run --data-dir data
```

`negative_to_positive` reads each dye record's density like a narrowband lab scanner. It inverts that density through the record's own profile density curve. It then passes through one logistic positive tone curve. A record never borrows density from the others, so saturated negatives do not clip one channel. The inverse is profile-derived and monotone. It has finite endpoint slopes beyond the curves. The tone keeps output within the paper range.

There is one negative conversion interpretation. It has no selectable method. The tone curve's constants come from a declared conversion style, described below. Zero balance renders a gray lit by the film's reference illuminant neutral. Scanner lens blur acts on transmitted light before the density reading.

Negative conversion applies to direct negative-film scans and the `input > convert-film > scan` route. Print-paper and scan-minus-base routes retain their existing route semantics. The displayed result interprets film density. It is not a measurement of the film or a match guarantee for a processed reference JPEG. Hold profile, exposure, white balance, and scan settings fixed when comparing results.

For a real negative scanner capture on `input > convert-film > scan`, disable virtual camera metering with `camera.auto_exposure=false`. Supply input transfer and color interpretation. Convert-film estimates the record densities that the negative converter reads. Set `film_render.convert.scan_illuminant` to the acquisition light (default `D55`). This is an imported-capture declaration, not a readout-light knob; final material readout remains fixed at D50 or BB5800.

The resident GPU route performs both density acquisition and convert-film inversion on the device. Negative-to-positive acquisition uses the profile-derived monotone axis of each record, including its finite endpoint extrapolation, and the resolved brightness, contrast, and maximum density of that channel's style. Scanner blur runs as density → transmission → reference Gaussian filter → density before interpretation; it is not applied a second time after interpretation.

Convert-film uses a bounded eight-iteration inverse scan under `film_render.convert.scan_illuminant`, with the declared input transfer, calibration matrix, and exposure compensation. The `scan`, `scan-minus-base`, and `print > scan` routes retain their respective base inclusion and paper stages. Output gamut processing, sharpening, transfer encoding, and output mix remain downstream. These device paths require native shader binary64; a selected GPU that cannot provide the required arithmetic reports an error rather than running image acquisition or inversion on the CPU.

## Conversion styles

`scanner.conversion_style` defaults to `standard`, the built-in frozen tone. It needs no data directory. `neutral` is an input-only migration alias; saved parameters report `standard`. Any other name resolves to `data/scan-styles/<name>.json`.

### Experimental frame setup

`scanner.experimental_setup` defaults to `false`. When enabled it measures the whole developed frame for exposure and color balance before the tone. It is off by default because the algorithm is a declared heuristic that has not been validated against real minilab references, not a measured Frontier or Noritsu behavior: it uses a weighted mean/minimum/maximum density estimate and a partial exposure correction, compares the frame to the stock's neutral locus, and reduces correction for dominant colors. Bright backgrounds can still darken a subject; no scene classifier or local tone mapper rescues it.

With the setup disabled, the readout is a fixed, frame-independent transform and no measurement cost is paid. The setup does not subtract simulated camera exposure compensation a second time; without it every style retains the historical camera compensation. The setup runs after scanner lens blur, before tone and output processing.

A frame-dependent scan cannot be represented by a point-wise LUT. While the setup is enabled, LUT baking and point-wise measurements that reach it fail and ask the caller to disable it. Upstream density and exposure taps remain measurable either way.

## Operator correction

Four controls adjust the positive image before the fixed tone. They are exposure-coordinate shifts and work with or without the experimental setup:

| Field under `scanner.operator` | Default | Positive direction |
| --- | --- | --- |
| `exposure_ev` | `0` | Brighter; common reconstructed exposure in stops |
| `warmth_ev` | `0` | Warmer; R−B exposure difference in stops |
| `tint_ev` | `0` | More magenta; mean(R,B)−G exposure difference in stops |
| `contrast` | `1` | Values above one increase common-exposure contrast about the reference |

Warmth and tint are differential EV, not Kelvin. These controls do not change negative density, camera exposure, Convert recovery, or the experimental setup. Contrast does not amplify channel differences before tone. The nonlinear tone can still change perceived hue and saturation. See the [equations and numerical contract](../specs/film-chain/lab-scan/spec.md#operator-correction).

```bash
./target/release/spektrafilm-f64 process input.tif -o corrected.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --scan-output negative_to_positive --data-dir data \
    --set scanner.operator.exposure_ev=0.5,scanner.operator.warmth_ev=0.3,scanner.operator.tint_ev=0.1,scanner.operator.contrast=1.1

./target/release/spektrafilm describe --module scanner.operator --format json
```

All four values must be finite, and contrast must be greater than zero. The GUI trial ranges are exposure ±3 EV, warmth/tint ±1 EV, and contrast 0.5–2. They are editor ranges, not document validation limits. Nondefault values require `standard`, `negative_to_positive`, and a supported negative-film route. CLI, recipes, and presets reject incompatible explicit combinations. The GUI retains inactive edits and renders with neutral operator values until the route and style support them. Reset restores `0, 0, 0, 1` without changing setup.

Material-producing CLI routes may retain a valid pending negative-scan correction in their `.sfstage` baseline. They report it as inactive because no scanner ran. On `film > scan`, the correction becomes active and can be changed without developing the material again; the experimental setup, when enabled, is measured from the retained complete material frame. See [material-stage continuation](../specs/cli/stage-artifacts.md).

With the experimental setup enabled, a negative-to-positive preview must render the complete crop/upscale working grid, including spatial effects, and reduce only the terminal RGB: the setup needs the whole frame, so preview size must not change its baseline. Without the setup, preview size caps the working resolution instead, which is the cheaper default. Operator variants share core preparation when reusable; optical Dye/Fotufilm grain may require exact full-route recomputation. There is no separate approximate scan engine or hidden cross-frame setup lock. Full-grid preparation costs memory; it must not silently use reduced precision or sampled setup while the setup is on.

The retained GPU scan cache owns one complete acquisition: binary64 reconstructed record exposures, full-frame setup, and device status. It keys the immutable developed-film owner, device, geometry, calibrated axes, scanner lens blur, and setup constants—not mutable working buffers. Operator and tone variants reuse that acquisition and rerun the output tail without measuring setup again. A different frame or acquisition setting replaces the entry; cancellation or a failed device status check cannot publish one. Observation and cancellation wrappers share the same device-owned entry. Ordinary uncached processing runs the same acquisition algorithm, and no frame's automatic offsets are carried into another frame.

### Output color and compatibility

Both standard and fixed negative-to-positive tone responses denote fixed linear sRGB. The renderer converts this response into `io.output_color_space` before the shared gamut and output tail. Destination conversion may mix channels; the record inverse itself remains independent. Writer saving-space conversion and display ICC processing are separate steps.

Neutral operator values bypass correction exactly. Pixel compatibility with PR #318 applies to sRGB only. Non-sRGB output intentionally changes because older versions assigned different primaries to unchanged tone response values. This changes the runtime model version, not the additive document schema.

## Fixed conversion styles

Any other name resolves to `data/scan-styles/<name>.json` in your data directory. No extra fixed style ships with the product. With a stock data directory, only `standard` is listed. A style selects the tone only; it never runs the frame setup, which is the separate experimental switch.

A style file is user-authored declared data. It declares one tone triple per channel:

```json
{
  "channels": [
    { "brightness_ev": 0.9281766618, "contrast": 0.7355140005821664, "dmax": 1.5625841465572565 },
    { "brightness_ev": 0.8781766618, "contrast": 0.7355140005821664, "dmax": 1.5625841465572565 },
    { "brightness_ev": 0.8281766618, "contrast": 0.7355140005821664, "dmax": 1.5625841465572565 }
  ]
}
```

The three triples are what let a style carry a color tendency on top of the tone. The file above lifts red by `+0.05 EV` and drops blue by `-0.05 EV` against the neutral constants, with the same contrast and maximum density, which gives a slight warm lab-print tendency. Save it as `data/scan-styles/warm.json` to use it.

Every value must be finite, and `contrast` and `dmax` must be positive. A parse error names the offending field.

A style parameterizes the constants of the same logistic curve family the built-in style uses, and it never adds a curve family. It is never fitted from a photograph or a chart, and it is not a measured or calibrated scanner response. The per-record reconstruction remains unchanged. Fixed styles preserve record independence before destination primary conversion; standard setup depends on the complete frame.

A custom fixed style requires `scanner.scan_output = negative_to_positive`. A `direct_scan` does not read the style slot, so the style is inert there. Style names must not be empty and must not contain `/`, `\`, `..`, or `:`. The style resolves once when the pipeline is built.

```bash
# Write the style file into your data directory
mkdir -p data/scan-styles
cat > data/scan-styles/warm.json <<'JSON'
{
  "channels": [
    { "brightness_ev": 0.9281766618, "contrast": 0.7355140005821664, "dmax": 1.5625841465572565 },
    { "brightness_ev": 0.8781766618, "contrast": 0.7355140005821664, "dmax": 1.5625841465572565 },
    { "brightness_ev": 0.8281766618, "contrast": 0.7355140005821664, "dmax": 1.5625841465572565 }
  ]
}
JSON

./target/release/spektrafilm-f64 process input.tif -o warm.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --set scanner.conversion_style=warm \
    --scan-output negative_to_positive --data-dir data
```

The resolved style appears in the effective configuration of the render report, alongside the scan output mode and the resolved readout illuminant.

## Experimental CineStill 800T Cs41 material

`cinestill_800t_cs41_approx` is an experimental fitted approximation of CineStill 800T in Cs41.

The material profile computes developed dye amounts and spectral density before printing or scanning. It does not provide measured CineStill spectral sensitivity, dye spectra, grain, or halation. Keep grain disabled for material comparisons. Compare halation separately.

The model exposure origin does not establish absolute ISO. It does not prove that the manufacturer's reference is an 18% gray exposure. Metadata records the sources, approximations, and attribution. The derivative retains Andrea Volpato's CC BY-SA 4.0 asset license.

## Experimental CineStill 800T Cs41 look

The built-in `CineStill 800T Cs41 (experimental)` look sets halation and Grain Dye controls.
The look keeps grain off because its grain controls were fitted at the 36 mm reference frame; enable them with `film_render.grain.active=true`. Another positive `camera.film_format_mm` changes the physical sampling and does not establish calibration there.

### Reference illuminant and tungsten balance

The film chain normalizes spectral upsampling to the profile's reference illuminant. A neutral input pixel means white under that light. For this tungsten profile, that light is `T` (CIE xy 0.4507, 0.4081; about 2812 K). An as-shot white-balanced input therefore hides the film's color balance.

To evaluate tungsten balance, adapt the input from its capture light to `T`. Decode RAW with `--raw-white-balance custom --raw-temperature 2812`. Adapt rendered input with CAT02 from its known capture white. Daylight scenes then render blue. Lamps warmer than 3200 K render orange. A 3200 K-lit gray renders neutral.

### Negative-to-positive tone fit

The look's `negative_to_positive` tone assumes scene-linear input. Its tone values live in the look bundle and preset.

```bash
./target/release/spektrafilm-f64 process input.tif -o cinestill.png \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --preset 'CineStill 800T Cs41 (experimental)' --backend cpu --data-dir data \
    --route 'input > film > scan' --scan-output negative_to_positive
```

## See also

- [Pipeline workflows](pipeline.md)
- [CLI usage](cli.md)
- [GUI usage](gui.md)
- [RAW input](raw.md)
- [Presets](presets.md)
- [Grain](grain.md)
- [Attribution](attribution.md)
- [Film-chain specification](../specs/film-chain/spec.md)
- [Project context](../CONTEXT.md)

## Saved document cutover

GUI state version 5, preset schema 6, and recipe schema `spektrafilm-rs-params-6` own the scanner cutover. The additive `scanner.operator` group uses these same versions; omitted values default to `0, 0, 0, 1`. Saved corrections contain no automatic offsets. Unknown and retired fields remain errors. Explicit older versions migrate `scanner.lamp` out of the readout controls. Complete captures preserve it as the Convert acquisition illuminant unless already declared. Current versions reject `scanner.lamp`. The built-in `neutral` spelling becomes `standard`; this migration intentionally changes the automatic scan output.
