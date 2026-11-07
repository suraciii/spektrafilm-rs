# Fujifilm Astia 100F

`fujifilm_astia_100f` approximates FUJICHROME ASTIA 100F Professional [RAP100F]
from published curves. It is not ASTIA 100, a Fujifilm digital-camera simulation,
or a measured match to an exposed film and scanner.

The built-in **Fujifilm Astia 100F** look uses neutral finishing and native positive
scanning. It enables fine Spektra grain and restrained halation and scatter.
It leaves DIR, chemistry changes, creative diffusion, print glare and scanner
sharpening off. Applying it preserves camera exposure and
metering. Its stored Crystal Archive paper reference satisfies the portable
look format; paper does not contribute on the default `input > film > scan`
route. Normal whole-look loading still requires that referenced profile file.
The look sets `use_cat16=false` to use CAT02 consistently with the CPU input
adapter and its calibration. This does not disable chromatic adaptation.

An already declared film-scan or convert-film route is preserved by the shared
preset resolver. For an ordinary scene photograph, select `input > film > scan`.
The positive material uses `direct_scan`; negative inversion is not eligible.
Direct material selection alone does not apply the built-in look's effect policy.

## Ready-to-use texture

The look sets Spektra RMS to `[7, 7, 7]`. Fujifilm reports visual RMS 7 at gross
visual density 1.0 through a 48 micrometer aperture. Spektra uses that aperture
scale but sizes particles from peak modeled variance. Equal channel targets are
a practical approximation, not measured channel granularity or final RGB noise.
The source profile retains one aggregate record per color.

The look uses grain post-blur 0.2 pixels and disables multiplicative grain USM.
This avoids the strong smoothing and edge enhancement of the generic settings.
Dye-cloud blur remains 2 micrometers. Halation amount is 0.2 and scatter amount
is 0.25, relative to the generic model. These restrained strengths approximate
a fine-grained reversal stock with strong antihalation; they are not measured
Astia point-spread functions. No creative diffusion filter is added.

Use Scan or a saved render to inspect Spektra texture: Preview disables this
engine by design. Grain scale follows the caller's film format and image sampling.
The look preserves camera exposure, metering and format rather than resetting
them. CPU and GPU Spektra produce different random realizations; do not expect
pixel equality between them. See [Grain](grain.md).

## Sources and reconstruction

