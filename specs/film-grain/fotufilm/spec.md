# Fotufilm Film grain

## Identity and scope

`fotufilm` MUST select the independent Fotufilm Film grain engine. The native GUI MUST display `Fotufilm Film`. Spektra MUST remain the default engine.

Selecting Fotufilm MUST NOT change the selected film or paper profile.

The engine MUST adapt the Film mechanism from Fotufilm revision `59d43471dbef45c85ae442e507a291fdebfb2ec5`. The image's current resolved spectral film profile MUST remain the authority for mean developed color and tone. The engine MUST add a grain residual after film development and DIR and before film scan or print exposure.

It MUST NOT add encoded RGB noise or replace the characteristic curve with its fitted population mean.

The supported configuration is an appearance model with explicitly authored effective grain parameters. Defaults MUST NOT imply stock-specific measured calibration or equality with Fotufilm's full renderer. The model MUST support color negative, dye reversal, silver monochrome and explicitly selected chromogenic monochrome material laws on valid resolved film curves.

It MUST reject invalid, nonfinite, empty or unsupported curve inputs with an actionable error. Curve-fit and final anchor convergence metrics MUST remain observable; convergence failure MUST NOT silently select a different noise model.

Film, print-film and film-scan routes MUST execute the model once when the film-development boundary is traversed. Collecting `cmy_film` MUST include that execution. Injecting already developed `cmy_film` or a later tap MUST NOT execute it again.

Passthrough, print-paper and convert-film MUST preserve their existing boundaries and MUST NOT synthesize new grain without film development.

## Parameters

`film_render.grain.fotufilm` MUST contain the independent engine's controls. Controls belonging to other engines MUST NOT change this model. All scalar and array coefficients below MUST be finite.

`amount` MUST range from 0 to 4 and default to 1. `size` and `softness` MUST range from 0.5 to 4 and default to 1. `color` MUST range from 0 to 1 and default to 1.

`layers` MUST contain three per-record amount multipliers from 0 to 4 and default to `[1, 1, 1]`.

`rms_density` MUST be strictly positive and at most 0.1 density units, with default 0.009. It is an effective model matching target through the model's 48-micrometer aperture approximation, not a published figure for the current film. `clump_radius_um` MUST be strictly positive and at most 32 micrometers, with default 5.

It describes the fitted fastest-bin scale, not an individual crystal radius. `layer_weights` MUST contain three strictly positive coefficients at most 4 and default to `[0.7, 1, 1.35]`. `layer_size_ratio` MUST contain three coefficients from 0.25 to 4 and default to `[1, 1, 1]`.

`radius_span` MUST range from 1 to 20 and default to 6. `sublayer_shares` MUST contain four strictly positive fitted-density allocation shares summing to 1 within absolute tolerance 0.000001. Its default MUST be `[0.25, 0.25, 0.25, 0.25]`.

The shares MUST NOT be interpreted as crystal-count or silver-mass fractions. `coating_density_scale` MUST range from 0.25 to 4 and default to 1. `fog_density` MUST range from 0 to 0.3 density units and default to 0.03.

`material` MUST accept `auto`, `silver`, `dye_negative` and `dye_reversal` and default to `auto`. Auto MUST select silver for a monochrome film, dye reversal for a positive color film, and dye negative for a negative color film; a monochrome stock is a silver image whether its profile is negative or positive.

Explicit material selection MUST allow chromogenic monochrome to use a dye law without changing the profile's channel model.

Film population parameters MUST be independent of runtime Amount, Size, Softness, Color, Layers, seed and raster size. Changes to effective population parameters or resolved reference/development curves MUST invalidate prepared population data. No stock-name heuristic MAY substitute another film's grain metadata.

## Image and backend behavior

`active = false`, effective `amount = 0`, and runtime stochastic-effect deactivation MUST be exact existing-path bypasses. They MUST prepare no grain bank. LUT generation MUST deactivate grain and prepare no bank.

The selected executor MUST implement Fotufilm preparation and sampling with the same model on CPU and GPU. GPU MUST preserve source/reference conditioning for formation, fitted population, crystal marks, and residual sampling, including exact discrete decisions where the model defines them. Backend selection, residency, and failure behavior follow [GPU execution](../../gpu-execution/spec.md#backend-selection) and [algorithm alignment](../../gpu-execution/spec.md#algorithm-alignment).

GPU execution MUST NOT use CPU-prepared stochastic population or bank data, CPU grain arithmetic, a Gaussian or other generic stochastic substitute, or another engine. Floating outputs need not be bit-identical between f32 and f64 builds.

The full-source film extent MUST determine physical pitch. Cropping MUST retain absolute film coordinates. Actual rounded output dimensions MUST determine both output pitches.

Downstream spatial processing MUST receive a lattice covering the full film before the requested crop is extracted. Preview MUST evaluate the same model on its declared reduced-resolution source; it MUST NOT claim pixel equality with full Scan or Export.

Every requested finite positive footprint MUST be read in full through the chosen periodic tile placement. The model MUST NOT silently truncate the footprint at a block margin. The complete-periodic readout extension MUST be identified as an approximation distinct from the upstream large-footprint clipped sampler.

Rectangular footprints MUST use their actual axis sizes. Geometry and resource limits MUST be checked before allocation with an actionable error.

Grain coordinates outside the population's density interval MUST use the nearest endpoint's grain statistics while preserving the unmodified developed CMY target. This is an explicit appearance approximation, not extrapolated stock calibration. Numerical or image density MUST NOT be clipped merely to satisfy the grain lookup.

## State, cancellation and delivery

CLI parameter discovery, presets, saved GUI state and render snapshots MUST retain all independent controls. Diagnostics MUST identify effective engine, authored-appearance status, source revision, execution precision, readout method and prepared fit/anchor metrics when available.

A render MUST retain its immutable bank through completion or cancellation. A canceled preparation MUST NOT publish partial data. Active preparation and sampling MUST observe a caller cancellation signal at bounded computation units.

Queued request replacement and stale-result rejection MUST NOT substitute for active cancellation. Full-input scanner setup MUST use the same effective grain population, seed and measurement parameters as its owning output snapshot.

PNG, JPEG, TIFF, EXR, float output, native Preview, Scan and Export MUST preserve their existing encoding, publication and metadata contracts. Package notices MUST retain the source attribution and Apache-2.0 license obligations. Any redistributed stock-derived data MUST retain its applicable data license and source provenance.

## Acceptance

Original-stock Rust preparation and corrected readout MUST be compared with executable pinned Fotufilm references separately from adapted current-profile appearance. Aligned executor evidence uses identical resolved curves, bank identity, seed, coordinates, and model-defined decisions; output statistics cannot replace those checks.

Population fit, anchor convergence, formation, pitch correction and final residual sampling MUST have measured evidence; formula-only probes MUST NOT certify those paths. Follow [GPU execution acceptance](../../gpu-execution/spec.md#acceptance) and [stochastic alignment](../../gpu-execution/design.md#stochastic-alignment).

Direct constant-transmittance texel integration MUST verify complete periodic boxes, seam placements and coarse footprints independently of summed-area-table arithmetic. Determinism, aligned crop identity, zero-strength bypass, material transitions, parameter invalidation and rejection MUST have regression coverage. Runtime and native GUI smoke MUST exercise the actual changed path.

Final positive photographic crops MUST assess bright/dark grain tails and chroma without treating density skew as positive-luminance proof. Controlled same-input renders MUST establish color/tone regression. Uncalibrated real scans MAY support appearance comparison only.

Physical stock calibration and exact joint spectral aperture integration require separate evidence and MUST NOT be inferred from source parity.
