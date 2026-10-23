# Film chain

## Contract

- The `input` workflow and film workflows accept a normalized image, a film profile, and runtime controls. Input processing follows the [CMYK ink style entry-point contract](../cmyk-style/spec.md#enablement).
- A photographic print route must provide a photographic print profile.
- Optional CMYK ink style must satisfy the [CMYK ink style](../cmyk-style/spec.md) contract. It must not determine base workflow topology.
- Base workflows include input color management, direct film scanning, photographic printing, and the existing film-conversion routes.
- The default film path is: source color and exposure → spectral film exposure and development → film-domain effects (including Spektra, Dye, and Fotufilm Film grain) → the selected base route → scanner interpretation → output gamut processing → unsharp masking → transfer encoding → Procedural grain → optional CMYK ink style → output mix → output handoff.
- [Output mix and material controls](../output-mix/spec.md) owns reuse of existing preflash and chemistry, retirement of Output lightness compression, and overall output mix. It adds no digital material-response layer or scan-reference isolation pass. CMYK ink style remains after all scanner-domain effects; overall output mix follows it before output handoff.
- Film, photographic paper, spectral, and color profiles are data inputs. A missing or invalid selected data input is an error before output publication.
- A profile's `hanatos2025_adaptation_window_params` and `hanatos2025_adaptation_surface_params` are **conditional fields** of a filming-stage material. A `stage = printing` material — photographic paper and motion-picture print film — does not convert an encoded input to a film exposure, so it carries neither field and must not be selected as a film.
- A filming-stage material whose `hanatos2025_adaptation_window_params` is absent runs the RGB-to-exposure conversion **without the sensitivity-adaptation window**. The conversion keeps white balance but models no ultraviolet or infrared cutoff, so that material's spectral response is idealized. The run must report the downgrade with the material identity instead of downgrading silently.
- A filming-stage material that carries `hanatos2025_adaptation_surface_params` runs the log-exposure correction surface when the look enables it. An absent surface field runs no surface correction.
- The selected route preserves the requested color, spatial, optical, and stochastic semantics. A backend must not silently disable or replace them to remain on a fast path.
- Execution policy and CPU/GPU algorithm alignment follow [GPU execution](../gpu-execution/spec.md). GPU execution must cover the selected chain without replacing its model.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.

## Capture light and scan light

Each light source in the chain has one owner, and the chain holds them separate. The **capture illuminant** is the assumed scene light of the photograph; its semantics, inheritance, validation, and report contract are owned by the [Image input capture illuminant contract](../image-input/spec.md#capture-illuminant), because the camera-side conversion into film exposure belongs to image input.

The capture illuminant MUST NOT change any of the following, and none of the following MAY change it:

- film exposure, development, film-domain effects, or the developed negative;
- the readout illuminant, a product constant selected by the material category;
- `enlarger.illuminant` and the enlarger filter pack, which belong to print exposure;
- the output white point and transfer encoding.

Stage preparation resolves the capture illuminant once for the selected profile and controls, before the camera-side spectral conversion. A route that computes no film exposure from scene RGB — the `input` route and a real negative-capture `input > convert-film > ...` route — does not consume it. A zero-balance negative conversion renders a grey lit by the profile reference illuminant neutral, and that rule continues to describe the conversion, not the capture declaration.

The readout illuminant is a product constant: `BB5800` for colour cine print materials (ISO 2910:2018) and `D50` for every other material (ISO 3664). It is not a runtime parameter, a profile field, or a preset value.

No Capture & Scan Setup document, universal camera or scanner registry, or automatic scene-light inference is part of this contract.

## Scanner language

Scanner terms follow the [Film simulation language](../../CONTEXT.md). A reversal-film scan and a photographic-paper scan already read positive images.

The scan is one colorimetric capture made through the readout illuminant. The readout illuminant is a product constant selected by the material category; it is not a control. It MUST NOT be derived from the capture illuminant, the enlarger illuminant, or image content. A document that carries `scanner.lamp` fails validation at the current schema version.

The scan output value `negative_to_positive` selects negative conversion. Negative conversion has one interpretation: it reads each dye record's density through the selected film's response and applies a positive tone response from one curve family, whose constants a declared conversion style selects. The built-in `standard` style uses automatic frame setup. It has no setup control and no scanner finish.

Image correction and digital output must not select a different scanned medium or a photographic print route.

`scanner.scan_output` accepts only `direct_scan` and `negative_to_positive`. The removed value `positive_scan` must fail validation in CLI options, runtime parameters, sparse sources, presets, render recipes, and imported GUI state. Readers must not alias it or silently reset it. Omitted scan output retains the `direct_scan` default. The [lab scan cutover](lab-scan/spec.md#point-wise-consumers-and-migration) owns document-version migration.

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
- Every scan exposes scanner lens blur, scanner unsharp mask, and the scan output mode.
- A negative film scan with `negative_to_positive` additionally exposes `scanner.conversion_style`, the declared stylization point of the conversion tone.
- The scanner owns no white/black correction, no white/black levels, and no film-scan black or white points. Those controls are removed from the runtime model, discovery, presets, recipes, GUI state, and CLI options; their canonical names must fail unknown-field validation at the current schema version.
- Negative conversion belongs to `negative_to_positive`. A photographic-paper scan and a slide film scan must not expose it. See [Direct negative-film scan output](#direct-negative-film-scan-output).
- Existing exposure and development controls belong to their material stage. No appearance treatment is applied to a completed scan: scanner finishing is removed from the contract.
- Scanner calibration, inverse curves, and correction references retain their existing dependencies on profile and development settings. Material controls must not add a second reference model or alter the real captured-negative recovery solve.
- Scan provenance records one colorimetric sensor and the clear-film white reference. It must publish the resolved readout illuminant, the output space white, and the identity matrix of the scan transform.

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
- Scan acquisition is one spectral integration of the scanned medium under the readout illuminant. The illuminant is resolved from the material category through the shared illuminant service, so one service owns every light-source name.
- The white reference of a scan is the clear film base of the scanned medium. The scan chromatically adapts from that clear-film white to the output color space's white and emits output-space linear RGB. The scan matrix is the identity; no receiver calibration matrix is applied.
- The clear film base must read as the output color space's white on every admitted route, for every material category, and for both scan output values.
- The readout illuminant belongs to positive transmission and paper capture. Negative-to-positive readout instead uses the fixed synthetic tone response and output conversion defined by the [lab scan contract](lab-scan/spec.md#output-primaries). Neither path may change reconstructed record exposure through readout-light selection.
- The readout illuminant must not change film exposure, development, film-domain effects, print exposure, or the input camera exposure. It must not be derived from the capture illuminant or the enlarger illuminant, and it must not be inferred from image content.
- `negative_to_positive` has one interpretation. It reads each dye record's density, as a narrowband scanner does, derives a per-record monotone inverse from the selected profile density curves, and applies a positive tone response from the same logistic curve family. One record's reconstructed exposure must not depend on another record's density. Standard setup is a separate frame-level stage.
- No selectable negative conversion method exists. The removed names `separable`, `coupled`, and `digital_reference`, and the leaf `scanner.negative_conversion_method`, must fail unknown-field validation in runtime parameters, sparse sources, inline assignments, current-schema presets, and imported recipes.
- The scanner exposes no automatic setup control and no finish. The removed leaves `scanner.auto_setup`, `scanner.finish`, `scanner.finish_strength`, `scanner.white_correction`, `scanner.black_correction`, `scanner.white_level`, `scanner.black_level`, `scanner.black_point`, `scanner.white_point`, `scanner.reference_exposure_ev`, and `scanner.reference_balance_ev` must fail unknown-field validation at the current schema version.
- The positive tone constants are not exposed as scalar knobs. The [lab scan contract](lab-scan/spec.md) owns operator correction, style names, automatic setup, constants, and point-wise consumer behavior.
- Scan provenance must identify the readout model, the colorimetric sensor, the clear-film white reference, the output space white, and the identity scan matrix.

## Direct negative-film scan output

- `scanner.scan_output` defaults to `direct_scan`. It preserves the selected medium's native polarity.
- `negative_to_positive` accepts a negative film on `input > film > scan` or `input > convert-film > scan`. Paper, positive film, print-paper, and scan-minus-base routes remain ineligible.
- The negative reconstruction reads developed record density over each record's own minimum. It inserts no synthetic scanner capture.
- The [lab scan contract](lab-scan/spec.md) owns the inverse, frame setup, tone, `standard` default, `neutral` migration alias, and fixed file styles.
- Scanner lens blur acts on transmitted light before density reconstruction. Output gamut compression, unsharp mask, encoding, CMYK ink style, and output handoff run after conversion.
- `negative_to_positive` rejects active Procedural grain. Backend selection follows [GPU execution](../gpu-execution/spec.md#backend-selection).

## Print exposure mode

- `enlarger.print_exposure_mode` selects `manual` or `auto`. `auto` is the default.
- `auto` normalizes the print exposure so the developed midgray density prints to the paper midgray, and follows camera exposure compensation.
- `manual` applies `enlarger.print_exposure` without midgray normalization.
- `settings.print_exposure_ev_tracking` selects `follow_mode` (default), `never`, or `always`. `never` removes camera exposure compensation from `auto`. `always` applies the relative compensation factor in `manual`. It must not change any other stage.
- The legacy `enlarger.normalize_print_exposure` and `enlarger.print_exposure_compensation` leaves must not appear in runtime models, editable paths, discovery, GUI, or new documents. Only a versioned migration maps them: both true to `auto`, both false to `manual`, normalization without compensation to `auto` with `never`, and compensation without normalization to `manual` with `always`. The mapping must preserve rendered pixels.

## Conversion control naming acceptance

1. Discovery, CLI help, effective parameter reports, saved presets, recipes, and GUI state must emit the canonical paths `scanner.scan_output`, `scanner.lens_blur`, `scanner.unsharp_mask`, and `enlarger.print_exposure_mode`.
2. GUI display names must use the real operation words: scan output, lens blur, unsharp mask, and print exposure mode.
3. Replacing a legacy path with its canonical path must preserve pixels, defaults, route eligibility, correction precedence, and execution backend for identical inputs and controls.
4. The legacy `scanner.receiver_profile`, `scanner.negative_conversion_method`, `scanner.conversion_brightness_ev`, `scanner.conversion_balance_ev`, `scanner.conversion_contrast`, `scanner.conversion_dmax`, `scanner.auto_setup`, `scanner.finish`, `scanner.finish_strength`, `scanner.white_correction`, `scanner.black_correction`, `scanner.white_level`, `scanner.black_level`, `scanner.black_point`, `scanner.white_point`, `scanner.reference_exposure_ev`, and `scanner.reference_balance_ev` paths, the legacy `enlarger.normalize_print_exposure` and `enlarger.print_exposure_compensation` leaves must fail unknown-field validation in runtime parameters, sparse sources, inline assignments, current-schema presets, and imported recipes. Only the versioned preset migration and the versioned complete GUI state migration may map them.
5. Rejection timing and failed-import retention follow [Scan output naming acceptance](#scan-output-naming-acceptance).
6. The implementation must identify the rename in its render/model version. Readers must not alias legacy paths at the current schema version.

## Scanner acceptance

- The clear film base of the scanned medium must read as the output color space's white on every admitted route, for every material category, and for both scan output values.
- The readout illuminant must follow the material category; no runtime control may override it. A clear-film area must remain the output white on every admitted route.
- The readout illuminant must not change film density, film-domain grain, halation, print exposure, or camera exposure. It may change only the readout colour transform.
- `direct_scan` must preserve the native polarity of the selected medium.
- Negative reconstruction must read each record independently and acquire no receiver capture. Frame setup may depend on all records and pixels.
- Negative conversion must produce finite, continuous output across the calibration range and finite tangent extrapolation outside it.
- `standard` must run the frame setup and fixed tone in the [lab scan contract](lab-scan/spec.md). `neutral` must act as an input-only alias, not a historical fixed-render mode.
- A fixed file style must change the source tone only through the per-channel constants of the same logistic curve family. Its inverse and source tone remain record-independent and monotone. Destination primary conversion may mix those source channels.
- A `direct_scan` must produce identical pixels for the built-in style and any fixed style, because the style slot is inert on a direct scan.
- An unknown fixed style, an invalid style name, an unreadable style file, and a style file with a non-finite or non-positive value must fail before output publication and must list the available styles where the renderer can enumerate them.
- The product readout illuminant must not change film exposure, development, grain, halation, print exposure, or camera exposure.
- Scan provenance must identify the readout model, the material-category readout illuminant, the clear-film white reference, the output space white, and the identity scan matrix. An approximate or measured-device receiver label must not appear in GUI, CLI diagnostics, or serialized provenance.
- Scanner parameter combinations that the selected route or medium cannot support must fail before output publication and must produce the same validation result for GUI, CLI dry run, and rendering.
- A nonuniform bright-edge capture and an active nonlinear downstream transform must distinguish blur-before-interpretation from blur-after-interpretation. Cover native negative, slide, print scan, and the negative conversion path. Zero blur must preserve the corresponding no-blur result; uniform fields must remain uniform with blur active.
- Changing only print viewing glare must leave film and print density unchanged and alter the linear readout before downstream interpretation. With glare disabled, spatially varying glare must not leak into calibration or other routes.
- CPU f64, native-f32, resident, per-stage, Preview, and Export must use the same optical order. A GPU request must implement that order or fail under [GPU execution](../gpu-execution/spec.md#backend-selection), not append blur to an already interpreted image. Cropped and tiled readout must include the required optical context and match full-frame readout within measured budgets.
