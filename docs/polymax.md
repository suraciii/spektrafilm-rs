# Kodak Polymax Fine-Art paper

The print-material selector includes Kodak Polymax Fine-Art F-surface paper.
One entry uses the published **No filter** condition. It is a fixed-condition
source-curve approximation, not a continuously variable-contrast emulsion model.
It works with the existing photographic-print route and needs no look preset.

## Source and scope

The source is Kodak's [G-24, June 2005](https://125px.com/docs/paper/kodak/g24.pdf),
page 5. All curves use F surface, DEKTOL 1:2, 90 seconds, and 20°C.
The general processing instructions on page 3 describe other conditions; they do
not replace the conditions printed beside these curves.

The source supplies reflection-density curves for no filter and filters −1,
0, 1, 2, 3, 4, and 5. The last label follows the graph's `#5`; its correspondence
to the table's `5+` remains uncertain. Half-step curves are not supplied or synthesized.
Only the unfiltered curve is shipped as a selectable profile. The other curves
remain research source data, not additional materials or development times.

The generator extracts the PDF's vector centerlines. It does not trace a raster,
copy another stock, or fit final RGB appearance. The
[source record](../data/profile_sources/kodak_polymax_fine_art.json) retains
all characteristic curves and all three density-threshold sensitivity curves.
The latter describe densities above D-min, not independent emulsion channels.

The runtime uses the identified D-min + 0.6 sensitivity curve as one effective
response. Its shape is resampled by PCHIP on the native 380–780 nm, 5 nm grid.
Endpoint log slopes extend unmeasured tails to a numerical floor of −12.
These tails are extrapolations, not measured cutoffs.

## What the model reproduces

- The published unfiltered characteristic curve.
- The source's white-end and black-end scalar reflection densities.
- A fixed-spectrum exposure response, most directly applicable to a spectrally neutral negative.

The profile has an increasing sampled density curve and no fitted curve model.
Its exposure origin is the unfiltered curve's paper-white-relative 0.184
reflectance point. The preparation report records the origin in source
log10 lux-seconds. This is a relative engine coordinate, not an absolute enlarger timer.

PCHIP preserves the centerline vertices. The generator also samples between them
for the renderer's linear lookup. It clamps outside each plotted exposure interval.
An equal-weight isotonic pass can remove drawing reversals; the report records
any adjustment. Numerical agreement with the published drawing is not physical
accuracy against a particular box of paper.

The silver-image basis and paper base are wavelength-neutral approximations.
The characteristic curve has its own D-min removed once, and the base stores that
D-min once. Reflection density already includes the paper's reflected-light response;
the renderer does not add a second double-pass attenuation.

The existing readout normalizes the paper base to output white. Final pixels are
therefore paper-white-relative, not absolute measured reflectance. This profile
does not change that product-wide readout contract.

## Limits

Kodak documents optical brighteners in the emulsion and base. No measured
excitation/emission data, complete paper-white spectrum, or density-dependent
silver-image spectra enter this profile. Surface gloss, texture, fluorescence,
toning, aging, reciprocity, and stock-specific grain are not calibrated.
F surface is glossy when ferrotyped and otherwise can dry to a smooth lustre;
no directional surface model is inferred from its name.

The exact plotted lamp spectrum and filter-transmission curves are unavailable.
The model uses one effective sensitivity. Changing enlarger light or
filters changes effective exposure, but does not predict physical variable contrast.
Colored or stained negatives can also exceed this fixed-spectrum approximation.
A numbered-filter spectrum alone cannot recover the other source curves.

Black-and-white paper uses zero automatic neutral CMY values unless an axis was
explicitly edited. Disabling automatic database filters preserves the configured
values instead. Existing color materials and black-and-white print films retain
their baseline policy. Color-neutralization calibration is unavailable for
black-and-white printing materials because RGB chroma cannot identify their contrast.

Gamma and other artistic controls remain edits to the source response, not
physical paper-grade calibration. Fitted-model chemistry is inactive because this
profile uses an authoritative sampled curve. No development-time selector is offered
for this fixed-process material.

## Use

The stock is `kodak_polymax_fine_art`. No filter-variant stocks are shipped.

```sh
spektrafilm process input.png -o polymax.png \
  --film kodak_doublex --paper kodak_polymax_fine_art \
  --route 'input > film > print > scan' --scan-output direct_scan \
  --backend cpu --data-dir data \
  --set 'enlarger.illuminant=T,enlarger.c_filter_neutral=0,enlarger.m_filter_neutral=0,enlarger.y_filter_neutral=0,enlarger.m_filter_shift=0,enlarger.y_filter_shift=0'
```

`T` is the runtime's reference-light convention, not a measured reconstruction of
Kodak's test enlarger. Match the input color space and transfer decoding to the file.
For source-response comparisons, disable spatial/stochastic effects, glare, automatic
camera exposure, preflash, and artistic tone changes. Use print gamma 1 and hold
lamp and filtration fixed. Compare paper densities before readout normalization.

## Rebuild

Use an external Python environment with NumPy and SciPy. The generation run was
checked with NumPy 2.5.4 and SciPy 1.18.1. Normal regeneration needs only checked-in
source data and the pinned runtime `T` illuminant. It executes no downloaded code.

```sh
python3 scripts/prepare_polymax.py \
  --output-dir /tmp/polymax-profiles \
  --report /tmp/polymax-preparation.json
```

The report contains source and generator hashes, package versions, the
exposure origin, endpoint densities, monotonic adjustments, and
runtime interpolation residuals. Repeated generation in the same numerical
environment produces identical profile bytes. Different library versions can
change final digits; compare reported residuals as well as bytes.

To reproduce source extraction, download G-24 and the
[pinned cross-check source](https://raw.githubusercontent.com/JanLohse/spectral_film_lut/6307a98f3cf4f2d7b6d09bab82fbf6f64356ccfe/src/spectral_film_lut/bw_print_film/kodak_polymax_fine_art.py)
outside the repository. The extractor requires Poppler's `pdftocairo` and NumPy.
It rejects different source hashes. Its path checks intentionally fail if the
SVG conversion changes rather than silently assigning different curves.

```sh
python3 scripts/extract_polymax.py \
  --pdf /tmp/g24.pdf \
  --digitization /tmp/kodak_polymax_fine_art.py \
  --output /tmp/kodak_polymax_fine_art.json
```

The manufacturer PDF is not redistributed. Read the
[attribution notice](../data/license/POLYMAX.txt) before distributing these assets.
