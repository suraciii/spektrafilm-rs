# Output mix and material controls design

## Ownership

The [product contract](spec.md) owns scope, controls, retirement, and acceptance. `core::runtime` remains the production entry point. Existing material stages own exposure, chemistry and density formation. Existing scanner preparation owns its references and inverses. Output composition belongs to core, not GUI widgets, the CLI, the image writer, or an independent RGB-response pipeline.

No prepared digital-response object, material-relative color coordinate system, tonal-mask pass, extra calibration render, or baseline/effective cache split is introduced. Preserve existing profile and development dependencies. The only added public parameter group is `output_mix`, containing f64 `amount`; use existing typed validation, preset, discovery and snapshot boundaries. Existing preflash and chemistry retain their storage types and capability checks.

## Print exposure composition

One existing derivation prepares the main filter pack `(C_neutral, M_neutral+M_shift, Y_neutral+Y_shift)`. Neutral-database lookup retains explicit-leaf protection. The preflash pack uses `(C_neutral, M_neutral+preflash_M_shift, Y_neutral+preflash_Y_shift)`. Main shifts must not leak into flash; preflash shifts must not modify the main illuminant. Calibrated cyan remains a reference, not a new creative shift.

Let `P` be main spectral exposure after filtration and transmission through film. Let `a` be the existing midgray normalization factor, `e` the existing print exposure, `b` the existing black-and-white print correction, and `F0` the preflash spectral vector. Preserve the current preflash derivation: integrate the independently filtered illuminant through the film base against print sensitivity, then multiply by `preflash_exposure`. Do not normalize each flash channel to an arbitrary RGB white.

Ignoring only the historical numerical floors for this optical equation, main exposure is `M = (a*P)*e*b`, flash is `F = F0*e*b`, and the material receives `K(M) + F`. `K` is the selected linear optical propagation on main exposure, including enlarger diffusion where enabled. Nonlinear print development follows total exposure. In particular, existing print exposure scales both terms; preflash independence concerns the light path and filter pack, not a changed gain convention.

Current calibrated exposure uses an inner `log10(max(a*P+F0,0)+1e-10)`, reconstructs linear exposure, multiplies by `e` then `b`, and applies an outer floor/log before development. The cutover must preserve the finite-exposure conventions and arithmetic-order budget, including zero exposure, rather than silently replacing them with the idealized equation. The positive-exposure optical equation does not authorize moving a clamp across a sum for signed numerical samples. Retaining the combined exposure representation is allowed only under the equivalence proof below; explicit branch separation must measure any residual caused by floors, rounding or interpolation.

