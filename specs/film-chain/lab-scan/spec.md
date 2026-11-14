# Lab scan readout

This contract owns the readout of developed photographic material. It does not own film exposure, development, print exposure, or output encoding.

## Readout routes

1. Negative-to-positive readout reconstructs relative log exposure from each record's density. It applies operator correction, then a positive tone.
2. Positive transmission readout integrates the material's spectral transmittance under the readout illuminant with the CIE 1931 2° observer.
3. Paper reflection readout uses the same integration with spectral reflectance instead of transmittance.

The negative-to-positive reconstruction MUST remain per-record. A change in one record MUST NOT change another record's reconstructed exposure.

No readout stage MAY derive a parameter from the complete frame's statistics. Every parameter MUST depend only on the material, the readout setting, and the operator values.

## Readout illuminant

The readout illuminant is a product constant. The material category selects it:

| Material category | Illuminant | Reference |
| --- | --- | --- |
| `stage = printing`, `use = cine`, `channel_model = color` | `BB5800` | ISO 2910:2018 §6 |
| All other materials | `D50` | ISO 3664 |

The readout MUST NOT apply a device infrared filter. It MUST NOT accept a runtime readout-light parameter or a readout-light field in a material profile. Current-schema documents that carry `scanner.lamp` MUST fail validation.

The readout illuminant MUST NOT change film exposure, development, or reconstructed record exposure. It MUST NOT depend on image content, the capture illuminant, or the enlarger illuminant. It MUST participate in scan preparation and its cache identity.

Positive transmission and paper reflection readout MUST use the clear material base as the white reference by default. The readout MUST adapt that white to the output color space white.

`scanner.white_reference = readout_illuminant` explicitly opts out to the readout-light Y and white, matching upstream experimental `28bf883`. It MUST be identified in provenance and does not claim clear-base calibration. The readout MUST NOT apply a selectable receiver profile or scan calibration matrix.

Paper readout MUST compute color from spectral reflectance. The densitometer convention describes profile measurement; it MUST NOT replace the reflectance integration.

## Imported capture illumination

`film_render.convert.scan_illuminant` describes the light that acquired an imported RGB image. It defaults to `D55` and uses the shared illuminant vocabulary and validator. It MUST affect convert-film inversion and calibration only.

It MUST NOT override the product readout illuminant. The Convert panel and saved state MUST retain this independent input declaration.

## Exposure reference and inverse

The negative-film exposure coordinate uses the normalized camera reference: a scene-linear neutral value of `0.184` under the stock reference illuminant maps approximately to `log_e = 0`. The target coordinate is

\[
x_{aim}=\log_{10}(0.16/0.184).
\]

This is the operator pivot. It does not require the output RGB value to equal `0.16`. The fixed positive tone determines that value.

The inverse MUST use the density domain the film stage emits: each record over its own curve minimum. It MUST regularize reversals with least-squares isotonic regression. Pooled plateaus use mean log exposure.

Endpoints use finite tangent extrapolation.

## Retired frame setup

The product MUST NOT execute an automatic frame setup. It has no runtime leaf, no control, and no execution on either backend. Automated exposure, cast, or contrast judgment is the user's [operator correction](#operator-correction).

The retired leaf `scanner.experimental_setup` MUST fail unknown-field validation in runtime parameters, sparse sources, inline assignments, current-schema presets, and imported recipes.

A future frame-dependent measurement requires a new contract. It MUST cite measured evidence on held-out material. It MUST state its effect on [LUT delivery](../../lut-delivery/spec.md).

Every style retains the historical camera compensation on simulated film-scan routes. Real captures receive no virtual camera compensation.

## Operator correction

`scanner.operator` MUST carry f64 `exposure_ev`, `warmth_ev`, `tint_ev`, and `contrast`. Their defaults MUST be `0`, `0`, `0`, and `1`. All four values MUST be finite.

Contrast MUST be positive. Editor trial ranges of exposure ±3 EV, warmth/tint ±1 differential EV, and contrast 0.5–2 MUST NOT become schema clamps.

Nondefault operator correction MUST require the built-in tone on `negative_to_positive` and an eligible negative-film scan route. Declared fixed styles are complete tone definitions and MUST reject explicit nondefault correction, as MUST direct scans, positive film, paper, and scan-minus-base. GUI state MUST retain valid inactive edits and expose their inactive status.

The GUI MUST pass neutral operator values to rendering while those edits are inactive.

Material-producing routes MAY retain otherwise valid pending scanner correction in their stage baseline, but MUST report the operator edits as inactive while no scan executes. Resumed negative `film > scan` MUST use the same reconstruction and correction math as an uninterrupted film scan.

Changing that downstream correction MUST NOT redevelop the stored material or reuse an earlier corrected positive as its correction input.

