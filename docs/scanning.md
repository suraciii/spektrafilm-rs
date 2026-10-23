# Scanning and negative conversion

This page explains the scan model, scan output modes, negative conversion, and film-scan controls. It also describes the experimental CineStill 800T Cs41 material and look.

## Neutral scan model

A scan is one colorimetric capture. The scanner has one sensor, modeled on the CIE 1931 color-matching functions, and one light source: the scanner lamp. There is no receiver profile to choose, no scanner calibration matrix, and no automatic setup or scanner finish.

`scanner.lamp` names that light source. It accepts the shared illuminant vocabulary — `D50`, `D55`, `D65`, `T`, `TH-KG3`, `TH-KG3-L`, `K75P`, and a `BB<kelvin>` blackbody token — and defaults to `D55`. The lamp is the same vocabulary the enlarger illuminant uses, but the two are independent: changing the enlarger lamp does not change the scan, and changing the scanner lamp does not change the print.

The white reference of a scan is the clear film base of the scanned medium. A clear area of that base reads as the output color space's white, and the scan chromatically adapts from the lamp's white to that output white. The scan matrix is the identity. Because the lamp is the only light in the acquisition, changing it changes the captured colors of a colored area while a clear area stays the output white.

The scanner controls are `scanner.lamp`, `scanner.scan_output`, `scanner.lens_blur`, and `scanner.unsharp_mask`. Lens blur models readout optics on linear transmitted light and runs before any density interpretation. Unsharp mask is digital sharpening and runs after interpretation and output gamut processing. White/black correction, white/black levels, film-scan black and white points, receiver profiles, negative conversion methods, automatic setup, scanner finish, and Digital Reference are removed; their field names fail validation at the current schema version.

## Experimental workflow routes and output modes

Runtime and GUI state accept passthrough, film-scan, film-print-scan, and the three convert-film routes. Direct negative-film scans expose `scanner.scan_output=direct_scan|negative_to_positive`.

`direct_scan` preserves the medium's polarity. Calibrated `negative_to_positive` supports direct `input > film > scan` and real negative-capture `input > convert-film > scan`. It rejects paper, positive-film, print-paper, scan-minus-base, and active-Grain-Procedural combinations.

`scanner.scan_output` accepts `direct_scan` and `negative_to_positive`. Replace the removed `positive_scan` value in CLI commands, parameter files, presets, recipes, and GUI states. Old values fail validation. The GUI displays **Preserve medium polarity** and **Negative to positive**. See [scanner language](../specs/film-chain/spec.md#scanner-language).

## Negative conversion

The following command selects the scan output and lamp explicitly.

```bash
# Negative conversion with an explicit scanner lamp
./target/release/spektrafilm-f64 process input.tif -o positive.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --set scanner.lamp=D55 \
    --scan-output negative_to_positive --data-dir data

# Inspect effective scanner selections without decoding or writing output
./target/release/spektrafilm process input.tif -o positive.tif --film kodak_portra_400 --scan-film \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --scan-output negative_to_positive --set scanner.lamp=D55 --dry-run --data-dir data
```

`negative_to_positive` reads each dye record's density like a narrowband lab scanner. It inverts that density through the record's own profile density curve. It then passes through one logistic positive tone curve. A record never borrows density from the others, so saturated negatives do not clip one channel. The inverse is profile-derived and monotone. It has finite endpoint slopes beyond the curves. The tone keeps output within the paper range.

There is one negative conversion interpretation. It has no selectable method. The tone curve's constants come from a declared conversion style, described below. Zero balance renders a gray lit by the film's reference illuminant neutral. Scanner lens blur acts on transmitted light before the density reading.

Negative conversion applies to direct negative-film scans and the `input > convert-film > scan` route. Print-paper and scan-minus-base routes retain their existing route semantics. The displayed result interprets film density. It is not a measurement of the film or a match guarantee for a processed reference JPEG. Hold profile, exposure, white balance, scanner lamp, and scan settings fixed when comparing results.

For a real negative scanner capture on `input > convert-film > scan`, disable virtual camera metering with `camera.auto_exposure=false`. Supply input transfer and color interpretation. Convert-film estimates the record densities that the negative converter reads.

## Conversion styles

`scanner.conversion_style` names the stylization of the negative-to-positive tone. It defaults to `neutral`, which is the frozen reference tone and renders byte-identically to the conversion without a style slot. `neutral` needs no data directory.

Any other name resolves to `data/scan-styles/<name>.json` in your data directory. No style ships with the product: with a stock data directory, only `neutral` resolves, and every other name fails with an error that lists the available styles.

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

A style parameterizes the constants of the same logistic curve family the `neutral` style uses, and it never adds a curve family. It is never fitted from a photograph or a chart, and it is not a measured or calibrated scanner response. The film's own tone stays visible under every style, and per-record independence and density monotonicity hold.

A non-neutral style requires `scanner.scan_output = negative_to_positive`. A `direct_scan` does not read the style slot, so the style is inert there. Style names must not be empty and must not contain `/`, `\`, `..`, or `:`. The style resolves once when the pipeline is built.

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

The resolved style appears in the effective configuration of the render report, alongside the scan output mode and the scanner lamp.

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
