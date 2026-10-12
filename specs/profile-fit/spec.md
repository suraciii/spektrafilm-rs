# Profile fitting

## Contract

**Profile fitting** derives parameter values from reference patch data instead of from hand measurement. A **scene patch table** is the ordered list of linear RGB triples that the film saw. A **reference patch table** is the ordered list of linear RGB triples measured from the developed film of the same chart. A **chart layout** defines the patch grid and the patch index sets that a fit consumes.

Profile fitting is a calibration mechanism for profile authors. It is not a GUI feature. It must not modify a preset library, a profile file, or any other input in place; every result is written to the paths the caller names.

The CLI exposes fitting as `spektrafilm-f64 fit`. The command must exist in the f64 reference binary and must not exist in the f32 preview binary.

## Inputs

The command reads three data files:

- `--scene <PATH>`: the scene patch table. Values are linear RGB in the pipeline input color space of the configured chain.
- `--reference <PATH>`: the reference patch table. Values are linear RGB with sRGB primaries, decoded from the encoding of the source measurement.
- `--layout <PATH>`: the chart layout. It carries the grid geometry `rows`, `cols`, `patch_px`, `gap_px`, `box_px`, and three patch index ranges: `color`, `gray`, and `fit_gray`. The gap separates the patches; the sample box is centered in each patch and must not touch the gap or a neighbor.

A patch index range is a half-open `[start, end)` pair. The color range selects the chromatic patches. The gray range selects the gray row. The fit-gray range selects the gray patches that the fit scores, which may exclude the brightest and darkest steps of that row.

The chain configuration reuses the process command surface: `--preset`, `--data-dir`, `--route`, and `--set`. The scene table and the chain must agree on the input color space. The comparison interprets the chain output as linear sRGB, so the configured route and scanner output must produce the finished positive scan.

A fit must fail with an actionable error when the two patch tables differ in length, when the patch count differs from `rows * cols`, when an index range falls outside the patch count, or when a table value is not finite.

## Fit targets

`--fit <GROUPS>` selects the parameter groups to fit. Groups are comma-separated and applied together in one optimization.

- `surface`: the `surface_terms` leading non-constant terms of every layer of the film profile's `hanatos2025_adaptation_surface_params`, that is `layers * terms` values. The fit replaces the whole surface: the fitted terms carry the fitted values and every other term is zero. The fit must not use the profile's stored surface values as a starting surface.
- `tone`: `scanner.positive_density_ev` and the three `scanner.positive_balance_ev` values. `scanner.positive_contrast` and `scanner.positive_dmax` keep the values of the chain configuration.

A common offset added to `positive_density_ev` and subtracted from every `positive_balance_ev` channel leaves the positive scan unchanged, so the four values are redundant by one degree of freedom. The fit must pin `positive_balance_ev[1]` to its resolved value and search only the other three, and the report must record the pinned channel and its value. Without the pin, identical data would fit equivalent but different tones.

The default is `surface`. `--surface-terms <N>` sets the term count of the surface group and defaults to 9.

Fitting the `surface` group must enable `settings.apply_hanatos2025_adaptation_surface`, because a fitted surface that the chain does not apply cannot be scored. The command must not change any other `settings` value.

## Cross validation

`--cv` measures whether a fit learned the stock or the chart. The fit must then run one extra fit per fold, in addition to the fit over every patch:

- **fold 1** trains on the even positions of the color range plus the fit-gray range, and is measured on the odd positions.
- **fold 2** trains on the odd positions plus the fit-gray range, and is measured on the even positions.

A fold fit uses the same groups, bounds, `--surface-terms`, `--lambda`, `--max-iters`, and starting vector as the main fit. Every fold reports the median of its training color patches, the median of its held-out color patches, and the median of the unfitted configuration on those held-out patches. The folds are diagnostic: the written profile and overlay must come from the fit over every patch, and `--cv` must not change them.

The command must fail when `--cv` is set and the color range holds fewer than four patches. The report records the folds and the held-out patch indices.

## Fit invariants

The command must disable halation and grain for every evaluation. Halation leaks light between neighboring patches and grain is stochastic; both would corrupt a patch comparison. The command must render the linear domain (`io.output_cctf_encoding` false), because the reference patch table is linear sRGB. The command must report these forced settings in the report.

## Objective

For every patch of the color range and the fit-gray range, the residual is the pair difference of the chain output patch and the reference patch in CIE L\*a\*b\* coordinates, computed from linear sRGB. The objective adds a ridge term `sqrt(lambda) * value` for every fitted surface coefficient; tone coefficients carry no ridge term.

The fitted target is deterministic: identical inputs, preset, and options must produce identical fitted values.

## Outputs

- `--report <PATH>`: the fit report as JSON. It is required.
- `--out-profile <PATH>`: a copy of the resolved film profile with the fitted surface. Required when the fit includes the `surface` group.
- `--out-overlay <PATH>`: a sparse parameter file with the fitted tone leaves. Required when the fit includes the `tone` group.

The report must carry the fitted values, the metrics below, every input path with its SHA-256, the resolved preset and route, the forced settings, and the tool version.

## Metrics

Both metrics are median CIEDE2000 values between the chain output patches and the reference patches in CIE L\*a\*b\* coordinates.

- `color`: over the color range.
- `gray`: over the fit-gray range.

The report must carry the value of both metrics before the fit and after the fit, and the per-patch difference for every patch.

## Verification

A profile produced by the fit must be loadable by the profile loader, and its surface must change the rendered result of a chart under an enabled surface toggle. The fitted values must reduce both metrics against the unfitted configuration on the same data. A cross-validated fit must reduce the held-out median of every fold against the unfitted configuration on the same held-out patches.

## Out of scope

Fitting grain, halation, measured spectra, and print profiles is outside this contract. Holdout frames are outside this contract: a scene comparison needs the scene preparation and the per-frame timing fit that the calibration workspace owns, and the workspace harness already measures it.
