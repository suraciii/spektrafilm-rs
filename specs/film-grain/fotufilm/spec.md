# Fotufilm Film grain

## Identity and scope

`fotufilm` must select the independent Fotufilm Film grain engine. The native GUI must display `Fotufilm Film`. Spektra must remain the default engine. Selecting Fotufilm must not change the selected film or paper profile.

The engine must adapt the Film mechanism from Fotufilm revision `59d43471dbef45c85ae442e507a291fdebfb2ec5`. The image's current resolved spectral film profile must remain the authority for mean developed color and tone. The engine must add a grain residual after film development and DIR and before film scan or print exposure. It must not add encoded RGB noise or replace the characteristic curve with its fitted population mean.

The supported configuration is an appearance model with explicitly authored effective grain parameters. Defaults must not imply stock-specific measured calibration or equality with Fotufilm's full renderer. The model must support color negative, dye reversal, silver monochrome and explicitly selected chromogenic monochrome material laws on valid resolved film curves. It must reject invalid, nonfinite, empty or unsupported curve inputs with an actionable error. Curve-fit and final anchor convergence metrics must remain observable; convergence failure must not silently select a different noise model.

Film, print-film and film-scan routes must execute the model once when the film-development boundary is traversed. Collecting `cmy_film` must include that execution. Injecting already developed `cmy_film` or a later tap must not execute it again. Passthrough, print-paper and convert-film must preserve their existing boundaries and must not synthesize new grain without film development.

## Parameters

`film_render.grain.fotufilm` must contain the independent engine's controls. Controls belonging to other engines must not change this model. All scalar and array coefficients below must be finite.

`amount` must range from 0 to 4 and default to 1. `size` and `softness` must range from 0.5 to 4 and default to 1. `color` must range from 0 to 1 and default to 1. `layers` must contain three per-record amount multipliers from 0 to 4 and default to `[1, 1, 1]`.

`rms_density` must be strictly positive and at most 0.1 density units, with default 0.009. It is an effective model matching target through the model's 48-micrometer aperture approximation, not a published figure for the current film. `clump_radius_um` must be strictly positive and at most 32 micrometers, with default 5. It describes the fitted fastest-bin scale, not an individual crystal radius. `layer_weights` must contain three strictly positive coefficients at most 4 and default to `[0.7, 1, 1.35]`. `layer_size_ratio` must contain three coefficients from 0.25 to 4 and default to `[1, 1, 1]`.

`radius_span` must range from 1 to 20 and default to 6. `sublayer_shares` must contain four strictly positive fitted-density allocation shares summing to 1 within absolute tolerance 0.000001. Its default must be `[0.25, 0.25, 0.25, 0.25]`. The shares must not be interpreted as crystal-count or silver-mass fractions. `coating_density_scale` must range from 0.25 to 4 and default to 1. `fog_density` must range from 0 to 0.3 density units and default to 0.03.

`material` must accept `auto`, `silver`, `dye_negative` and `dye_reversal` and default to `auto`. Auto must select silver for a monochrome film, dye reversal for a positive color film, and dye negative for a negative color film; a monochrome stock is a silver image whether its profile is negative or positive. Explicit material selection must allow chromogenic monochrome to use a dye law without changing the profile's channel model.

Film population parameters must be independent of runtime Amount, Size, Softness, Color, Layers, seed and raster size. Changes to effective population parameters or resolved reference/development curves must invalidate prepared population data. No stock-name heuristic may substitute another film's grain metadata.

## Image and backend behavior

`active = false`, effective `amount = 0`, and runtime stochastic-effect deactivation must be exact existing-path bypasses. They must prepare no grain bank. LUT generation must deactivate grain and prepare no bank.

The selected executor must implement Fotufilm preparation and sampling with the same model on CPU and GPU. CPU f64 remains the numerical reference; GPU must preserve source/reference conditioning for formation, fitted population, crystal marks, and residual sampling, including exact discrete decisions where the model defines them. Backend selection, residency, and failure behavior follow [GPU execution](../../gpu-execution/spec.md#backend-selection) and [algorithm alignment](../../gpu-execution/spec.md#algorithm-alignment). GPU execution must not use CPU-prepared stochastic population or bank data, CPU grain arithmetic, a Gaussian or other generic stochastic substitute, or another engine. Floating outputs need not be bit-identical between f32 and f64 builds.

The full-source film extent must determine physical pitch. Cropping must retain absolute film coordinates. Actual rounded output dimensions must determine both output pitches. Downstream spatial processing must receive a lattice covering the full film before the requested crop is extracted. Preview must evaluate the same model on its declared reduced-resolution source; it must not claim pixel equality with full Scan or Export.

Every requested finite positive footprint must be read in full through the chosen periodic tile placement. The model must not silently truncate the footprint at a block margin. The complete-periodic readout extension must be identified as an approximation distinct from the upstream large-footprint clipped sampler. Rectangular footprints must use their actual axis sizes. Geometry and resource limits must be checked before allocation with an actionable error.

Grain coordinates outside the population's density interval must use the nearest endpoint's grain statistics while preserving the unmodified developed CMY target. This is an explicit appearance approximation, not extrapolated stock calibration. Numerical or image density must not be clipped merely to satisfy the grain lookup.

## State, cancellation and delivery

CLI parameter discovery, presets, saved GUI state and render snapshots must retain all independent controls. Diagnostics must identify effective engine, authored-appearance status, source revision, execution precision, readout method and prepared fit/anchor metrics when available.

A render must retain its immutable bank through completion or cancellation. A canceled preparation must not publish partial data. Active preparation and sampling must observe a caller cancellation signal at bounded computation units. Queued request replacement and stale-result rejection must not substitute for active cancellation. Full-input scanner setup must use the same effective grain population, seed and measurement parameters as its owning output snapshot.

PNG, JPEG, TIFF, EXR, float output, native Preview, Scan and Export must preserve their existing encoding, publication and metadata contracts. Package notices must retain the source attribution and Apache-2.0 license obligations. Any redistributed stock-derived data must retain its applicable data license and source provenance.

## Acceptance

Original-stock Rust preparation and corrected readout must be compared with executable pinned Fotufilm references separately from adapted current-profile appearance. This upstream/source parity evidence is separate from CPU/GPU alignment: aligned executor evidence uses identical resolved curves, bank identity, seed, coordinates, and model-defined decisions, and output statistics cannot replace those checks. Population fit, anchor convergence, formation, pitch correction and final residual sampling must have measured evidence; formula-only probes must not certify those paths. Follow [GPU execution acceptance](../../gpu-execution/spec.md#acceptance) and [stochastic alignment](../../gpu-execution/design.md#stochastic-alignment).

Direct constant-transmittance texel integration must verify complete periodic boxes, seam placements and coarse footprints independently of summed-area-table arithmetic. Determinism, aligned crop identity, zero-strength bypass, material transitions, parameter invalidation and rejection must have regression coverage. Runtime and native GUI smoke must exercise the actual changed path.

Final positive photographic crops must assess bright/dark grain tails and chroma without treating density skew as positive-luminance proof. Controlled same-input renders must establish color/tone regression. Uncalibrated real scans may support appearance comparison only. Physical stock calibration and exact joint spectral aperture integration require separate evidence and must not be inferred from source parity.
