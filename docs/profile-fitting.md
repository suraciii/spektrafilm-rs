# Profile fitting

`spektrafilm-f64 fit` derives film-profile surface terms and negative-conversion tone values from reference patch data instead of hand measurement. It renders a chart through the calibrated chain and minimizes the CIELAB difference between the rendered patches and the reference table. The command exists only in the f64 reference binary.

## Inputs and fitting

Inputs are three JSON files:

- A chart layout with the patch grid and index ranges.
- A scene patch table in the declared input color space.
- A reference patch table of the developed film in linear sRGB.

Each patch table names the `source` of its values. The fitted surface learns the errors of the scene table. A surface fitted from camera readings therefore describes that camera as well as the stock.

Halation and grain are off in every evaluation. The scanner writes linear sRGB. Fitting the surface enables the surface toggle. `--lambda` must be finite and non-negative. See the [profile fitting contract](../specs/profile-fit/spec.md) for the fitting requirements.

A run succeeds only when the fit reduces both the color and the gray ΔE00 medians. A surface-only fit can trade gray for color. For this reason, `--fit surface,tone` is the usual choice.

## Outputs

A successful run writes:

- A profile copy with the fitted surface and a provenance sentence in `metadata.datasource`.
- A sparse parameter overlay with the fitted tone.
- A report with the metrics before and after the fit, the per-patch differences, the input SHA-256 values and sources, the resolved chain, the forced settings, and the tool version.

A failed run leaves none of these outputs.

## Cross validation

`--cv` adds a cross validation of the color patches. It performs one extra fit per fold. Each fit trains on half of the color patches and is measured on the other half.

The command reports the training and held-out medians of both folds. It leaves the written profile and overlay from the fit over every patch. See the [cross-validation contract](../specs/profile-fit/spec.md#cross-validation).

## Shipped Portra 800 fit

The shipped `kodak_portra_800` surface came from this command; its recorded metrics live in the profile's `metadata.datasource`.

## Example

```bash
./target/release/spektrafilm-f64 fit \
    --layout layout.json --scene scene.json --reference reference.json \
    --fit surface,tone --surface-terms 9 --lambda 1.0 \
    --preset "CineStill 800T Cs41 (experimental)" \
    --route "input > film > scan" --scan-output negative_to_positive \
    --input-color-space "ProPhoto RGB" --input-cctf-decoding false \
    --data-dir data \
    --out-profile fitted_profile.json --out-overlay fitted_tone.json \
    --report fit_report.json
```

## Fitted density models

All fitted density models refresh sampled curves on load, including `sept_norm_cdfs` with per-layer median-preserving skew parameters.

## See also

- [Profile fitting contract](../specs/profile-fit/spec.md).
- [Cross-validation contract](../specs/profile-fit/spec.md#cross-validation).
- [Building](building.md).
- [Scanning](scanning.md).
- [Presets](presets.md).
