# Film chain

## Contract

- Film workflows accept a normalized image, a film profile, and runtime controls. Finished RGB processing follows the [Magazine print color entry-point contract](../magazine-print-color/spec.md#enablement).
- A photographic print route must provide a photographic print profile.
- Optional magazine finishing must satisfy the [Magazine print color](../magazine-print-color/spec.md) contract. It must not determine base workflow topology.
- Base workflows include input color management, direct film scanning, photographic printing, and the existing film-conversion routes.
- The default film path is: source color and exposure → spectral film exposure and development → film-domain effects (including Spektra, Dye, and Fotufilm Film grain) → the selected base route → destination color transform and encoding → scanner-domain effects (including Procedural grain) → optional magazine finishing → output handoff.
- [Material controls and output composition](../material-response/spec.md) owns reuse of existing preflash and chemistry, retirement of Output lightness compression, and overall output mix. It adds no digital material-response layer or scan-reference isolation pass. Magazine appearance remains after scanner-domain effects; overall output mix follows it before output handoff.
- Film, photographic paper, spectral, and color profiles are data inputs. A missing or invalid selected data input is an error before output publication.
- The selected route preserves the requested color, spatial, optical, and stochastic semantics. A backend must not silently disable or replace them to remain on a fast path.
- Execution policy and CPU/GPU algorithm alignment follow [GPU execution](../gpu-execution/spec.md). CPU f64 remains the numerical reference; GPU execution must cover the selected chain without replacing its model.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.

## Scanner language

Scanner terms follow the [Film simulation language](../../CONTEXT.md). A reversal-film scan and a photographic-paper scan already read positive images.

The scan output value `negative_to_positive` selects negative conversion. The existing `density`, `separable`, and `coupled` methods reconstruct relative exposure through the selected film's response, then apply a base positive tone response. Automatic image correction and scanner finishing remain independent controls; conversion must work with `scanner.auto_setup=off` and `scanner.finish=neutral`.

Image correction, scanner finishing, and digital output must not select a different scanned medium or a photographic print route.

`scanner.scan_output` accepts only `direct_scan` and `negative_to_positive`. The removed value `positive_scan` must fail validation in CLI options, runtime parameters, sparse sources, presets, render recipes, and imported GUI state. Readers must not alias it or silently reset it. Omitted scan output retains the `direct_scan` default. Existing schema versions and parameter field names remain unchanged.

The [Digital Reference contract](digital-reference/spec.md) defines a common reference conversion within Scan. It does not add a scan output polarity.

## Scan output naming acceptance

1. Discovery, CLI help, effective parameter reports, saved presets, recipes, and GUI state must emit the canonical scan output value.
2. GUI controls must display `Preserve medium polarity` for `direct_scan`, `Negative to positive` for `negative_to_positive`, and `Negative conversion method` for `scanner.positive_interpretation`.
3. Replacing the removed value with `negative_to_positive` must preserve pixels, defaults, route eligibility, correction precedence, and execution backend for identical inputs and controls. Verify density, separable, coupled, direct-negative, reversal-film, and paper paths with the same executable precision.
4. Removed-value rejection must occur before output publication. A failed GUI import must retain the accepted parameters and image.
5. Conversion telemetry must identify stage `negative_to_positive` and its actual executor under [Telemetry](../telemetry/metrics.md#effective-configuration). Selecting this mode must not require a CPU reason or a resident decline.

## Scan medium and scanner controls

- The selected route determines the scanned medium. A film-scan route scans the
  film; a print route scans the photographic paper.
- The scanned medium determines the scanner controls that apply.
- A photographic-paper scan and a slide film scan read a positive image. A
  negative film scan reads the negative image on the film; `negative_to_positive`
  produces the positive image by negative conversion.
- Every scan exposes scanner lens blur and scanner unsharp mask.
- Scanner white and black correction, with its white and black levels, applies
  to a photographic-paper scan and a slide film scan. It does not change a
  negative film scan.
- Film-scan black and white points apply to a slide film scan and to a
  negative film scan with `negative_to_positive`. See
  [Film-scan black and white points](#film-scan-black-and-white-points).
- A negative film scan exposes the scan output mode. Negative conversion
  and its controls belong to `negative_to_positive`. A photographic-paper scan and a
  slide film scan must not expose them. See
  [Direct negative-film scan output](#direct-negative-film-scan-output).
- An appearance applied to a completed paper or film scan belongs to finishing. Existing exposure and development controls belong to their material stage. The optional Fotufilm-derived scanner finish remains a film-scan treatment.
- Scanner calibration, inverse curves, point ranges, correction references, and automatic setup retain their existing dependencies on profile and development settings. Material-control simplification must not add a second reference model or alter the real captured-negative recovery solve.
- The product may model a scanner receiver response. A receiver profile must identify its spectral response, reference levels, source, and calibration status.

## Scan optics and print viewing glare

Scanner lens blur must model readout optics on linear transmitted or reflected light. It must run before nonlinear density or exposure interpretation, scanner points, white/black correction, scanner finishing, output gamut processing, unsharp mask, and transfer encoding. Direct scans and identity scanner points must not select a later blur position. The operation must not blur developed density or encoded RGB as a substitute for optical mixing.

Spectral integration may precede blur when the receiver is linear and spatially uniform and the same achromatic spatial kernel applies to every channel. Otherwise blur must occur in the corresponding light domain before integration. Changing destination primaries must not change the optical kernel or where it runs. Scanner blur must not change the developed medium or the input recovery solve of a convert-film route.

`print_render.glare` represents print viewing/readout stray light. It must be added in the linear readout-light domain after scanner lens blur and before scanner correction or nonlinear interpretation. It must not change print exposure, chemistry, or developed print density. It remains eligible only on the existing print-scan routes, with its existing controls and seed behavior. Film-scan routes must not acquire this effect through `film_render.glare` or another alias.

Unsharp mask is digital scan sharpening, not lens resolution. It remains downstream of interpretation, optional scanner finish, and output gamut processing. Procedural grain retains its separate [post-scan boundary](../film-grain/v2/spec.md); it must not be moved into optical acquisition to make the grain engines appear equivalent.

## Optical-order cutover

The historical direct-scan sequence that blurs after output gamut processing must not remain as a compatibility mode. All admitted scan routes must obey [Scan optics and print viewing glare](#scan-optics-and-print-viewing-glare). Parameter names, units, and zero-blur meaning remain unchanged. Nonzero blur on a previously late-blur path may intentionally change pixels even with unchanged material settings and full output amount.

The implementation must identify the new ordering in its render/model version and user-facing change notes. Recipe replay must report the model version used and must not claim historical pixel identity. It must not silently rescale blur or scanner points to emulate the old order. Existing before/after budgets and photographic evidence must isolate this cutover from preflash branch separation and output mixing.

The [Print exposure composition](../material-response/design.md#print-exposure-composition) fixes the print optical boundary. Separating a uniform preflash must preserve the old result within its numerical budget where the old linear constant-preserving operator was equivalent, including existing exposure gains and finite-exposure floors. This separation is not a general exemption from numerical preservation.


## Scanner receiver and interpretation

- `scanner.receiver_profile` selects the scanner receiver used by a film-scan route. `legacy_colorimetric` is the default and preserves the existing spectral-to-output behavior. `lab_scan_approx` is an approximate narrow-band receiver profile.
- A receiver profile changes scanner acquisition only. It must not change film exposure, development, film-domain grain, halation, print exposure, or the input camera exposure.
- The receiver profile must be resolved before scan calibration. Direct film scans and capture-based positive interpretations must use the same receiver capture for one render.
- `input > convert-film > scan` must treat the incoming image as the input capture for the conversion stage. The selected receiver applies to the output scan stage and must not be applied a second time to the input capture.
- `scanner.positive_interpretation` selects `density`, `separable`, `coupled`, or `digital_reference` when `scanner.scan_output=negative_to_positive`. `density` is the default. The matched receiver, controls, route restrictions, and reference response for `digital_reference` follow the [Digital Reference contract](digital-reference/spec.md).
- `density` reads record densities through the film's own monotone characteristic curves. It acquires no receiver capture and must produce identical output for identical record densities and scanner controls, whatever `scanner.receiver_profile` selects.
- `separable` uses the per-channel monotone inverse defined by [Direct negative-film scan output](#direct-negative-film-scan-output).
- `coupled` uses a three-channel inverse of the resolved film, receiver, and output response. It must be prepared once per render configuration and evaluated per pixel through a finite lookup table or an equivalent bounded interpolation structure.
- A coupled inverse must use the CPU f64 path as its reference. It must preserve finite endpoints, bounded output, and continuous behavior at the film toe, mid-range, and shoulder. It must not run an iterative solver per image pixel.
- `scanner.auto_setup` selects `off` or `per_frame`. `off` is the default. `per_frame` derives scanner exposure, channel balance, and bounded contrast placement from the full input before crop and resize.
- Per-frame setup may use image measurements to derive scanner controls. It must not derive or replace the receiver response, film profile, or positive calibration from image content.
- One immutable per-frame setup must be shared by Preview, Scan, and Export for the same input and parameter revision. Changing the input, route, profile, or relevant scanner parameter must invalidate it.
- Per-frame setup must not change film exposure, development, film-domain effects, camera exposure compensation, or the conversion solve. For a real negative capture, it must not apply virtual camera metering.
- `scanner.finish` selects `neutral` or `lab_scan` for a converted negative image. `neutral` is the default. `scanner.finish_strength` ranges from 0 to 1 and defaults to 1 when `lab_scan` is selected.
- Scanner finishing runs after negative conversion and before output gamut compression, unsharp masking, transfer encoding, and output handoff. It must not run on a direct negative output.
- Scanner finishing must be independently disableable from the receiver profile and negative conversion. Disabling it must return the converted image before scanner finishing, subject to the shared output stages.
- Receiver profiles, negative conversion methods, per-frame setup, and scanner finishes must be serialized in complete GUI state and render recipes. Preset documents may carry only the scanner controls allowed by the [preset specification](../presets/spec.md#look-ownership), with receiver and finish provenance retained when present. An unavailable profile or invalid combination must fail before output publication.

## Film-scan black and white points

- `scanner.black_point` and `scanner.white_point` are three-channel positions
  on the point range of the scanned film. A photographic-paper scan and a
  negative film scan with `direct_scan` must not expose them.
- On a negative film scan with `negative_to_positive`, the point range is the relative
  log-exposure range that the derived inverse covers in all three channels.
  `0.0` is the lowest exposure of that range and `1.0` the highest.
- On a slide film scan, each channel's scanned density is the negative base-10
  logarithm of its capture divided by its zero-dye capture. The shared range
  runs from zero to the largest channel density of the maximum-dye reference.
  `0.0` is that shared maximum density, and `1.0` is zero density.
- The defaults `0.0` and `1.0` leave the scan unchanged.
- Each channel must satisfy `0.0 <= black_point < white_point <= 1.0`.
- Point-map scales, mapped endpoint captures, and endpoint tangents must be
  representable in the selected computation precision. An unrepresentable
  active window must fail with an error before output publication.
- Each channel linearly maps its black point to the start of the range and its
  white point to the end. Values outside the points continue the same linear
  map; the points do not clip.
- A point pair acts on its own channel only. Points that are equal across the
  three channels apply the same map to every channel.
- On a negative film scan, the points act on the estimated relative log
  exposure before the density brightness, RGB balance, and contrast controls.
  Points that are equal across the three channels must not introduce a color
  cast on a neutral ramp.
- On a slide film scan, non-default points act on the scanner capture after
  scanner spectral capture and scanner lens blur, and before scanner white
  and black correction. The correction keeps its references and its levels.
  Default points must bypass their mapping without changing the optical order.
  Captures outside the reference range use finite endpoint tangent
  extrapolation without taking a logarithm of clipped or nonpositive values.

## Direct negative-film scan output

- `scanner.scan_output` is `direct_scan` by default. It preserves the
  selected medium's native scan polarity.
- `negative_to_positive` is valid for a negative film profile on either the direct
  `input > film > scan` route or the real negative-capture
  `input > convert-film > scan` route. Paper profiles, print-paper routes,
  and `scan-minus-base` are not eligible.
- Negative conversion input is record density by default. `density` reads
  each dye record's density, as a narrowband scanner does, and derives a
  per-record monotone inverse from the selected profile density curves. One
  record's output must not depend on another record's density. The simulated
  `input > film > scan` route hands it the film stage's record densities and
  inserts no synthetic capture.
- `separable` and `coupled` interpret a resolved receiver capture instead.
  `separable` derives a per-channel monotone inverse from the selected profile
  density curves after converting the capture to density through its per-channel
  clear reference. `coupled` uses the three-channel inverse defined in
  [Scanner receiver and interpretation](#scanner-receiver-and-interpretation).
  A real negative-capture `input > convert-film > scan` route and the exact-cell
  scan path interpret their capture with one of these methods; `density` behaves
  as `separable` where the incoming image is a capture.
- Every exposure-reconstruction method estimates relative log exposure, subtracts camera exposure
  compensation only on the simulated `input > film > scan` route, applies the
  configured density brightness, RGB balance, contrast, and maximum-density
  controls, then emits output-space linear RGB. Zero balance renders a grey lit
  by the profile reference illuminant neutral.
- Densities outside the profile curves and calibration captures outside the
  profile range use finite endpoint tangent extrapolation; the tone curve keeps
  output finite and within the configured density range. Negative and
  above-clear scanner RGB remains representable until the normal writer; no
  log-of-clipped-RGB operation is permitted.
- Scanner lens blur acts on transmitted light before the density reading, and on
  the capture before a capture interpretation. Optional scanner finishing runs
  after negative conversion. Output gamut compression, unsharp mask,
  transfer encoding, and output handoff run after scanner finishing.
- `negative_to_positive` rejects scanner white/black correction and active Procedural grain.
  Its execution follows [GPU execution](../gpu-execution/spec.md#backend-selection); conversion selection must not force CPU.

## Scanner acceptance

- `legacy_colorimetric` with `separable`, `off`, and `neutral` must preserve the existing scan contract within the established CPU f64 reference budget, except for the explicitly identified [optical-order cutover](#optical-order-cutover).
- Changing `scanner.receiver_profile` must not change film density, film-domain grain, halation, print exposure, or camera exposure. It must change only the scan acquisition and the downstream interpretation of that capture.
- `direct_scan` must preserve the native polarity of the selected medium. Scanner finishing must not run on a direct negative-film output.
- Capture-based negative conversion must use one receiver capture for calibration and image pixels. It must not rebuild spectral calibration per pixel.
- `density` must not change its output when only `scanner.receiver_profile` changes, and it must not acquire a receiver capture. `separable` and `coupled` must change only through the capture the receiver profile produces.
- `coupled` must produce finite, continuous output across the calibration range and finite tangent extrapolation outside it. It must not run an iterative solver per image pixel.
- `per_frame` must measure the full input before crop and resize. Preview, Scan, and Export must produce the same scanner setup for the same input and parameter revision.
- Changing film exposure, development, grain, halation, print exposure, or camera exposure while holding scanner settings constant must not be an effect of receiver or scanner finish selection.
- Disabling scanner finishing must expose the calibrated scan result before finishing, subject to the shared output stages.
- Approximate receiver profiles must be labeled approximate in GUI, CLI diagnostics, and serialized provenance. A measured-device label requires measured response data and calibration provenance.
- Receiver, interpretation, setup, and finish combinations that the selected route or medium cannot support must fail before output publication and must produce the same validation result for GUI, CLI dry run, and rendering.
- A nonuniform bright-edge capture and an active nonlinear downstream transform must distinguish blur-before-interpretation from blur-after-interpretation. Cover native negative, slide with identity and nonidentity points, print scan, and admitted positive interpretation. Zero blur must preserve the corresponding no-blur result; uniform fields must remain uniform with blur active.
- Changing only print viewing glare must leave film and print density unchanged and alter the linear readout before downstream correction. With glare disabled, spatially varying glare must not leak into calibration or other routes.
- CPU f64, native-f32, resident, per-stage, Preview, and Export must use the same optical order. A GPU request must implement that order or fail under [GPU execution](../gpu-execution/spec.md#backend-selection), not append blur to an already interpreted image. Cropped and tiled readout must include the required optical context and match full-frame readout within measured budgets.
