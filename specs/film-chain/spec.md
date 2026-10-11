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
- Calibration captures outside the profile range use finite endpoint tangent
  extrapolation. Negative and above-clear scanner RGB remains representable
  until the normal writer; no log-of-clipped-RGB operation is permitted.
- Positive interpretation runs after scanner spectral capture and scanner lens
  blur, then output gamut compression, unsharp mask, transfer encoding, and
  output handoff.
- `positive_scan` rejects scanner white/black correction and active Grain V2.
  It uses the CPU per-stage backend; resident WGPU execution is not eligible.
