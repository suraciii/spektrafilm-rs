# Lab scan readout

This contract owns the readout of developed photographic material. It does not own film exposure, development, print exposure, or output encoding.

## Readout routes

1. Negative-to-positive readout reconstructs relative log exposure from each record's density. It applies frame setup when the conversion style is `standard`, then a positive tone.
2. Positive transmission readout integrates the material's spectral transmittance under the readout illuminant with the CIE 1931 2° observer.
3. Paper reflection readout uses the same integration with spectral reflectance instead of transmittance.

The negative-to-positive reconstruction must remain per-record. A change in one record must not change another record's reconstructed exposure. Frame setup is separate and may depend on all pixels and records.

## Readout illuminant

The readout illuminant is a product constant. The material category selects it:

| Material category | Illuminant | Reference |
| --- | --- | --- |
| `stage = printing`, `use = cine`, `channel_model = color` | `BB5800` | ISO 2910:2018 §6 |
| All other materials | `D50` | ISO 3664 |

The readout must not apply a device infrared filter. It must not accept a runtime readout-light parameter or a readout-light field in a material profile. Current-schema documents that carry `scanner.lamp` must fail validation.

The readout illuminant must not change film exposure, development, or reconstructed record exposure. It must not depend on image content, the capture illuminant, or the enlarger illuminant. It must participate in scan preparation and its cache identity.

Positive transmission and paper reflection readout must use the clear material base as the white reference. The readout must adapt that white to the output color space white. It must not apply a selectable receiver profile or scan calibration matrix.

Paper readout must compute color from spectral reflectance. The densitometer convention describes profile measurement; it must not replace the reflectance integration.

## Exposure reference and inverse

The negative-film exposure coordinate uses the normalized camera reference: a scene-linear neutral value of `0.184` under the stock reference illuminant maps approximately to `log_e = 0`. The target coordinate for frame setup is

\[
x_{aim}=\log_{10}(0.16/0.184).
\]

This is an exposure target. It does not require the output RGB value to equal `0.16`. The fixed positive tone determines that value.

The inverse must use the density domain the film stage emits: each record over its own curve minimum. It must regularize reversals with least-squares isotonic regression. Pooled plateaus use mean log exposure. Endpoints use finite tangent extrapolation. Setup must use the neutral locus and slope from these same regularized axes.

## Standard frame setup

`standard` is the built-in automatic style. It must measure the complete frame delivered to the scan acquisition boundary, after transmission-domain scanner lens blur and before the positive tone. It must not measure separate tiles. It must not cache offsets across frames.

The following constants are declared product choices. They are not calibrated Frontier or Noritsu settings.

| Constant | Value |
| --- | --- |
| Exposure correction slope | `0.75` |
| Color correction slope | `0.9` |
| Color-weight floor | `0.2` |
| Color-distance coefficient | `40` |
| Contrast branch threshold | `1.8` density units |

### Exposure

For each pixel, let `N = (d_C + d_M + d_Y) / 3`. Compute the arithmetic mean `Nave`, minimum `Nmin`, maximum `Nmax`, and fifth percentile `N5`. Use linearly interpolated percentile ranks `(count - 1) * 0.05`.

Select weights `w = (0.33, 0.30, 0.35)` when `Nmax - N5 < 1.8`. Otherwise select `(0.30, 0.35, 0.29)`.

\[
E_n=\frac{w_0N_{ave}+w_1N_{max}+w_2N_{min}}{w_0+w_1+w_2},\qquad
\Delta x=0.75\frac{aim-E_n}{\gamma}.
\]

Here `aim` is the mean neutral density at `x_aim`. `gamma` is the mean local neutral-curve slope there. The branches sum to `0.98` and `0.94`; normalize each by its own sum. Reject a nonfinite or nonpositive slope.

This estimator uses arithmetic density statistics. It is not the physical LATD statistic `-log10(mean(10^-d))`.

### Color

For each pixel, reconstruct `x_C`, `x_M`, and `x_Y`. Evaluate the stock neutral locus at their mean exposure. Subtract that neutral density from the pixel's record density. Average those residuals over the frame to obtain `r`.

