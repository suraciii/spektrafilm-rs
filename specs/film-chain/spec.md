# Film chain

## Contract

- The chain accepts a normalized image, a film profile, a print profile when the selected route uses print stages, and runtime controls.
- It renders either a direct film scan or a film-print-scan. The selected route determines whether print stages run.
- The default path is: source color and exposure → spectral film exposure and development → film-domain effects (including V1 grain) → optional print exposure and development → scanner → destination color transform and encoding → scanner-domain effects (including V2 grain) → output handoff.
- Film, paper, spectral, and color profiles are data inputs. A missing or invalid profile is an error before output publication.
- Spatial, optical, and stochastic effects keep their requested semantics. A backend must not silently disable or replace them to remain on a fast path.
- CPU f64 is the reference and export path. WGPU f32 is an interactive or explicitly selected fast path; it may use CPU stages for unsupported effects and is not f64 reference output.
- The chain preserves floating headroom until a bounded image writer clips and encodes it.
- Invalid color spaces, routes, dimensions, crops, resize factors, and effect parameters fail before a final output is written.
