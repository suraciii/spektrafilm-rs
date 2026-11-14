# Scanning and negative conversion

This page explains scanner readout, scan output modes, negative conversion, and the broader Scan workflow. It also describes the experimental CineStill 800T material and look.

## Neutral scan model

Scanner readout is one colorimetric capture. The scanner has one sensor, modeled on the CIE 1931 color-matching functions, and one readout light. There is no receiver profile to choose, no scanner calibration matrix, and no scanner finish.

The GUI Scan stage covers readout, interpretation, and digital adjustments. **Scan → Tone & Color** adjusts an eligible completed positive without changing the scanned material, acquisition, or negative-conversion operator correction. Its independent `finishing.*` controls and preset section remain separate from `scanner.*`; see [Tone & Color](gui.md#tone--color).

The readout light is a product constant selected by the material category: `BB5800` for colour cine print materials (ISO 2910:2018) and `D50` for every other material (ISO 3664). It is not a runtime parameter and not a profile field, so there is no `scanner.lamp` to set; a document that carries that field fails validation.

By default, the white reference of a scan is the clear film base of the scanned medium. A clear area of that base reads as the output color space's white, and the scan chromatically adapts from that clear-film white to the output white. The scan matrix is the identity. The readout light is part of the final colour transform, not an acquisition control.

`scanner.white_reference` selects the calibration reference. `clear_film_base` is the default calibrated readout. `readout_illuminant` opts out: the scan divides by the readout-light Y and adapts from the readout-light white. That mode reproduces the upstream experimental `28bf883` scanner normalization and is intended for upstream comparison; it does not claim clear-base calibration. Scan provenance reports the selected reference. On the pinned Portra Endura flat reference, the remaining encoded difference after matching all other settings is below `2e-5`, attributable to upstream output-gamut compression.

The scanner controls include `scanner.scan_output`, `scanner.white_reference`, `scanner.conversion_style`, `scanner.operator`, `scanner.scan_profile`, `scanner.lens_blur`, and `scanner.unsharp_mask`. Lens blur models readout optics on linear transmitted light and runs before any density interpretation. Unsharp mask is digital sharpening and runs after interpretation and output gamut processing. Legacy white/black correction, white/black levels, film-scan black and white points, receiver profiles, negative conversion methods, the retired `scanner.auto_setup` and `scanner.experimental_setup` controls, scanner finish, and Digital Reference remain removed; their field names fail validation at the current schema version.

`--scan-profile` selects the built-in scan-stage appearance after parameter sources: `none` bypasses it and `sp3000_inspired` applies the catalog curve and its default strength. `custom` is an effective state, not a selectable name. Any other value fails before rendering. A custom curve travels in a look or section preset.

`settings.use_scanner_lut` selects a PCHIP lattice of normalized `log_xyz`, not a direct spectral scan or an RGB lattice. After interpolation, the scanner applies `10^log_xyz`, chromatic adaptation, and XYZ-to-RGB in that order. On the GPU route, lattice generation, slope and cell-hull preparation, interpolation, and the color transforms remain on the device. Storage limits split the lattice into overlapping source halos without changing its domain or resolution. A limit below the minimum halo or spectral coefficient binding returns an error; it does not select a CPU implementation.

## Experimental workflow routes and output modes

The GUI exposes passthrough, film-scan, film-print-scan, and the three convert-film routes. The CLI also accepts material-producing and continuation routes described in [stage artifacts](../specs/cli/stage-artifacts.md). Direct negative-film scans expose `scanner.scan_output=direct_scan|negative_to_positive`.

`direct_scan` preserves the medium's polarity. Calibrated `negative_to_positive` supports direct `input > film > scan`, resumed `film > scan`, and real negative-capture `input > convert-film > scan`. It rejects paper, positive-film, print-paper, and scan-minus-base combinations; every grain engine is accepted.

`scanner.scan_output` accepts `direct_scan` and `negative_to_positive`. Replace the removed `positive_scan` value in CLI commands, parameter files, presets, recipes, and GUI states. Old values fail validation. The GUI displays **Preserve medium polarity** and **Negative to positive**. See [scanner language](../specs/film-chain/spec.md#scanner-language).

To match the upstream experimental scanner while keeping the product default, select the readout-light white reference explicitly:

```bash
./target/release/spektrafilm-f64 process input.tif -o upstream-aligned.tif \
    --input-color-space 'sRGB' --backend cpu --film kodak_portra_400 \
    --paper kodak_portra_endura --route 'input > film > print > scan' --data-dir data \
    --set scanner.white_reference=readout_illuminant
```

The same field is available to presets, recipes, and GUI state. Omitted values default to `clear_film_base`.

## Negative conversion

The following commands select the scan output and the scan-stage appearance explicitly.

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

# Apply the built-in scan-stage appearance
./target/release/spektrafilm-f64 process input.tif -o positive.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --scan-output negative_to_positive --scan-profile sp3000_inspired --data-dir data
```

`negative_to_positive` reconstructs each dye record's exposure from that record's own density. The readout integrates the material under the product illuminant with the CIE 1931 observer. It inverts the record's density through its own profile density curve. It then passes through one logistic positive tone curve. A record never borrows density from the others, so saturated negatives do not clip one channel. The inverse is profile-derived and monotone. It has finite endpoint slopes beyond the curves. The tone keeps output within the paper range.

There is one negative conversion interpretation. It has no selectable method. The tone curve's constants come from a declared conversion style, described below. Zero balance renders a gray lit by the film's reference illuminant neutral. Scanner lens blur acts on transmitted light before the density reading.

Negative conversion applies to direct negative-film scans and the `input > convert-film > scan` route. Print-paper and scan-minus-base routes retain their existing route semantics. The displayed result interprets film density. It is not a measurement of the film or a match guarantee for a processed reference JPEG. Hold profile, exposure, white balance, and scan settings fixed when comparing results.

For a real negative scanner capture on `input > convert-film > scan`, disable virtual camera metering with `camera.auto_exposure=false`. Supply input transfer and color interpretation. Convert-film estimates the record densities that the negative converter reads. Set `film_render.convert.scan_illuminant` to the acquisition light (default `D55`). This is an imported-capture declaration, not a readout-light knob; final material readout remains fixed at D50 or BB5800.

The resident GPU route performs both density acquisition and convert-film inversion on the device. Negative-to-positive acquisition uses the profile-derived monotone axis of each record, including its finite endpoint extrapolation, and the resolved brightness, contrast, and maximum density of that channel's style. Scanner blur runs as density → transmission → reference Gaussian filter → density before interpretation; it is not applied a second time after interpretation.

Convert-film uses a bounded eight-iteration inverse scan under `film_render.convert.scan_illuminant`, with the declared input transfer, calibration matrix, and exposure compensation. The `scan`, `scan-minus-base`, and `print > scan` routes retain their respective base inclusion and paper stages. Output gamut processing, sharpening, transfer encoding, and output mix remain downstream. Negative-to-positive acquisition runs in f32 on a device without native binary64 (measured 8.5e-7 log10 exposure and 3.6e-7 tone against the f64 reference), and the experimental convert-film inverse still requires native binary64: a selected GPU that cannot provide its arithmetic reports an error rather than running image inversion on the CPU.

## Conversion styles

`scanner.conversion_style` defaults to `standard`, the built-in frozen tone. It needs no data directory. `neutral` is an input-only migration alias; saved parameters report `standard`. Any other name resolves to `data/scan-styles/<name>.json`.

### Automatic frame setup

The product has no automatic frame setup. The readout is a fixed, frame-independent transform. No stage measures the complete frame, so picture content cannot change exposure, cast, or contrast.

An earlier experimental setup measured the developed frame. It was removed because it made the scan depend on picture content, could not be baked into a point-wise LUT, and had no paired minilab evidence. Manual exposure, warmth, tint, and contrast judgment belong to the Negative conversion group.

## Operator correction

Four controls adjust the positive image before the fixed tone. They are exposure-coordinate shifts:

| Field under `scanner.operator` | Default | Positive direction |
| --- | --- | --- |
| `exposure_ev` | `0` | Brighter; common reconstructed exposure in stops |
| `warmth_ev` | `0` | Warmer; R−B exposure difference in stops |
| `tint_ev` | `0` | More magenta; mean(R,B)−G exposure difference in stops |
| `contrast` | `1` | Values above one increase common-exposure contrast about the reference |

Warmth and tint are differential EV, not Kelvin. These controls do not change negative density, camera exposure, or Convert recovery. Conversion contrast does not amplify channel differences before tone. The nonlinear tone can still change perceived hue and saturation. See the [equations and numerical contract](../specs/film-chain/lab-scan/spec.md#operator-correction).

```bash
./target/release/spektrafilm-f64 process input.tif -o corrected.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --scan-output negative_to_positive --data-dir data \
    --set scanner.operator.exposure_ev=0.5,scanner.operator.warmth_ev=0.3,scanner.operator.tint_ev=0.1,scanner.operator.contrast=1.1

./target/release/spektrafilm describe --module scanner.operator --format json
```

All four values must be finite, and contrast must be greater than zero. The GUI trial ranges are exposure ±3 EV, warmth/tint ±1 EV, and contrast 0.5–2. They are editor ranges, not document validation limits. Nondefault values require `standard`, `negative_to_positive`, and a supported negative-film route. CLI, recipes, and presets reject incompatible explicit combinations. The GUI retains inactive edits and renders with neutral operator values until the route and style support them. Reset restores `0, 0, 0, 1` without changing the conversion style.

Material-producing CLI routes may retain a valid pending negative-scan correction in their `.sfstage` baseline. They report it as inactive because no scanner ran. On `film > scan`, the correction becomes active and can be changed without developing the material again. See [material-stage continuation](../specs/cli/stage-artifacts.md).

A negative-to-positive preview may reduce the working grid to the preview size and reduce only the terminal RGB, because the readout is frame-independent. Operator variants share core preparation when reusable; optical Dye/Fotufilm grain may require exact full-route recomputation. There is no separate approximate scan engine. Full-grid preparation costs memory; it must not silently use reduced precision.

The retained GPU scan cache owns one complete acquisition: reconstructed record exposures (f32 on devices without binary64) and device status. It keys the immutable developed-film owner, device, geometry, calibrated axes, and scanner lens blur—not mutable working buffers. Operator and tone variants reuse that acquisition and rerun the output tail. A different frame or acquisition setting replaces the entry; cancellation or a failed device status check cannot publish one. Observation and cancellation wrappers share the same device-owned entry. Ordinary uncached processing runs the same acquisition algorithm.

### Output color and compatibility

Both standard and fixed negative-to-positive tone responses denote fixed linear sRGB. The renderer converts this response into `io.output_color_space` before the shared gamut and output tail. Destination conversion may mix channels; the record inverse itself remains independent. Writer saving-space conversion and display ICC processing are separate steps.

Neutral operator values bypass correction exactly. Pixel compatibility with PR #318 applies to sRGB only. Non-sRGB output intentionally changes because older versions assigned different primaries to unchanged tone response values. This changes the runtime model version, not the additive document schema.

## Fixed conversion styles

Any other name resolves to `data/scan-styles/<name>.json` in your data directory. No extra fixed style ships with the product. With a stock data directory, only `standard` is listed. A style selects the tone only. It never measures the frame.

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

A style parameterizes the constants of the same logistic curve family the built-in style uses, and it never adds a curve family. It is never fitted from a photograph or a chart, and it is not a measured or calibrated scanner response. The per-record reconstruction remains unchanged. Every style preserves record independence and is independent of the frame.

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

The material profile computes developed dye amounts and spectral density before printing or scanning. It does not provide measured CineStill spectral sensitivity, dye spectra, grain, or halation. For material-only comparisons, disable the look's grain and halation; those are separate appearance controls.

The model exposure origin does not establish absolute ISO. It does not prove that the manufacturer's reference is an 18% gray exposure. Metadata records the sources, approximations, and attribution. The derivative retains Andrea Volpato's CC BY-SA 4.0 asset license.

## Experimental CineStill 800T look

The built-in `CineStill 800T` look sets halation and Grain Dye controls. Both are fitted to public CineStill 800T lab scans: halation uses a 160 um first Gaussian sigma, three bounces and RGB strength `[0.15, 0.045, 0.005]`; Dye grain uses RMS `[13.1, 19.2, 24.4]` and particle scales `[4.0, 0.15, 0.03]`. These are appearance controls, not a measured PSF or granularity.

The look enables grain by default, but its Dye controls were fitted at the 36 mm reference frame. Set `camera.film_format_mm=36` for exact physical-scale interpretation; another positive format changes the physical sampling and does not establish calibration there. The former `Night 280um` appearance is documented below as an optional override, not a separate built-in. The look name changed from `CineStill 800T Cs41 (experimental)`; the old selector string is not an alias.

### Reference illuminant and tungsten balance

The film chain normalizes spectral upsampling to the profile's reference illuminant. A neutral input pixel means white under that light. For this tungsten profile, that light is `T` (CIE xy 0.4507, 0.4081; about 2812 K). An as-shot white-balanced input therefore hides the film's color balance.

To evaluate tungsten balance, adapt the input from its capture light to `T`. Decode RAW with `--raw-white-balance custom --raw-temperature 2812`. Adapt rendered input with CAT02 from its known capture white. Daylight scenes then render blue. Lamps warmer than 3200 K render orange. A 3200 K-lit gray renders neutral.

### Negative-to-positive tone fit

The look's `negative_to_positive` tone assumes scene-linear input. Its tone values live in the look bundle and preset.

```bash
./target/release/spektrafilm-f64 process input.tif -o cinestill.png \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --preset 'CineStill 800T' --backend cpu --data-dir data \
    --route 'input > film > scan' --scan-output negative_to_positive
```

### Former Night 280 um appearance

The broad-halation night variant is no longer a separate built-in. Starting from `CineStill 800T`, these overrides reproduce its appearance experiment (not a second scan fit):

```bash
--set film_render.halation.halation_first_sigma_um=[280,280,280] \
--set film_render.halation.halation_n_bounces=2 \
--set film_render.halation.halation_bounce_decay=0.45 \
--set film_render.halation.halation_strength=[0.15,0.045,0.0] \
--set film_render.grain.rms_granularity=[4.5,7.0,9.0]
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

GUI state version 5, preset schema 6, and recipe schema `spektrafilm-rs-params-6` own the scanner cutover. The additive `scanner.operator` group uses these same versions; omitted values default to `0, 0, 0, 1`. The additive `scanner.white_reference` leaf also uses those versions; an omitted value defaults to `clear_film_base`, and existing documents retain their pixels. Saved corrections contain no automatic offsets. Unknown and retired fields remain errors. Explicit older versions migrate `scanner.lamp` out of the readout controls. Complete captures preserve it as the Convert acquisition illuminant unless already declared. Current versions reject `scanner.lamp`. The built-in `neutral` spelling becomes `standard`; this migration intentionally changes the automatic scan output.