The primary reference is Fujifilm's
[AF3-149E, EIGI-03.11-HB](https://125px.com/docs/film/fuji/astia_100f_datasheet.pdf),
page 8. The PDF SHA-256 is
`a3ed5826d934743b91d750a7ce6c79041c8b3ba0b1afc05ff3cf0b8d175e0809`.
The characteristic curves specify daylight, 1/50 second, E-6/CR-56 and Fuji
FAD-30S (Status A). The sensitivity curves specify density 1.0 above D-min.
The dye curves specify separated-light exposure and spectral diffuse density.
The April 2003 English edition has identical centerline geometry for these nine
curves, despite different stroke colors. The Japanese edition is a qualitative
cross-check, not an independent physical measurement.

The extractor samples the original Bézier centerlines. Affine axis mappings fit
all labelled ticks because the printed grid spacing is slightly inconsistent.
The source file retains paths, coordinates, ticks and hashes. Source validation
rejects substituted geometry, axes, coordinates and instrument tables.

The generator samples spectra at 5 nm from 380 to 780 nm. PCHIP interpolates
within the plotted domains. Endpoint slopes extend log sensitivity and linear
dye density outside those domains; outward growth is prevented. Log sensitivity
has a numerical floor of -12 and dye density is nonnegative. These tails are
assumptions, not measured cutoffs.

The photographic curves contain RGB densitometer readings, not CMY dye amounts.
The generator solves nonnegative dye amounts through spectral transmittance:

\[
D_k=-\log_{10}\sum_\lambda W_k(\lambda)
10^{-b(\lambda)-\sum_j a_jq_j(\lambda)},\qquad \sum_\lambda W_k=1.
\]

The legacy Status A tables come from the pinned MIT-licensed transcription
identified in the source JSON. They are interpolated and extrapolated in log space at 1 nm, exponentiated,
then integrated with Simpson quadrature against 5 nm linear transmission basis
functions and normalized. This is a Status A approximation, not certification
against ISO 5-3 reference weighting factors.

A constant neutral base prior uses the minimum of the common clear-end curve.
Isotonic regression removes small drawing reversals from the dye amounts. Each
record's residual minimum moves into the effective clear-film spectrum exactly
once, because runtime development subtracts record minima. This inferred base
is not a measured support spectrum. The 1,025 sampled exposure rows are
canonical; there is no analytic model that overrides them on load. Each channel
has one aggregate record, not a fabricated decomposition into emulsion layers.
The curves clamp outside the publication domain rather than adding latitude.

Exposure zero is the reconstructed green Status A clear-end density plus 1.0.
The stored reference is about -1.09819 log10(lux-seconds). D55 RGB midgray maps
to this coordinate. Neither the coordinate nor ISO 100 metadata calibrates
absolute scene exposure or fixes output brightness. D55 is an implementation
convention for the source's unspecified daylight spectrum.

The existing RGB adapter algorithm is refitted to Astia sensitivity, using
99 CIE 2017 reflectances and a separate 24-patch BabelColor evaluation set.
The built-in enables the fitted window and surface. These correct spectral
reconstruction errors, not film color or a camera/scan measurement. The red
holdout error remains approximately 0.153 stops RMS and 0.538 stops maximum
with the surface. Its RMS improves over window-only, but its maximum worsens.
Do not interpret a small density-curve residual as accurate reproduction of
arbitrary colored scenes.

## Rebuild

Use Python 3.13, NumPy 2.5.4, SciPy 1.18.1 and colour-science 0.4.7 in an
external environment. Obtain the primary PDF and the pinned `densiometry.py`
source URL recorded in the source JSON. The extractor parses only its Status A
literal; it never executes downloaded Python.

```sh
python3 scripts/extract_astia100f.py \
  --pdf /path/to/astia_100f_datasheet.pdf \
  --status-source /path/to/densiometry.py \
  --output /tmp/astia-source.json
cmp /tmp/astia-source.json data/profile_sources/fujifilm_astia_100f.json
OPENBLAS_NUM_THREADS=1 PYTHONDONTWRITEBYTECODE=1 \
  python3 scripts/prepare_astia100f.py \
  --output /tmp/astia-profile.json --report /tmp/astia-report.json
cmp /tmp/astia-profile.json data/profiles/fujifilm_astia_100f.json
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_prepare_astia100f.py
```

The report records input, generator, observer, illuminant, adapter and output
hashes, package versions, reconstruction residuals and RGB adaptation errors.
Different numerical-library versions may change final digits.

```sh
spektrafilm process input.tif -o astia.png \
  --preset 'built-in:Fujifilm Astia 100F' \
  --route 'input > film > scan' --backend cpu --data-dir data
```

Set input color space and transfer decoding to match the file. Fixed-exposure
comparisons must explicitly disable camera auto exposure. Input gamut controls
remain caller context and can change the adapter result. The material's D50
readout uses the product's clear-film white normalization and chromatic
adaptation; it is not an uncorrected transmittance measurement.

## Limits

- Published curves represent typical production, not a particular batch or aged roll.
- Base spectrum, spectral tails and additive dye separation remain assumptions.
- White-light curves do not identify MCCL/HTILE or nonneutral layer coupling.
- Manufacturer RMS granularity and MTF do not calibrate runtime grain or blur.
- RGB input cannot distinguish metameric spectra that produce the same RGB.
- Numerical reconstruction and software consistency do not establish physical fidelity.

See the [source notice](../data/license/ASTIA_100F.txt) before redistribution.
