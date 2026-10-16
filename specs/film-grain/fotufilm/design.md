# Fotufilm Film grain design

## Boundary

The model owns exposure-class fitting, four speed bins, fog/development adaptation, marked-crystal formation, local dye capacity, bank generation, aperture matching, pitch calibration and residual sampling. Core owns resolved profile adaptation, immutable population lifetime, requested film geometry, pipeline dispatch and observation. Existing print and scan stages retain spectral dye/base projection, optical effects, negative interpretation and output encoding.

A population is an effective distribution fitted to a characteristic curve. A bin is a fitted speed grouping, not one of Spektra's supplied three emulsion layers. A bank is a canonical stationary texture at 17 density levels, with one shared crystal realization across levels. A placement is a seed-derived periodic offset and orientation of that bank. A footprint is a complete film-space rectangle read through a placement. These terms must not imply a uniquely reconstructed physical emulsion.

Rust remains the shipped host/runtime stack, with CPU and WGPU executors selected by [GPU backend selection](../../gpu-execution/spec.md#backend-selection). Swift and Halide may supply external reference evidence but must not become implicit shipped fallbacks. Failure to prepare the selected model must remain an error.

## Density adaptation

Core must use the actual loaded and runtime-resolved film curve, including current chemistry and development-time resolution. The adapter must normalize each record by its own curve minimum. The adapted grain-local D-min must be zero; spectral `base_density` must remain separate and unchanged. Source-oracle cases may retain original gross density and D-min explicitly.

Original loaded reference curves and resolved current curves must remain distinct for development fitting. A gamma edit changes exposure sampling and must be represented consistently in the grain input curve coordinate. DIR changes the per-pixel density target; it must not rerun local formation chemistry or redefine stationary coating geometry. Grain-only endpoint lookup must preserve the target outside the fitted interval as specified by the product contract.

The population fit must use nonnegative exposure-class weights, source hit-threshold responses, authored four-bin density shares and refitted bin weights. Material/fog/development semantics must follow the executable source. A monochrome profile carries one emulsion curve and no per-record columns, so the adapter must hand that single curve to every record: the model's monochrome path reads record 1 for levels and residual and mirrors its anchor measurements across records, while zero curves in the other records fail curve validation and fit nothing. Silver remains the automatic material law for any monochrome stock, including positive ones such as the loaded `kodak_trix` profile whose stored `type` is `positive`. A fit that cannot represent the adopted curve within measured admission bounds must fail with its record/error. Admission bounds must be based on source/candidate evidence and may not conceal a failing port by changing the image's tone curve.

## Formation and calibration

The reference model's counter/hash streams must fix coating count, crystal position, activation draw and mass independently of output dimensions. The canonical bank must use the pinned fixed formation seed. Crystal mass marks, deposition, blur terms, capacity saturation and supersampling order must match the source behavior before optimization. CPU and GPU must preserve these streams and model-defined discrete choices under [stochastic alignment](../../gpu-execution/design.md#stochastic-alignment).

Each dye bin must sum cloud demand before applying capacity saturation. Density must sum through film depth; transmitted light must average over area. The bank must store centered transmittance running sums and per-level mean light. Construction must retain sufficient precision to avoid cancellation-induced noise at large box areas.

The 48-micrometer matching loop must expose target, measured result, pass count and final mismatch. The source's finite covariance-window aperture calculation is a model approximation. Independent circular-aperture integration and covariance-tail convergence are required for a physical calibration claim. A matching-loop exit without convergence must not certify the requested appearance parameters.

Pitch calibration must measure per-level mean density and adjacent-level correlation using the actual complete readout geometry. Interpolation must subtract those means and normalize retained variance. Color mixing must use the source's variance-compensated own-record/shared-mean rule after independent record residuals are sampled. Monochrome must retain one record realization across output channels.

## Complete periodic footprints

For the source's 128-micrometer block, 64-micrometer margin and 16-micrometer seam, a footprint up to 96 micrometers per axis is geometrically complete for every positive-weight placement. Upstream clips larger footprints. This implementation must not copy that truncation.

For every chosen placement, extend its bank periodically to the entire requested rectangle. A periodic primitive must decompose a coordinate into signed whole periods and its nonnegative remainder. Whole-period, row, column and residual contributions must integrate the complete rectangle with four primitive evaluations, independent of footprint area. The box's denominator must be the requested area, not retained block coverage.

Block selection, seed offsets, eight orientations and seam weighting must remain the source rule. Complete integration over each chosen placement extends the source approximation, rather than constructing one globally coherent crystal field across every block. Results within the unclipped domain must match the source. Larger footprints must match an independent direct periodic-transmittance integral. Variance normalization and pitch means must be measured for this same extension, not copied from the clipped sampler.

Size must scale bank-coordinate pitch and footprint and compensate amplitude by its reciprocal. Softness must scale only the footprint. Both axes must retain exact rounded physical pitches. Amount and record layer amounts must scale only sampled residuals.

## Geometry and runtime

The film long-edge convention, source crop rounding and output dimension rounding remain the existing contracts. The fourth engine must develop the full source before requested output resampling. Core must conservatively area-resample the developed source density targets to the requested output lattice; subject variation is a declared piecewise-constant reconstruction, not finer source information. Grain-free bypass must retain existing ordering exactly.

A crop with downstream spatial optics must extend its output lattice to enclose the full film before those optics and crop afterward. Parent geometry calculations must be shared only when both film-coordinate engines consume the same invariant; Dye formation law and cell readout remain separate. Reference comparisons must cover non-square rounding and global origin shifts.

Preparation must be lazy. A population/bank identity must include all numerical curve/metadata/reference and algorithm inputs, but not Amount, Size, Softness, Color, Layers, seed, crop or display dimensions. Prepared pitch tables must use bank identity, both footprint axes and readout method. Stage configuration and scanner setup must track their own effective measurement inputs. A rendering snapshot must retain its immutable bank even after cache eviction. A bounded cache must not publish canceled partial preparation. A cache hit does not establish GPU preparation; full-GPU execution must perform numerical population/bank preparation and sampling on the device under [device residency](../../gpu-execution/design.md#device-residency).

Cancellation must reach fitting, anchor work, bank formation, pitch preparation and sampling with bounded polling units. The existing render worker may keep its queue and revision filtering. Film preparation must not require an unrelated job framework. A stock/metadata change or shutdown may cancel obsolete bank work; an Amount-only edit must not discard a reusable completed bank.

## Precision and evidence

Source Float preparation, source production arithmetic, and Rust reference arithmetic must have separate measured comparison budgets. CPU/GPU alignment evidence uses identical resolved curves, bank identity, seed, coordinates, and model-defined discrete decisions plus scoped floating agreement under [numerical alignment](../../gpu-execution/design.md#numerical-alignment) and [stochastic alignment](../../gpu-execution/design.md#stochastic-alignment). Full source input, resolved curves, parameter identity, source revision, compiler/toolchain, seed, physical pitch, disabled effects and output encoding must accompany verification.

A synthetic sampler probe establishes arithmetic only. A real stock oracle must establish preparation, real pitch correction and residual behavior. A real CLI/native GUI run must establish product wiring, publication and state. Output regression, source parity, adapted appearance and physical calibration must remain separate acceptance claims.
