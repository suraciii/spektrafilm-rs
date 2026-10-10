# Film chain

## Contract

- The chain accepts a normalized image, a film profile, and runtime controls.
- A photographic print route must provide a photographic print profile.
- A magazine print appearance route must satisfy the [Magazine print color](../magazine-print-color/spec.md) contract.
- The chain renders one of three routes: a direct film scan, a film-print-scan, or a film scan followed by magazine print appearance.
- The default path is: source color and exposure → spectral film exposure and development → film-domain effects (including V1 grain) → the selected route → destination color transform and encoding → scanner-domain effects (including V2 grain) → output handoff.
- Film, photographic paper, spectral, and color profiles are data inputs. A missing or invalid selected data input is an error before output publication.
- The selected route preserves the requested color, spatial, optical, and stochastic semantics. A backend must not silently disable or replace them to remain on a fast path.
- CPU f64 is the reference and export path. WGPU f32 is an interactive or explicitly selected fast path; it may use CPU stages for unsupported effects and is not f64 reference output.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.

## Direct negative-film scan output

- `scanner.scan_output` is `direct_scan` by default. It preserves the
  selected medium's native scan polarity.
- `positive_scan` is valid only for the exact `input > film > scan` route with
  a negative film profile in the filming stage. Paper profiles are not eligible.
  It interprets the linear scanner capture as a positive image using clear-film
  and dense-film endpoint references derived from the selected profile.
  Endpoint interpretation must preserve values outside the reference interval.
- Positive interpretation runs after scanner spectral capture and scanner lens
  blur, then output gamut compression, unsharp mask, transfer encoding, and
  output handoff.
- `positive_scan` rejects scanner white/black correction and active Grain V2.
  It uses the CPU per-stage backend; resident WGPU execution is not eligible.
