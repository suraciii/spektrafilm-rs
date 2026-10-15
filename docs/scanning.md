# Scanning and negative conversion

This page explains scan output modes, negative conversion, and film-scan controls. It also describes the experimental CineStill 800T Cs41 material and look.

## Experimental workflow routes and output modes

Runtime and GUI state accept passthrough, film-scan, film-print-scan, and the three convert-film routes. Direct negative-film scans expose `scanner.scan_output=direct_scan|negative_to_positive`.

`direct_scan` preserves the medium's polarity. Calibrated `negative_to_positive` supports direct `input > film > scan` and real negative-capture `input > convert-film > scan`. It rejects paper, positive-film, print-paper, scan-minus-base, white/black-corrected, and active-Grain-Procedural combinations. Convert-film inverts the spectral scan model with bounded Gauss-Newton. It supports scan illuminant, exposure, and calibration controls. It can reuse the calibrated positive interpretation on its direct scan route.

`scanner.scan_output` accepts `direct_scan` and `negative_to_positive`. Replace the removed `positive_scan` value in CLI commands, parameter files, presets, recipes, and GUI states. Old values fail validation. The GUI displays **Preserve medium polarity** and **Negative to positive**. It displays **Negative conversion method** for the `scanner.negative_conversion_method` field; the former `scanner.positive_interpretation` spelling migrates only through versioned preset and GUI state imports. Conversion includes the calibrated inverse and base positive tone. Automatic correction and the optional LabScan finish remain separate. This rename preserves the existing methods, defaults, and numerical behavior. See [scanner language](../specs/film-chain/spec.md#scanner-language).

## Negative conversion

The following command bypasses printing. It selects the receiver, inverse, automatic setup, and finish explicitly.

```bash
# Negative conversion with explicit receiver, inverse, setup and finish
./target/release/spektrafilm-f64 process input.tif -o positive.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --set scanner.receiver_profile=lab_scan_approx,scanner.negative_conversion_method=coupled \
    --set scanner.auto_setup=per_frame,scanner.finish=lab_scan,scanner.finish_strength=0.75 \
    --scan-output negative_to_positive --data-dir data

# Inspect effective scanner selections without decoding or writing output
./target/release/spektrafilm process input.tif -o positive.tif --film kodak_portra_400 --scan-film \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --scan-output negative_to_positive --set scanner.receiver_profile=lab_scan_approx,scanner.finish=lab_scan --dry-run --data-dir data
```

`negative_to_positive` reads each dye record's density like a narrowband lab scanner. It inverts that density through the record's own profile density curve. It then applies a print-like logistic tone curve. A record never borrows density from the others, so saturated negatives do not clip one channel. The inverse is profile-derived and monotone. It has finite endpoint slopes beyond the curves. The logistic tone keeps output within the paper range.

`scanner.conversion_brightness_ev` controls brightness in stops. `scanner.conversion_balance_ev` supplies independent RGB exposure offsets. Zero renders a gray lit by the film's reference illuminant neutral. `scanner.conversion_contrast` and `scanner.conversion_dmax` control the tone curve. Scanner lens blur acts on transmitted light before the density reading.

`scanner.negative_conversion_method` selects the method. `density`, the default, reads record densities as described here. It ignores `scanner.receiver_profile`. `separable` and `coupled` interpret the receiver capture instead. `coupled` uses a three-channel inverse sampled from the resolved film, receiver, and output response.

`density_direct` reads the same record densities into the scanner's own positive response without inverting the film's characteristic curves, so the film's tone and colour stay visible in the result. Use it when the picture should show the film rather than a reconstruction of the scene; its black and white points, brightness, balance and contrast are yours to dial, exactly as the controls above describe. A real photograph has no detectable black or white point, so set them by eye on the frame you are converting.

These controls apply to direct negative-film scans and the `input > convert-film > scan` route. Print-paper and scan-minus-base routes retain their existing route semantics. The displayed result interprets film density. It is not a measurement of the film or a match guarantee for a processed reference JPEG. Hold profile, exposure, white balance, scan illuminant, and scan settings fixed when comparing results.

For a real negative scanner capture on `input > convert-film > scan`, disable virtual camera metering with `camera.auto_exposure=false`. Supply input transfer and color interpretation. Supply the matching `film_render.convert.scan_illuminant`. Convert-film estimates the record densities that the negative converter reads.

## Digital Reference

Digital Reference is an optional modeled conversion for simulated negative film on `input > film > scan`. It uses a common spectral receiver and fixed positive response. It preserves film-record and dye differences through a shared output convention.

Select both `scanner.negative_conversion_method=digital_reference` and `scanner.receiver_profile=digital_reference`. Keep `scanner.auto_setup=off`. Keep identity black/white points. Keep the reconstruction controls at their defaults. The GUI pairs these selections under **Digital Reference (modeled)**. It shows the fixed **Reference Exposure** placement.

`scanner.reference_exposure_ev` adjusts output exposure in stops. `scanner.reference_balance_ev` supplies independent linear-sRGB RGB gains before output-space conversion. Both default to zero. Optional `scanner.finish=lab_scan` applies after conversion.

See the [Digital Reference contract](../specs/film-chain/digital-reference/spec.md) for scope. See the [reference model](../specs/film-chain/digital-reference/design.md#numerical-reference-model) for spectral sources, normalization, response, and provenance.

```bash
./target/release/spektrafilm-f64 process input.tif -o reference.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --film kodak_portra_400 --scan-film --scan-output negative_to_positive \
    --set scanner.negative_conversion_method=digital_reference,scanner.receiver_profile=digital_reference \
    --backend cpu --data-dir data
```

## Film-scan black and white points

Film scans expose per-channel `scanner.black_point` and `scanner.white_point` for slide film and converted negatives. Defaults `[0,0,0]` and `[1,1,1]` preserve the existing rendering. Each channel must satisfy `0 <= black_point < white_point <= 1`.

Points stretch the shared relative log-exposure range for negatives. They stretch the shared scanned-density range for slides. Values outside the points continue the same map without clipping. On slides, non-default points run after scanner lens blur and before white/black correction. They do not change that correction's references. Paper scans and direct negative captures ignore these points. See [film-scan points](../specs/film-chain/spec.md#film-scan-black-and-white-points) for the range definitions.

```bash
./target/release/spektrafilm-f64 process input.tif -o slide.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --film fujifilm_provia_100f --route 'input > film > scan' --backend cpu \
    --set 'scanner.black_point=[0.05,0.05,0.05],scanner.white_point=[0.95,0.95,0.95]' \
    --data-dir data
```

## Experimental CineStill 800T Cs41 material

`cinestill_800t_cs41_approx` is an experimental fitted approximation of CineStill 800T in Cs41.

The material profile computes developed dye amounts and spectral density before printing or scanning. It does not provide measured CineStill spectral sensitivity, dye spectra, grain, or halation. Keep grain disabled for material comparisons. Compare halation separately.

The model exposure origin does not establish absolute ISO. It does not prove that the manufacturer's reference is an 18% gray exposure. Metadata records the sources, approximations, and attribution. The derivative retains Andrea Volpato's CC BY-SA 4.0 asset license.

The stock-specific entry in [coupler initialization data](../data/presets/couplers.toml) gives 800T the 500T tungsten DIR gamma coefficients during ordinary stock selection. These coefficients are inherited model priors, not measured Cs41 coupler data. Explicit user controls retain precedence. This does not change the stock's `still` classification, grain, halation, or DIR diffusion defaults, and does not require a creative preset.

### Portra Endura print baseline

The `kodak_portra_endura` / `TH-KG3` / `cinestill_800t_cs41_approx` entry in the [print-filter database](../data/filters/neutral_print_filters.json) supplies the default C/M/Y pack for this combination. Explicit filter controls override the corresponding database axes. Other papers retain their own entries.

This pack is an empirical reference-appearance baseline, not a measured neutral-filtration specification. The controlled material comparison fixes exposure, paper response, and the inherited DIR coefficients, with spatial and stochastic effects disabled. It uses the native product direct scan and standard sRGB output, without an external paper-white transform. The pack is nondominated on unrounded objectives among the compared configurations, but not when the Lab and color-error objectives are rounded to 0.01 bins. It remains a conservative baseline without an additional Cs41 curve correction, not a robust or global optimum. Some individual patches become less accurate.

The pack does not change film response curves, paper profiles, exposure controls, color-space conversion, or scanner readout. The comparison does not validate other papers, arbitrary scanner or look settings, or a unique physical film response. It does not introduce a creative preset or a per-patch correction surface.

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
- [Digital Reference contract](../specs/film-chain/digital-reference/spec.md)
- [Project context](../CONTEXT.md)
