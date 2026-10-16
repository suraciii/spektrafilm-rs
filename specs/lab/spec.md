# Lab CLI

The lab CLI (`spektrafilm-lab`) is the development command surface for profile authors and researchers. It supports profile development, calibration, and controlled experiments. Normal image editing and delivery do not need it.

The lab binary must run at f64 reference precision. It is built locally like the other workspace binaries. Release packages must not include it.

## Command surface

- `spektrafilm-lab fit` — calibration of a film profile from reference patch data. The [profile authoring contract](../profile-authoring/spec.md) owns the command.
- `spektrafilm-lab spectral evaluate` — roundtrip evaluation of the product path on measured spectra. Its contract is below.

The `spectral` group carries spectral diagnostics. `evaluate` is its first command.

## Spectral evaluation

`spectral evaluate` reports the roundtrip error of the product path on measured spectra. It computes values and writes evidence. It must not fit parameters, render images, or judge a profile.

### Inputs

- `--spectra <PATH>` — the measured-spectra set. The file is a JSON manifest. The arrays live in `.npz`/`.npy` files beside it. The manifest declares the wavelength grid, the per-sample reflectance arrays, per-sample source and partition labels, provenance hashes, and the valid band.
- `--preset <NAME>` or `--profile <PATH>` — the chain to evaluate, with `--data-dir` and optional `--set` overrides from the process surface.
- `--condition <REFERENCE>=<TARGET>` — the illuminant pair. A second pair is a second run.
- `--out <DIR>` — the report destination. The command must not replace an existing report.

### Results

- The truth value integrates the measured spectrum, the illuminant, and the film sensitivity. It is normalized at the declared mid-gray anchor (0.184).
- The reconstruction runs the product path over the same spectrum: XYZ, RGB, the reference-illuminant adaptation (CAT16), the hanatos2025 upsampling, the window, the surface, and the sensitivity integrals. It is normalized at the same anchor.
- Each sample reports its reflectance ρ, `max_abs_ev`, `common_ev`, `balance_rms_ev`, `ΔEV_R/G/B`, `rb_ev`, `gb_ev`, and a validity flag.
- A non-positive response is invalid. Samples outside the valid band are invalid. The evaluator must not clip, extrapolate, or repair them.
- The report carries both anchor roles, raw and relative. It must not judge which one is correct.
- Results group by the partitions the set declares, with ECDF, median, and P90 per group.

### Outputs

- `report.json` records the input hashes, the resolved chain, the tool version, and the per-group summaries.
- `samples.csv` carries one row per sample.
- A run writes no output unless the evaluation completes.

### Acceptance

- On a frozen measured-spectra set, the values must agree with the frozen Python evaluation of the calibration method within a relative tolerance of 1e-9.
- On a synthetic chart, the log-exposure values must cross-check against `process --stage-evidence` `log_e_film` output.
- Repeated runs on identical inputs must produce identical values.
