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