\[
u=(r_C-r_M)/\sqrt{2},\quad
v=(r_C+r_M-2r_Y)/\sqrt{6},\quad
W=\max(1-40(u^2+v^2),0.2).
\]

Set `du = -0.9 W u / gamma` and `dv = -0.9 W v / gamma`. The log-exposure offsets are

\[
\Delta x_c=\Delta x+\left(
 du/\sqrt2+dv/\sqrt6,\;
-du/\sqrt2+dv/\sqrt6,\;
-2dv/\sqrt6\right)_c.
\]

A neutral ramp must not acquire a cast solely because the stock's three neutral curves differ. A dominant scene color must reduce the correction through `W`. This neutral-locus approximation does not reproduce a stock-specific population average of real negatives.

### Execution

Setup must accumulate statistics in f64 in raster order. It must reject empty frames, nonfinite densities, and nonfinite corrections. It must not modify input densities or upstream diagnostic taps.

The same setup implementation must serve per-stage CPU execution and the assembled resident scan acquisition fence. It must not silently switch a requested GPU render to a CPU render. The existing mixed execution fence remains visible in backend reporting.

`standard` must not subtract camera exposure compensation again after automatic setup. Fixed styles retain the historical camera compensation on simulated film-scan routes. Real captures receive no virtual camera compensation.

## Tone and style names

Apply the existing logistic family to `x'_c = x_c + delta_x_c`. Fixed styles use a zero setup offset. `standard` keeps the historical tone constants in all three channels:

| `brightness_ev` | `contrast` | `dmax` |
| --- | --- | --- |
| `0.8781766618` | `0.73551400058216643` | `1.5625841465572565` |

The only built-in style name offered by discovery and the GUI is `standard`. Accept `neutral` as an input-only migration alias. Saved runtime snapshots and effective configuration must emit `standard`. The alias does not preserve the historical fixed-curve pixels.

Any other valid name resolves to `<data-dir>/scan-styles/<name>.json`. A file declares three finite tone triples with positive `contrast` and `dmax`. It runs no frame setup. Invalid or missing style data must fail before output publication. Style names must not contain `/`, `\`, `..`, or `:`. A catalog must not offer files named `standard` or `neutral` as additional choices.

`direct_scan` must not apply conversion setup or tone. It admits the built-in name without effect. Custom fixed styles remain valid only with `negative_to_positive`.

## Point-wise consumers and migration

A `negative_to_positive` render with `standard` must fail LUT baking. A point-wise measurement whose range reaches `rgb_out` through this setup must fail before writing measurement output. Both failures must identify frame dependence and direct the caller to a declared fixed style. Earlier film-density or exposure taps remain measurable.

GUI state version 5 owns the cutover. Versions 1–4 may discard retired `lamp` keys from scanner sections during migration. Current version 5 and unversioned current documents must reject them. Style aliases normalize on load and save. This migration intentionally changes scan appearance.

Preset schema 6 owns the same scanner cutover for versions 1–5. It must validate historical scanner provenance against the built-in model before replacing it. It must reject forged device metadata. Current schema 6 rejects `lamp` and retains all unrelated retired-field rejection. Sparse sections must preserve omission when normalizing the style alias.

## Acceptance and limits

- A uniform 16% neutral patch has a setup offset of zero within precision tolerance.
- Gray ramps remain neutral, monotone, and finite. Positive and negative exposure shifts remain distinguishable. Exact residual `(1 - slope)` applies to linear response curves, not arbitrary toe/shoulder regions.
- Repeated execution on the same frame produces identical setup and pixels at the same precision and backend.
- Changing a frame's other pixels may change a pixel's output under `standard`. Fixed styles retain record independence.
- A dominant red field remains red after correction. A dark field must not be forced to the same result as the neutral aim.
- A bright background may still darken a subject. Setup provides no face detection, subject classification, or local tone correction. Do not weaken this limit into a claim that inverse-lighting scenes are rescued.
- Backend parity, style changes, canonical alias output, versioned migration, and point-wise rejection require real-consumer tests.
- No paired real minilab scans are available. Passing these tests does not establish a Frontier/Noritsu match or prove this algorithm best.
