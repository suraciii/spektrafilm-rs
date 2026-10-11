# Film chain

## Contract

- Film workflows accept a normalized image, a film profile, and runtime controls. Finished RGB processing follows the [Magazine print color entry-point contract](../magazine-print-color/spec.md#enablement).
- A photographic print route must provide a photographic print profile.
- Optional magazine finishing must satisfy the [Magazine print color](../magazine-print-color/spec.md) contract. It must not determine base workflow topology.
- Base workflows include input color management, direct film scanning, photographic printing, and the existing film-conversion routes.
- The default film path is: source color and exposure → spectral film exposure and development → film-domain effects (including V1 grain) → the selected base route → destination color transform and encoding → scanner-domain effects (including V2 grain) → optional magazine finishing → output handoff.
- Film, photographic paper, spectral, and color profiles are data inputs. A missing or invalid selected data input is an error before output publication.
- The selected route preserves the requested color, spatial, optical, and stochastic semantics. A backend must not silently disable or replace them to remain on a fast path.
- CPU f64 is the reference and export path. WGPU f32 is an interactive or explicitly selected fast path; it may use CPU stages for unsupported effects and is not f64 reference output.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.

## Scan medium and scanner controls

- The selected route determines the scanned medium. A film-scan route scans the
  film; a print route scans the photographic paper.
- The scanned medium determines the scanner controls that apply.
- A print route carries its positive image on the photographic paper, and its
  scan reads that paper. A negative film-scan route carries the negative image
  on the film; `positive_scan` produces the positive image by positive-film
  interpretation.
- Scanner capture corrections are scanner lens blur, scanner white and black
  correction with its white and black levels, and scanner unsharp mask.
- A photographic-paper scan exposes the scanner capture corrections only. It
  must not expose the scan light or the scan polarity.
- A film scan exposes the scan light, the scanner capture corrections, and the
  scan polarity. A slide film scan and a negative film scan expose the same
  scanner controls.
- The scan light is the spectrum that reads the film. `scanner.illuminant`
  selects it and accepts the same illuminant names as `enlarger.illuminant`. An
  unset `scanner.illuminant` follows the film profile viewing illuminant.
- Scanner white and black correction applies to a photographic-paper scan and
  to a slide film scan. It must not change a negative film scan.
- Positive-film interpretation belongs to a negative film profile with
  `positive_scan`. A photographic-paper scan and a slide film scan must not
  expose it. See
  [Direct negative-film scan output](#direct-negative-film-scan-output).
- Look that must apply to both a photographic-paper scan and a film scan
  belongs to the finishing treatment, not to the scanner controls. See
  [Magazine print color](../magazine-print-color/spec.md).
- The product must not model a physical scanner device. It must not expose a
  measured sensor response, transport artifacts, or dust controls.

## Direct negative-film scan output

- `scanner.scan_output` is `direct_scan` by default. It preserves the
  selected medium's native scan polarity.
- `positive_scan` is valid for a negative film profile on either the direct
  `input > film > scan` route or the real negative-capture
  `input > convert-film > scan` route. Paper profiles, print-paper routes,
  and `scan-minus-base` are not eligible.
- Positive interpretation derives a per-channel monotone inverse from the
  selected profile density curves after the same spectral scan context. It
  estimates relative log exposure, subtracts camera exposure compensation
  only on the simulated `input > film > scan` route, applies the configured
  density brightness, RGB balance, contrast, and maximum-density controls,
  then emits output-space linear RGB.
- `scanner.positive_black_point` and `scanner.positive_white_point` are
  three-channel positions inside the density window that the derived inverse
  covers. `0.0` is the channel's minimum interpreted density and `1.0` its
  maximum. Their defaults are the unclipped window: `0.0` and `1.0`.
- Every channel must keep its black point below its white point, with both
  points inside the window. A capture at or below the black point maps to
  output black, and a capture at or above the white point maps to output white.
- A point pair acts on its own channel only. Points that stay equal across the
  three channels must not introduce a color cast on a neutral ramp. The points
  act on the estimated relative log exposure before the density brightness, RGB
  balance, and contrast controls.
- The built-in preset library must ship one scanner section preset that
  provides starting values for the interpretation points.
- Calibration captures outside the profile range use finite endpoint tangent
  extrapolation. Negative and above-clear scanner RGB remains representable
  until the normal writer; no log-of-clipped-RGB operation is permitted.
- Positive interpretation runs after scanner spectral capture and scanner lens
  blur, then output gamut compression, unsharp mask, transfer encoding, and
  output handoff.
- `positive_scan` rejects scanner white/black correction and active Grain V2.
  It uses the CPU per-stage backend; resident WGPU execution is not eligible.
