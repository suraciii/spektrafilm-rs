# Profile authoring design

## Boundaries

Profile fitting lives in `spektrafilm-core::fit` and is gated on the `precision-f64` feature. The CLI subcommand lives in the lab binary and contains argument parsing, file loading, and file writing only.

The module owns: the chart layout and patch tables, chart image assembly and patch sampling, the objective, the optimizer, the metrics, the report, and the forward evaluation over the calibrated pipeline.

The module reuses, and must not duplicate:

- `runtime::Runtime` and the calibrated `pipeline` for the forward evaluation.
- `presets` for preset resolution into `RuntimeParams`.
- `profile` loading and the public `Profile` fields for the surface values.
- `lut_qa::delta_e_2000` for CIEDE2000 and `lut_qa::xyz_to_lab` for the final L\*a\*b\* conversion; `spektrafilm_math` color-space matrices for the sRGB-to-XYZ step.

## Ownership

The fitted surface belongs to the film profile; the fitted tone belongs to the look. The command writes a profile copy and a sparse parameter overlay. It must not write into a preset document, because a preset library edit is a user action with its own rules.

## Forward evaluation

One evaluation renders the assembled chart image through a calibrated runtime and samples one mean linear RGB triple per patch.

- The chart image is assembled from the scene patch table with the layout geometry. Patches are separated by the layout gap so that no stage can bleed between them.
- A runtime is constructed per candidate parameter vector, because a surface change invalidates the spectral calibration. The runtime is built in memory from the resolved film and print profiles; no temporary data directory is written.
- The evaluation runs on the CPU backend at the crate scalar precision, which is f64 in this feature.
- The evaluation renders the linear domain: it clears `io.output_cctf_encoding`, because the reference patch table is linear sRGB. The negative-conversion stage would otherwise apply the destination transfer curve inside the same buffer.
- Sampling reads a centered box per patch and must not include gap pixels.

## Optimizer

The optimizer is a bounded least-squares solver over the residual vector of the objective.

- Finite-difference Jacobian, forward differences, step `0.05` for surface coefficients.
- Trust-region behavior over a bounded parameter box: surface coefficients in `[-40, 40]`.
- At most 15 accepted parameter vectors; the iteration count is the number of accepted vectors. The command must report the iteration and evaluation counts.
- Finite differences are independent and may run in parallel on the rayon pool. Parallelism must not change the result.

## Cross validation

`--cv` answers whether the fit learned the stock or the chart, which is the question the calibration workspace answered with its ablation table.

- The folds are the even and odd positions of the color range; the fit-gray range joins every training set, and the held-out set is never scored by its own fold's objective.
- A fold clones the evaluator and rescoring is the only difference: the chain data, the chart image, the starting vector, the bounds, and the optimizer stay identical, so a fold result is comparable with the other fold.
- The main fit still scores every patch, and the folds do not feed back into the written profile or overlay.

The report records per fold the training median, the held-out median, and the held-out median of the unfitted configuration, so a reader can compare the fit against not-fitting at all on the same patches.

## Determinism

The chain must be deterministic for a fixed candidate vector: halation and grain are off, all other stages are deterministic, and the CPU backend is used. The optimizer state is a pure function of the objective. Therefore identical inputs produce identical fitted values, and the report may record exact values.

## Failure handling

- Input parsing and validation failures carry the file path and the failing field.
- A film profile with no `hanatos2025_adaptation_surface_params` fails the `surface` group, because the group's fitted values have nowhere to be written. A profile that carries the field is fittable whether or not the look currently applies the surface.
- A chain configuration whose output is not the converted negative image fails validation once, before the first evaluation, reusing the existing route and scan-output validation.
- The command writes no output file unless the fit completes.

## Provenance

The report records every input file with its SHA-256, the resolved preset name, the route, the run's tool version, the forced settings, the used term count, lambda, and the evaluation count. A profile produced by a fit is an experimental profile until the repository's profile review accepts it; the report is the evidence for that review.
