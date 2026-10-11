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
- Look that must apply to both a photographic-paper scan and a film scan
  belongs to the finishing treatment, not to the scanner controls. See
  [Magazine print color](../magazine-print-color/spec.md).
- The product must not model a physical scanner device. It must not expose a
  measured sensor response, transport artifacts, or dust controls.

## Film-scan black and white points

- `scanner.black_point` and `scanner.white_point` are three-channel positions
  on the point range of the scanned film. A photographic-paper scan and a
  negative film scan with `direct_scan` must not expose them.
- On a negative film scan with `positive_scan`, the point range is the relative
  log-exposure range that the derived inverse covers in all three channels.
  `0.0` is the lowest exposure of that range and `1.0` the highest.
- On a slide film scan, the point range is the scanned density range shared by
  the three channels. `0.0` is the highest scanned density of the film's
  maximum dye density, and `1.0` is zero dye density.
- The defaults `0.0` and `1.0` leave the scan unchanged.
- Each channel must satisfy `0.0 <= black_point < white_point <= 1.0`.
- Each channel linearly maps its black point to the start of the range and its
  white point to the end. Values outside the points continue the same linear
  map; the points do not clip.
- A point pair acts on its own channel only. Points that are equal across the
  three channels apply the same map to every channel.
- On a negative film scan, the points act on the estimated relative log
  exposure before the density brightness, RGB balance, and contrast controls.
  Points that are equal across the three channels must not introduce a color
  cast on a neutral ramp.
- On a slide film scan, the points act on the scanner capture after scanner
  spectral capture and scanner lens blur, and before scanner white and black
  correction. The correction keeps its references and its levels.

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
