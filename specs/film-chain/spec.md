# Film chain

## Contract

- Film workflows accept a normalized image, a film profile, and runtime controls. Finished RGB processing follows the [Magazine print color entry-point contract](../magazine-print-color/spec.md#enablement).
- A photographic print route must provide a photographic print profile.
- Optional magazine finishing must satisfy the [Magazine print color](../magazine-print-color/spec.md) contract. It must not determine base workflow topology.
- Base workflows include input color management, direct film scanning, photographic printing, and the existing film-conversion routes.
- [Film and print digital response](../material-response/spec.md) owns creative material response, scan-reference isolation, and the retirement of Output lightness compression. Film and Print response must alter their material outputs, not completed RGB. Magazine appearance remains after scanner-domain effects. Overall output mix follows Magazine appearance before output handoff.
- Film, photographic paper, spectral, and color profiles are data inputs. A missing or invalid selected data input is an error before output publication.
- The selected route preserves the requested color, spatial, optical, and stochastic semantics. A backend must not silently disable or replace them to remain on a fast path.
- CPU f64 is the reference and export path. WGPU f32 is an interactive or explicitly selected fast path; it may use CPU stages for unsupported effects and is not f64 reference output.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.

## Scan medium and scanner controls

- The selected route determines the scanned medium. A film-scan route scans the
  film; a print route scans the photographic paper.
- The scanned medium determines the scanner controls that apply.
- A photographic-paper scan and a slide film scan read a positive image. A
  negative film scan reads the negative image on the film; `positive_scan`
  produces the positive image by positive interpretation.
- Every scan exposes scanner lens blur and scanner unsharp mask.
- Scanner white and black correction, with its white and black levels, applies
  to a photographic-paper scan and a slide film scan. It does not change a
  negative film scan.
- Film-scan black and white points apply to a slide film scan and to a
  negative film scan with `positive_scan`. See
  [Film-scan black and white points](#film-scan-black-and-white-points).
- A negative film scan exposes the scan output mode. Positive interpretation
  and its controls belong to `positive_scan`. A photographic-paper scan and a
  slide film scan must not expose them. See
  [Direct negative-film scan output](#direct-negative-film-scan-output).
- An appearance applied to a completed paper or film scan belongs to finishing. A creative adjustment to material formation belongs to [Film or Print response](../material-response/spec.md), even when it affects both delivery routes. The optional Fotufilm-derived scanner finish remains a film-scan treatment.
- [Scan reference isolation](../material-response/spec.md#scan-reference-isolation) must apply to inverse calibration, scanner point ranges, scanner correction references, and automatic setup. Receiver capture must read effective material density; calibration must not invert away its creative adjustments.
- The product may model a scanner receiver response. A receiver profile must identify its spectral response, reference levels, source, and calibration status.

## Scanner receiver and interpretation

- `scanner.receiver_profile` selects the scanner receiver used by a film-scan route. `legacy_colorimetric` is the default and preserves the existing spectral-to-output behavior. `lab_scan_approx` is an approximate narrow-band receiver profile.
- A receiver profile changes scanner acquisition only. It must not change film exposure, development, film-domain grain, halation, print exposure, or the input camera exposure.
- The receiver profile must be resolved before scan calibration. Direct film scans and positive interpretations must use the same receiver capture for one render.
- `input > convert-film > scan` must treat the incoming image as the input capture for the conversion stage. The selected receiver applies to the output scan stage and must not be applied a second time to the input capture.
- `scanner.positive_interpretation` selects `separable` or `coupled` positive interpretation when `scanner.scan_output=positive_scan`. `separable` is the default.
- `separable` uses the per-channel monotone inverse defined by [Direct negative-film scan output](#direct-negative-film-scan-output).
- `coupled` uses a three-channel inverse of the resolved film, receiver, and output response. It must be prepared once per render configuration and evaluated per pixel through a finite lookup table or an equivalent bounded interpolation structure.
- A coupled inverse must use the CPU f64 path as its reference. It must preserve finite endpoints, bounded output, and continuous behavior at the film toe, mid-range, and shoulder. It must not run an iterative solver per image pixel.
- `scanner.auto_setup` selects `off` or `per_frame`. `off` is the default. `per_frame` derives scanner exposure, channel balance, and bounded contrast placement from the full input before crop and resize.
- Per-frame setup may use image measurements to derive scanner controls. It must not derive or replace the receiver response, film profile, or positive calibration from image content.
- One immutable per-frame setup must be shared by Preview, Scan, and Export for the same input and parameter revision. Changing the input, route, profile, or relevant scanner parameter must invalidate it.
- Per-frame setup must not change film exposure, development, film-domain effects, camera exposure compensation, or the conversion solve. For a real negative capture, it must not apply virtual camera metering.
- `scanner.finish` selects `neutral` or `lab_scan` for positive scan output. `neutral` is the default. `scanner.finish_strength` ranges from 0 to 1 and defaults to 1 when `lab_scan` is selected.
- Scanner finishing runs after positive interpretation and before output gamut compression, unsharp masking, transfer encoding, and output handoff. It must not run on a direct negative output.
- Scanner finishing must be independently disableable from the receiver profile and positive interpretation. Disabling it must return the calibrated scan before scanner finishing, subject to the shared output stages.
- Receiver profiles, positive interpretation methods, per-frame setup, and scanner finishes must be serialized in complete GUI state and render recipes. Preset documents may carry only the scanner controls allowed by the [preset specification](../presets/spec.md#look-ownership), with receiver and finish provenance retained when present. An unavailable profile or invalid combination must fail before output publication.

## Film-scan black and white points

- `scanner.black_point` and `scanner.white_point` are three-channel positions
  on the point range of the scanned film. A photographic-paper scan and a
  negative film scan with `direct_scan` must not expose them.
- On a negative film scan with `positive_scan`, the point range is the relative
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
  Default points retain the existing capture, correction, and blur order.
  Captures outside the reference range use finite endpoint tangent
  extrapolation without taking a logarithm of clipped or nonpositive values.

## Direct negative-film scan output

- `scanner.scan_output` is `direct_scan` by default. It preserves the
  selected medium's native scan polarity.
- `positive_scan` is valid for a negative film profile on either the direct
  `input > film > scan` route or the real negative-capture
  `input > convert-film > scan` route. Paper profiles, print-paper routes,
  and `scan-minus-base` are not eligible.
- Positive interpretation uses the resolved receiver capture. `separable` derives a per-channel monotone inverse from the baseline profile density curves, excluding the new creative response. `coupled` uses the three-channel inverse defined in [Scanner receiver and interpretation](#scanner-receiver-and-interpretation), with the same baseline-reference boundary. Both modes estimate relative log exposure, subtract camera exposure compensation only on the simulated `input > film > scan` route, apply the configured density brightness, RGB balance, contrast, and maximum-density controls, then emit output-space linear RGB.
- Calibration captures outside the profile range use finite endpoint tangent
  extrapolation. Negative and above-clear scanner RGB remains representable
  until the normal writer; no log-of-clipped-RGB operation is permitted.
- Positive interpretation runs after scanner spectral capture and scanner lens
  blur. Optional scanner finishing runs after positive interpretation. Output
  gamut compression, unsharp mask, transfer encoding, and output handoff run
  after scanner finishing.
- `positive_scan` rejects scanner white/black correction and active Procedural grain.
  It uses the CPU per-stage backend; resident WGPU execution is not eligible.

## Scanner acceptance

- `legacy_colorimetric` with `separable`, `off`, and `neutral` must preserve the existing scan contract within the established CPU f64 reference budget.
- Changing `scanner.receiver_profile` must not change film density, film-domain grain, halation, print exposure, or camera exposure. It must change only the scan acquisition and the downstream interpretation of that capture.
- `direct_scan` must preserve the native polarity of the selected medium. Scanner finishing must not run on a direct negative-film output.
- `positive_scan` must use one receiver capture for calibration and image pixels. It must not rebuild spectral calibration per pixel.
- `coupled` must produce finite, continuous output across the calibration range and finite tangent extrapolation outside it. It must not run an iterative solver per image pixel.
- `per_frame` must measure the full input before crop and resize. Preview, Scan, and Export must produce the same scanner setup for the same input and parameter revision.
- Changing film exposure, development, grain, halation, print exposure, or camera exposure while holding scanner settings constant must not be an effect of receiver or scanner finish selection.
- Disabling scanner finishing must expose the calibrated scan result before finishing, subject to the shared output stages.
- Approximate receiver profiles must be labeled approximate in GUI, CLI diagnostics, and serialized provenance. A measured-device label requires measured response data and calibration provenance.
- Receiver, interpretation, setup, and finish combinations that the selected route or medium cannot support must fail before output publication and must produce the same validation result for GUI, CLI dry run, and rendering.