`scanner.operator` remains exposure-domain negative conversion, distinct from [Tone & Color](../../finishing/spec.md#controls). Its values and units MUST NOT be migrated or equated with finishing controls.

Let `x_c` be reconstructed log10 exposure. No frame-level offset exists.

Define `p = log10(0.16 / 0.184)` and `L = log10(2)`. The operator coordinates and correction are:

```text
e_c = (x_c - p) / L
m = (e_R + e_G + e_B) / 3
v_c = e_c - m
q = (W/2 + M/3, -2M/3, -W/2 + M/3)
e'_c = k*m + v_c + E + q_c
z_c = p + L*e'_c
r_c = T(z_c)
```

`E` is `exposure_ev`: a common exposure increase in stops. Positive values make the positive image brighter. `W` is `warmth_ev`: an increase in the R−B exposure difference in stops.

Positive values make the image warmer. `M` is `tint_ev`: an increase in mean(R,B)−G exposure difference in stops. Positive values make the image more magenta.

`k` is `contrast`: the positive multiplier of common exposure about the reference point. Values above one increase contrast. Warmth and tint are not Kelvin or device button steps.

The common exposure `m` is not CIE Y.

Correction MUST NOT alter density, reconstructed exposure taps, or camera or Convert acquisition settings. Contrast MUST preserve exposure channel differences before tone. The channel-wise nonlinear tone does not guarantee perceptual hue, chroma, or output luminance preservation.

Default correction MUST bypass the coordinate round trip exactly.

## Exact preview and prepared baseline

A negative-to-positive preview MAY reduce the working grid to the preview size, because the readout is frame-independent. Only terminal RGB MAY be reduced afterward.

Diagnostic density and exposure taps MUST retain their actual grid and meaning. Pan, zoom, and display size MUST NOT change the working grid.

The core MUST own one immutable prepared full-frame baseline shared by operator variants, comparison, preview, and export. Operator-only edits MUST reuse reconstruction when the upstream route exposes reusable preparation. Optical Dye/Fotufilm grain MAY require exact full-route recomputation.

A new input, crop, upscale, material, upstream processing, acquisition setting, backend, or precision MUST invalidate incompatible preparation. Cancellation and failed preparation MUST NOT publish a cache entry. Stale results MUST NOT replace a newer input epoch, parameter revision, or request.

Insufficient memory MUST cause explicit failure or exact recomputation, not reduced precision. No separate approximate preview engine is introduced.

## Output primaries

The positive tone output MUST denote fixed linear sRGB. The scan MUST convert this response into the selected output RGB space before gamut processing, CMYK, output mixing, sharpening, and transfer encoding. Selecting sRGB MUST use an exact identity transform.

The independent [tone and color boundary](../../finishing/design.md#color-and-order) applies before output gamut processing on eligible positive scans.

Writer saving-space conversion and viewer ICC conversion remain separate boundaries.

This is a declared synthetic response, not a measured film or scanner calibration. With default correction, pixel compatibility with PR #318 applies only to sRGB. Other output RGB spaces intentionally change because the old path assigned selected primaries to unchanged response numbers.

The runtime model version MUST change; saved carrier versions need not change for additive neutral-defaulted operator fields.

## Tone and style names

Apply the existing logistic family to the corrected coordinate `z_c`. `standard` keeps the historical tone constants in all three channels:

| `brightness_ev` | `contrast` | `dmax` |
| --- | --- | --- |
| `0.8781766618` | `0.73551400058216643` | `1.5625841465572565` |

`standard` is the built-in tone, not an automatic mode. Accept `neutral` as an input-only migration alias. Saved runtime snapshots and effective configuration MUST emit `standard`.

The alias does not preserve the historical fixed-curve pixels.

Any other valid name resolves to `<data-dir>/scan-styles/<name>.json`. A file declares three finite tone triples with positive `contrast` and `dmax`.

Invalid or missing style data MUST fail before output publication. Style names MUST NOT contain `/`, `\`, `..`, or `:`. A catalog MUST NOT offer files named `standard` or `neutral` as additional choices.

`direct_scan` MUST NOT apply conversion setup or tone. It admits the built-in name without effect. Custom style names remain valid only with `negative_to_positive`.

## Point-wise consumers and migration


Earlier film-density or exposure taps remain measurable.

GUI state version 5 owns the cutover. Versions 1–4 MUST remove retired `lamp` keys from scanner sections during migration. A complete legacy capture MUST preserve the old light under `film_render.convert.scan_illuminant` unless it already declares an acquisition light.

Scanner-only preset sections MUST NOT acquire a Convert section. GUI state version 5 and later and unversioned current documents MUST reject retired `lamp` keys. Style aliases normalize on load and save.

This migration intentionally changes scan appearance.

Preset schema 6 and recipe schema `spektrafilm-rs-params-6` own the same scanner cutover for versions 1–5. It MUST validate historical scanner provenance against the built-in model before replacing it. It MUST reject forged device metadata.

Current schemas reject `lamp` and retain all unrelated retired-field rejection. Sparse sections MUST preserve omission when normalizing the style alias.

The additive defaulted `scanner.operator` fields require no additional schema migration. Omitted operator fields MUST receive neutral defaults. Unknown and retired fields MUST remain rejected.

Operator correction MUST NOT revive `conversion_*`, `auto_setup`, `experimental_setup`, or `finish` controls.

Presets, recipes, parameter discovery, CLI `--set`, GUI persistence, and effective render configuration MUST share the typed core model. Automatic offsets MUST NOT enter ordinary saved correction parameters.

## Acceptance and limits

- A uniform neutral patch MUST keep its value when the rest of the frame changes.
- Gray ramps remain neutral, monotone, and finite. Positive and negative exposure shifts remain distinguishable.
- Exact residual `(1 - slope)` applies to linear response curves, not arbitrary toe/shoulder regions.
- Repeated execution on the same frame produces identical pixels at the same precision and backend.
- Every style retains record independence before destination primary conversion.
- A dominant red field remains red after correction. A dark field MUST NOT be forced to the same result as the neutral aim.
- A bright background MUST NOT change a subject's result. The readout provides no face detection, subject classification, or local tone correction.
- Do not weaken this limit into a claim that inverse-lighting scenes are rescued.
- Backend parity, style changes, canonical alias output, versioned migration, and point-wise rejection require real-consumer tests.
- No paired real minilab scans are available. Passing these tests does not establish a Frontier/Noritsu match or prove this algorithm best.
