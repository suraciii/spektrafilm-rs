# Film chain

## Contract

- Existing image-entry `input` and film workflows accept a normalized image, a film profile, and runtime controls. Input processing follows the [CMYK ink style entry-point contract](../cmyk-style/spec.md#enablement). Completed-material entry follows [Film and print continuation](#film-and-print-continuation-target) instead; it MUST NOT be interpreted as a new scene image.
- A route that exposes and develops photographic print material must provide a photographic print profile. A film-only continuation MUST NOT acquire a paper dependency.
- Optional CMYK ink style must satisfy the [CMYK ink style](../cmyk-style/spec.md) contract. It must not determine base workflow topology.
- Base workflows include input color management, direct film scanning, photographic printing, and the existing film-conversion routes.
- The default film path is: source color and exposure → spectral film exposure and development → film-domain effects (including Spektra, Dye, and Fotufilm Film grain) → the selected base route → scanner interpretation → output gamut processing → unsharp masking → transfer encoding → Procedural grain → optional CMYK ink style → output mix → output handoff.
- [Output mix and material controls](../output-mix/spec.md) owns reuse of existing preflash and chemistry, retirement of Output lightness compression, and overall output mix. It adds no digital material-response layer or scan-reference isolation pass. CMYK ink style remains after all scanner-domain effects; overall output mix follows it before output handoff.
- Film, photographic paper, spectral, and color profiles are data inputs. A missing or invalid data input consumed by the selected route is an error before output publication. A continuation MUST NOT require unused material dependencies or prepare already completed operations merely to validate them.
- A profile's `hanatos2025_adaptation_window_params` and `hanatos2025_adaptation_surface_params` are **conditional fields** of a filming-stage material. A `stage = printing` material — photographic paper and motion-picture print film — does not convert an encoded input to a film exposure, so it carries neither field and must not be selected as a film.
- A filming-stage material whose `hanatos2025_adaptation_window_params` is absent runs the RGB-to-exposure conversion **without the sensitivity-adaptation window**. The conversion keeps white balance but models no ultraviolet or infrared cutoff, so that material's spectral response is idealized. The run must report the downgrade with the material identity instead of downgrading silently.
- A filming-stage material that carries `hanatos2025_adaptation_surface_params` runs the log-exposure correction surface when the look enables it. An absent surface field runs no surface correction.
- The selected route preserves the requested color, spatial, optical, and stochastic semantics. A backend must not silently disable or replace them to remain on a fast path.
- Execution policy and CPU/GPU algorithm alignment follow [GPU execution](../gpu-execution/spec.md). GPU execution must cover the selected chain without replacing its model.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.

## Film and print continuation (target)

**Implementation gap:** the routes and material-state entry described in this section
are normative target behavior, not implemented CLI capabilities. The
[stage-artifact contract](../cli/stage-artifacts.md) is authoritative for the `.sfstage`
format, validation, frozen controls, seed inheritance, and publication. This section owns
the film-chain meaning of continuing from a developed medium. Existing image-entry and
real-negative `convert-film` behavior remains unchanged.

The added routes are `input > film`, `input > film > print`, `film > scan`, `film > print`,
`film > print > scan`, and `print > scan`. A route ending in `film` or `print` produces
one completed, always-resumable material state in a `.sfstage` directory containing
`stage.json`; a route ending in `scan` produces the ordinary image output. A material
input MUST declare its route explicitly. It is not an optional observation bundle, and
it MUST NOT be accepted as an ordinary image or a captured-negative `convert-film` input.

- A film state is developed film record density with its required geometry, material
  identity, frozen source controls, resolved camera exposure compensation, and seed.
  Source interpretation, metering, crop/resize, camera conversion, exposure, development,
  halation, DIR, and film-domain grain have already run. `film > scan` runs only scan and
  downstream output processing. `film > print` runs only print exposure and development;
  `film > print > scan` then scans that developed print.
- A print state is developed print record density with the context required to scan it.
  `print > scan` MUST NOT repeat film preparation, enlarger exposure, print development,
  or effects already represented by that state. A print state is not a scan image and
  contains no already applied scanner/output effects to replay.
- A continuation MUST preserve the stored geometry and physical pitch; it MUST NOT
  re-meter, crop, resize, re-expose, or redevelop a completed source. Frozen-control
  conflicts fail before output publication. Downstream controls remain editable. An
  omitted seed inherits the source seed under the stage-artifact contract.
- Dependency preparation follows the requested remaining operations, not the original
  image route. Resolve material data needed to interpret the stored density, construct
  a scan reference or inverse, or expose/develop a new print. A film scan needs no paper
  profile; a print scan needs no film preparation or camera-side spectral resources.
  Required source context MUST be carried by the artifact rather than recovered by
  loading or reprocessing the original image.
- On a negative `film > scan` with `scanner.scan_output = negative_to_positive`, the
  conversion reads the stored film record densities directly and retains the simulated
  negative's frozen camera exposure compensation. It subtracts that compensation just
  as `input > film > scan` does. It MUST NOT estimate a new camera exposure from density,
  zero the compensation merely because the input is a material state, or synthesize a
  captured-negative recovery step. Real-negative `input > convert-film > scan` retains
  its existing exposure semantics.
- Numerical research observations are owned by dedicated instrument workflows and the
  core observer under [Native observations](../instrument/native-observations.md).
  They are not `.sfstage` states and MUST NOT be attached to ordinary processing through
  an optional capture flag. Boolean `--debug` is stderr-only and `--diagnostics REPORT`
  is execution reporting under [Debug and diagnostics](../cli/spec.md#debug-diagnostics);
  neither changes chain numerics or selects observation/material files.

### Continuation acceptance

- Splitting an admitted image chain at film or print and resuming it with identical
  downstream controls, inherited seed, computation precision, and backend MUST preserve
  the completed densities and final scan result. Stage loading MUST NOT add another
  geometry transform, metering pass, optical effect, or stochastic effect.
- Film-only scanning MUST work without a paper selection or unused paper data. Print
  scanning MUST work without invoking film preparation or requiring unused film data.
  Missing dependencies actually used by the continuation MUST still fail.
- A conflicting frozen override MUST fail; editing only an admitted downstream control
  MUST leave the source state unchanged. Omitted seed MUST inherit rather than reset.
- Negative `film > scan` conversion MUST match the corresponding unsplit simulated
  negative chain with nonzero camera exposure compensation; real-negative conversion
  MUST retain its existing behavior.


## Capture light and scan light

Each light source in the chain has one owner, and the chain holds them separate. The **capture illuminant** is the assumed scene light of the photograph; its semantics, inheritance, validation, and report contract are owned by the [Image input capture illuminant contract](../image-input/spec.md#capture-illuminant), because the camera-side conversion into film exposure belongs to image input.

The capture illuminant MUST NOT change any of the following, and none of the following MAY change it:

- film exposure, development, film-domain effects, or the developed negative;
- `scanner.lamp`, the single light source of a scan, which belongs to scan acquisition;
- `enlarger.illuminant` and the enlarger filter pack, which belong to print exposure;
- the output white point and transfer encoding.

Stage preparation resolves the capture illuminant once for the selected profile and controls, before the camera-side spectral conversion. A route that computes no film exposure from scene RGB — the `input` route, a real negative-capture `input > convert-film > ...` route, or a target continuation from completed film or print — does not consume it for a new camera conversion. A continuation retains the frozen source context rather than resolving a new capture declaration. A zero-balance negative conversion renders a grey lit by the profile reference illuminant neutral, and that rule continues to describe the conversion, not the capture declaration.

The scanner lamp is resolved through the same shared illuminant service. Its accepted values are the illuminant vocabulary of `enlarger.illuminant`. An invalid lamp name fails before output publication.

No Capture & Scan Setup document, universal camera or scanner registry, or automatic scene-light inference is part of this contract.

## Scanner language

Scanner terms follow the [Film simulation language](../../CONTEXT.md). A reversal-film scan and a photographic-paper scan already read positive images.

The scan is one colorimetric capture made through the scanner lamp. `scanner.lamp` names that single physical light source; its accepted values are `D50`, `D55`, `D65`, `T`, `TH-KG3`, `TH-KG3-L`, `K75P`, and a blackbody temperature. `D55` is the default. The lamp belongs to scan acquisition and MUST NOT be derived from the capture illuminant, the enlarger illuminant, or image content.

The scan output value `negative_to_positive` selects negative conversion. Negative conversion has one interpretation: it reads each dye record's density through the selected film's response and applies a positive tone response from one curve family, whose constants a declared conversion style selects. It requires no automatic setup and no scanner finish.

Image correction and digital output must not select a different scanned medium or a photographic print route.

`scanner.scan_output` accepts only `direct_scan` and `negative_to_positive`. The removed value `positive_scan` must fail validation in CLI options, runtime parameters, sparse sources, presets, render recipes, and imported GUI state. Readers must not alias it or silently reset it. Omitted scan output retains the `direct_scan` default. Existing schema versions and parameter field names remain unchanged.

The white reference of a scan is the clear film base of the scanned medium. A clear area of that base reads as the output color space's white; the chromatically adapted scan of any other area is expressed against that white.

## Scan output naming acceptance

1. Discovery, CLI help, effective parameter reports, saved presets, recipes, and GUI state must emit the canonical scan output value.
2. GUI controls must display `Preserve medium polarity` for `direct_scan` and `Negative to positive` for `negative_to_positive`.
3. Replacing the removed value with `negative_to_positive` must preserve pixels, defaults, route eligibility, correction precedence, and execution backend for identical inputs and controls. Verify density, direct-negative, reversal-film, and paper paths with the same executable precision.
4. Removed-value rejection must occur before output publication. A failed GUI import must retain the accepted parameters and image.
5. Conversion telemetry must identify stage `negative_to_positive` and its actual executor under [Telemetry](../telemetry/metrics/spec.md#effective-configuration). Selecting this mode must not require a CPU reason or a resident decline.

## Scan medium and scanner controls

- The selected route determines the scanned medium. A film-scan route scans the film; a print route scans the photographic paper.
- The scanned medium determines the scanner controls that apply.
- A photographic-paper scan and a slide film scan read a positive image. A negative film scan reads the negative image on the film; `negative_to_positive` produces the positive image by negative conversion.
- Every scan exposes the scanner lamp, scanner lens blur, scanner unsharp mask, and the scan output mode.
- A negative film scan with `negative_to_positive` additionally exposes `scanner.conversion_style`, the declared stylization point of the conversion tone.
- The scanner owns no white/black correction, no white/black levels, and no film-scan black or white points. Those controls are removed from the runtime model, discovery, presets, recipes, GUI state, and CLI options; their canonical names must fail unknown-field validation at the current schema version.
- Negative conversion belongs to `negative_to_positive`. A photographic-paper scan and a slide film scan must not expose it. See [Direct negative-film scan output](#direct-negative-film-scan-output).
- Existing exposure and development controls belong to their material stage. No appearance treatment is applied to a completed scan: scanner finishing is removed from the contract.
- Scanner calibration, inverse curves, and correction references retain their existing dependencies on profile and development settings. Material controls must not add a second reference model or alter the real captured-negative recovery solve.
- Scan provenance records one colorimetric sensor and the clear-film white reference. It must publish the resolved lamp, the output space white, and the identity matrix of the scan transform.

## Scan optics and print viewing glare

Scanner lens blur must model readout optics on linear transmitted or reflected light. It must run before nonlinear density or exposure interpretation, output gamut processing, unsharp mask, and transfer encoding. The operation must not blur developed density or encoded RGB as a substitute for optical mixing.

Spectral integration may precede blur when the sensor is linear and spatially uniform and the same achromatic spatial kernel applies to every channel. Otherwise blur must occur in the corresponding light domain before integration. Changing destination primaries must not change the optical kernel or where it runs. Scanner blur must not change the developed medium or the input recovery solve of a convert-film route.

`print_render.glare` represents print viewing/readout stray light. It must be added in the linear readout-light domain after scanner lens blur and before nonlinear interpretation. It must not change print exposure, chemistry, or developed print density. It remains eligible only on the existing print-scan routes, with its existing controls and seed behavior. Film-scan routes must not acquire this effect through `film_render.glare` or another alias.

Unsharp mask is digital scan sharpening, not lens resolution. It remains downstream of interpretation and output gamut processing. Procedural grain retains its separate [post-scan boundary](../film-grain/v2/spec.md); it must not be moved into optical acquisition to make the grain engines appear equivalent.

## Film halation source response

`film_render.halation.source_compression` is an experimental appearance control, not a measured CineStill threshold or a replacement for film halation physics. It defaults to `0`, which preserves the existing linear halation source response. A positive value applies a single per-pixel response to the post-scatter film raw only for the halation blur input; it does not modify the base film raw, film exposure, density, grain, bloom, or scanner readout.

The response uses the maximum nonnegative channel value at each pixel and the fixed linear film-raw anchor `1.0`. For a peak at or below the anchor, the response is `source / ((1 - compression) + compression * peak)`; for a peak above the anchor, the response is unchanged. CPU and resident GPU paths must prepare this source once and reuse it for every halation bounce. The value is bounded to `0..=0.999999`. It is intended for experimental night-scene looks where secondary bright sources should contribute more nearly like the brightest source; it must not be described as stock calibration or a physical trigger threshold.

This control remains separate from `halation_strength`, `halation_amount`, `boost_ev`, camera diffusion, and lens/atmospheric bloom. The general GPU execution contract still governs convolution support and CPU/GPU alignment.

## Optical-order cutover

The historical direct-scan sequence that blurs after output gamut processing must not remain as a compatibility mode. All admitted scan routes must obey [Scan optics and print viewing glare](#scan-optics-and-print-viewing-glare). Parameter names, units, and zero-blur meaning remain unchanged. Nonzero blur on a previously late-blur path may intentionally change pixels even with unchanged material settings and full output amount.

The implementation must identify the new ordering in its render/model version. Recipe replay must report the model version used and must not claim historical pixel identity. It must not silently rescale blur to emulate the old order.

The [Print exposure composition](../output-mix/design.md#print-exposure-composition) fixes the print optical boundary and owns the preflash-separation equivalence conditions.

## Neutral scan model

- The scanner has one colorimetric sensor modeled on the CIE 1931 color-matching functions. No narrow-band, approximate, or measured-device receiver profile exists; the sensor is not selectable.
- Scan acquisition is one spectral integration of the scanned medium under the scanner lamp. `scanner.lamp` resolves to one spectral power distribution through the shared illuminant service that `enlarger.illuminant` uses, so one service owns every light-source name.
- The white reference of a scan is the clear film base of the scanned medium. The scan chromatically adapts from that clear-film white to the output color space's white and emits output-space linear RGB. The scan matrix is the identity; no receiver calibration matrix is applied.
- The clear film base must read as the output color space's white on every admitted route, for every accepted lamp, and for both scan output values.
- The lamp changes the capture. Changing `scanner.lamp` must change the scanned pixels of a colored area, while a clear-film area remains the output white.
- The lamp must not change film exposure, development, film-domain effects, print exposure, or the input camera exposure. It must not be derived from the capture illuminant or the enlarger illuminant, and it must not be inferred from image content.
- `negative_to_positive` has one interpretation. It reads each dye record's density, as a narrowband scanner does, derives a per-record monotone inverse from the selected profile density curves, and applies a positive tone response from the same logistic curve family. One record's output must not depend on another record's density.
- No selectable negative conversion method exists. The removed names `separable`, `coupled`, and `digital_reference`, and the leaf `scanner.negative_conversion_method`, must fail unknown-field validation in runtime parameters, sparse sources, inline assignments, current-schema presets, and imported recipes.
- The scanner owns no automatic setup and no finish. The removed leaves `scanner.auto_setup`, `scanner.finish`, `scanner.finish_strength`, `scanner.white_correction`, `scanner.black_correction`, `scanner.white_level`, `scanner.black_level`, `scanner.black_point`, `scanner.white_point`, `scanner.reference_exposure_ev`, and `scanner.reference_balance_ev` must fail unknown-field validation at the current schema version.
- Conversion tone controls — brightness, RGB balance, contrast, and maximum density — are removed as selectable controls. The `neutral` style carries the frozen reference tone constants of that curve family; `scanner.conversion_style` selects a declared style that parameterizes the same curve family's constants per channel. No new curve family is introduced, and no style is fitted from image or chart data.
- Scan provenance must serialize the neutral scan model identity: one colorimetric sensor, the clear-film white reference, the output space white, and the identity scan matrix.

## Direct negative-film scan output

- `scanner.scan_output` is `direct_scan` by default. It preserves the selected medium's native scan polarity.
- `negative_to_positive` is valid for a negative film profile on either the direct `input > film > scan` route or the real negative-capture `input > convert-film > scan` route; the target continuation additionally admits `film > scan` under [Film and print continuation](#film-and-print-continuation-target). Paper profiles, print-paper routes, and `scan-minus-base` are not eligible.
- Negative conversion input is record density. It reads each dye record's density, as a narrowband scanner does, and derives a per-record monotone inverse from the selected profile density curves. One record's output must not depend on another record's density. The simulated `input > film > scan` route hands it the film stage's record densities and inserts no synthetic capture; the target `film > scan` continuation reads those same stored densities.
- Negative conversion estimates relative log exposure, subtracts camera exposure compensation on a simulated negative chain (`input > film > scan`, or its target `film > scan` continuation using the frozen source compensation), applies the positive tone response selected by `scanner.conversion_style`, then emits output-space linear RGB. It does not subtract simulated camera exposure compensation on the real-negative `input > convert-film > scan` route. Zero balance renders a grey lit by the profile reference illuminant neutral.
- `scanner.conversion_style` is the declared stylization point of negative conversion. It accepts a style name; `neutral` is the default and is byte-identical to the frozen reference conversion. Any other name resolves to `data/scan-styles/<name>.json`.
- A style file declares one tone triple per channel — `brightness_ev`, `contrast`, and `dmax` — of the same logistic curve family the `neutral` style uses. The per-channel triples are what allow a color tendency; they do not add a curve family and do not change the record-density reading or its monotone inverse.
- A style is declared data. It is never fitted from image or chart data, and it must not be described as a measured or calibrated scanner response. The film's own tone stays visible under every style.
- No conversion style ships with the product. `neutral` is built in; every other style is a user-authored file in the data directory's `scan-styles/` folder. The product must not promise a built-in style catalog.
- A style must keep per-record independence and density monotonicity. One record's output must not depend on another record's density under a non-neutral style.
- `scanner.conversion_style` is resolved once at pipeline construction. A name that is not `neutral` requires a pipeline built from a data directory; an unknown style must fail with an error that lists the available styles. A parse error must name the offending field.
- A style file must declare three finite channel triples with `contrast > 0` and `dmax > 0`. A missing channel, a non-finite value, or a non-positive contrast or dmax must fail before output publication.
- A style name must not be empty and must not contain `/`, `\`, `..`, or `:`. An invalid name must fail before output publication.
- A non-neutral style requires `scanner.scan_output = negative_to_positive`. A `direct_scan` must not read the style slot, so the style is inert on a direct scan.
- `scanner.conversion_style` must be reported in the effective configuration under [Telemetry](../telemetry/metrics/spec.md#effective-configuration).
- Densities outside the profile curves use finite endpoint tangent extrapolation; the tone curve keeps output finite and within the configured density range. Negative and above-clear scanner RGB remains representable until the normal writer; no log-of-clipped-RGB operation is permitted.
- Scanner lens blur acts on transmitted light before the density reading. Output gamut compression, unsharp mask, transfer encoding, Procedural grain, CMYK ink style, and output handoff run after negative conversion.
- `negative_to_positive` rejects active Procedural grain. Its execution follows [GPU execution](../gpu-execution/spec.md#backend-selection); conversion selection must not force CPU.

## Print exposure mode

- `enlarger.print_exposure_mode` selects `manual` or `auto`. `auto` is the default.
- `auto` normalizes the print exposure so the developed midgray density prints to the paper midgray, and follows camera exposure compensation.
- `manual` applies `enlarger.print_exposure` without midgray normalization.
- `settings.print_exposure_ev_tracking` selects `follow_mode` (default), `never`, or `always`. `never` removes camera exposure compensation from `auto`. `always` applies the relative compensation factor in `manual`. It must not change any other stage.
- The legacy `enlarger.normalize_print_exposure` and `enlarger.print_exposure_compensation` leaves must not appear in runtime models, editable paths, discovery, GUI, or new documents. Only a versioned migration maps them: both true to `auto`, both false to `manual`, normalization without compensation to `auto` with `never`, and compensation without normalization to `manual` with `always`. The mapping must preserve rendered pixels.

## Conversion control naming acceptance

1. Discovery, CLI help, effective parameter reports, saved presets, recipes, and GUI state must emit the canonical paths `scanner.lamp`, `scanner.scan_output`, `scanner.lens_blur`, `scanner.unsharp_mask`, and `enlarger.print_exposure_mode`.
2. GUI display names must use the real operation words: scanner lamp, scan output, lens blur, unsharp mask, and print exposure mode.
3. Replacing a legacy path with its canonical path must preserve pixels, defaults, route eligibility, correction precedence, and execution backend for identical inputs and controls.
4. The legacy `scanner.receiver_profile`, `scanner.negative_conversion_method`, `scanner.conversion_brightness_ev`, `scanner.conversion_balance_ev`, `scanner.conversion_contrast`, `scanner.conversion_dmax`, `scanner.auto_setup`, `scanner.finish`, `scanner.finish_strength`, `scanner.white_correction`, `scanner.black_correction`, `scanner.white_level`, `scanner.black_level`, `scanner.black_point`, `scanner.white_point`, `scanner.reference_exposure_ev`, and `scanner.reference_balance_ev` paths, the legacy `enlarger.normalize_print_exposure` and `enlarger.print_exposure_compensation` leaves, and the legacy `film_render.convert.scan_illuminant` leaf must fail unknown-field validation in runtime parameters, sparse sources, inline assignments, current-schema presets, and imported recipes. Only the versioned preset migration and the versioned complete GUI state migration may map them.
5. Rejection timing and failed-import retention follow [Scan output naming acceptance](#scan-output-naming-acceptance).
6. The implementation must identify the rename in its render/model version. Readers must not alias legacy paths at the current schema version.

## Scanner acceptance

- The clear film base of the scanned medium must read as the output color space's white on every admitted route, for every accepted lamp, and for both scan output values.
- Changing `scanner.lamp` must change the scanned pixels of a colored area. A clear-film area must remain the output white under every lamp.
- The lamp must not change film density, film-domain grain, halation, print exposure, or camera exposure. It must change only the scan acquisition.
- `direct_scan` must preserve the native polarity of the selected medium.
- Negative conversion must read one record's density without depending on another record's density, and must not acquire a receiver capture.
- Negative conversion must produce finite, continuous output across the calibration range and finite tangent extrapolation outside it.
- `conversion_style = neutral` must render byte-identically to the frozen reference conversion and must not require a data directory.
- A non-neutral style must change the converted pixels only through the per-channel tone constants of the same logistic curve family, and must keep per-record independence and density monotonicity.
- A `direct_scan` must produce identical pixels for `conversion_style = neutral` and any other style, because the style slot is inert on a direct scan.
- An unknown non-neutral style, an invalid style name, an unreadable style file, and a style file with a non-finite or non-positive value must fail before output publication and must list the available styles where the renderer can enumerate them.
- Changing film exposure, development, grain, halation, print exposure, or camera exposure while holding scanner settings constant must not be an effect of lamp selection.
- Scan provenance must identify the neutral scan model, the resolved lamp, the clear-film white reference, the output space white, and the identity scan matrix. An approximate or measured-device receiver label must not appear in GUI, CLI diagnostics, or serialized provenance.
- Scanner parameter combinations that the selected route or medium cannot support must fail before output publication and must produce the same validation result for GUI, CLI dry run, and rendering.
- A nonuniform bright-edge capture and an active nonlinear downstream transform must distinguish blur-before-interpretation from blur-after-interpretation. Cover native negative, slide, print scan, and the negative conversion path. Zero blur must preserve the corresponding no-blur result; uniform fields must remain uniform with blur active.
- Changing only print viewing glare must leave film and print density unchanged and alter the linear readout before downstream interpretation. With glare disabled, spatially varying glare must not leak into calibration or other routes.
- CPU f64, native-f32, resident, per-stage, Preview, and Export must use the same optical order. A GPU request must implement that order or fail under [GPU execution](../gpu-execution/spec.md#backend-selection), not append blur to an already interpreted image. Cropped and tiled readout must include the required optical context and match full-frame readout within measured budgets.