`K` must preserve constant fields at image and tile boundaries. Applying `K` to `M+F` is equivalent only when the implementation proves linearity and constant preservation for the actual operator, boundary policy and precision. No new nonuniform flash model or spatial-path selector is needed. Propagation cannot be moved after nonlinear development for fusion or LUT baking. Normal spectral rendering, internal spectral LUTs, CPU and GPU paths use the same exposure derivation. Unsupported GPU execution follows the explicit failure policy in [GPU execution](./gpu-execution/spec.md#backend-selection).

The scanner optical cutover remains independently owned by [Film chain](./film-chain/spec.md#optical-order-cutover). Separating preflash must not be used to excuse unrelated changes to film response, scan calibration or inverse reconstruction. There is no requirement that automatic scan correction preserve the full visual magnitude of every existing material edit; its established semantics stay unchanged.

## Output composition

Remove the standalone Output lightness parameter and its operation across runtime, discovery, GUI, shaders and serialization. Preserve existing chroma/gamut algorithms, scanner conversion and optional named appearances. Do not add a hidden compensation curve or rewrite a retired triplet into preflash, chemistry, or another control.

All final RGB paths, including Finished RGB, resident execution and final diagnostic collection, use one composition boundary after scanner-domain effects, existing output gamut processing, and Magazine appearance. Prepare the source before simulated camera metering/exposure. Reuse input orientation, requested rotation, crop and resize geometry; viewer downsampling and display-ICC conversion are not valid source branches. RAW processing, decoding and requested lens correction are already part of the input. Convert that source to linear output primaries through existing color-management code without creative gamut or tone processing.

For `m = output_mix.amount`, compose `(1-m)*source + m*result` in linear output RGB. If the complete result is retained encoded at this boundary, decode it using the actual configured output transfer, blend, and restore the requested output transfer once. If output encoding is disabled, do not infer a transfer from the space name. Saving-space conversion remains later. Do not clip either branch merely to mix it.

Branch before additional color conversion at endpoints. At 1 return the original completed buffer; do not prepare or retain a second full-resolution source. At 0 return the prepared source with the requested output encoding, without requiring a processed-buffer round trip. Both endpoints still validate the requested recipe and delivery. At intermediate amounts reuse the existing completed result and allocate source storage only when needed; no second stochastic render, per-pixel allocation, or duplicated material chain is allowed. GUI amount edits must invalidate final output under the existing snapshot/cache rules without inventing a second preset or render identity system.

Intermediate density/exposure taps are upstream and unchanged. Final collection with amount below 1 and no source branch fails before publication. The implementation must never substitute an inverted negative, viewport image, blank buffer or identity scan-stage input. Use the same rule for all admitted backends rather than silently forcing amount to 1.

## Persistence and delivery

The [preset contract](./presets/spec.md) owns schema version 3 and migration; [CLI parameters](./cli/spec.md) owns sparse composition and discovery. Complete GUI state stores the added group under the [GUI state compatibility contract](./presets/spec.md#gui-state-compatibility). Older complete looks and complete state/recipe documents gain only an absent output-mix default of 1.  No response fields or cyan shift are materialized.

Retired-field migration inspects explicit legacy presence before typed defaults erase it. Absent/null lightness is eligible only in a versioned legacy complete document; an explicit numeric triplet fails even if the previous algorithm ignored it. Current schemas and sparse validators do not retain a deprecated optional field. Withdrawn draft response and cyan keys fail as unknown fields. Candidate validation precedes source writes, active-state mutation and publication.

[LUT delivery](./lut-delivery/spec.md) owns topology and artifact provenance. Mix below 1 requires a complete source-to-output bake: each lattice input supplies the prepared source and the complete transformed result in the declared transport. A scan-stage lattice cannot reconstruct original source pixels. Reject segmented targets before writing artifacts. Spatial processing and image geometry retain existing bake neutralization rules; the bake must not advertise equivalence to active spatial rendering.

Image and LUT provenance must identify the selected amount, model revision, optical order, and actual backend/precision through existing reporting structures. A declared stage or version is not a timing observation or numerical proof. Render once, retain the completed result, and save it without applying mix or Magazine a second time.

## Numerical acceptance

Use the [product acceptance matrix](spec.md#acceptance) and [benchmark contract](./benchmark/spec.md); no new benchmark framework is required. Compare unchanged amount-1 paths against the established per-scenario budgets, with scanner-order differences isolated. Cross-backend bit identity is not promised.

Preflash evidence records the derived `F0`, main exposure, total material exposure, print density and output under fixed scan settings. Check both main and flash filter isolation, gain conventions, zero flash, and constant-field behavior. Compare photographic-paper highlight/detail tradeoffs on real profiles and images; do not use a toy logarithm to claim those outcomes. Existing developer exhaustion is not required to match a withdrawn digital compressor, and no saturation acceptance is attached to this scope.

Mix evidence records both same-geometry branches, selected output transfer, exact endpoints and intermediate linear arithmetic. Exercise negative and super-white samples, negative capture, RAW, output encoding on/off, immutable Export, retained Save, and missing-source diagnostics. Validate LUT complete-bake behavior separately from segmented rejection and spatial-neutralization limits. Parser/link checks and analytic smoke probes support specification consistency only; production image, native GUI and measured backend acceptance remain required for implementation.
