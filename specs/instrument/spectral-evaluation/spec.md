# Spectral evaluation

`spektrafilm-lab spectral evaluate` reports the roundtrip error of the product path on measured spectra. It computes values and writes evidence. It MUST NOT fit parameters, render images, or judge a profile.

It is an experimental command: it carries no compatibility promise.

## Inputs

- `--spectra <PATH>` — the measured-spectra set. The file is a JSON manifest of the `stimuli` kind.
- The arrays live in `.npz`/`.npy` files beside it. The manifest declares the wavelength grid, the per-sample reflectance arrays, per-sample source and partition labels, provenance hashes, the valid band, and the illuminant pair.
- `--preset <NAME>` or `--profile <PATH>` — the chain to evaluate, with `--data-dir` and optional `--set` overrides from the process surface.
- `--out <DIR>` — the report destination. The command MUST NOT replace an existing report.

The illuminant pair is a property of the measured set, not of the invocation: it is declared in the manifest as the film reference illuminant and the scene illuminant that produced the input RGB. A second pair is a second document, and the command MUST NOT accept the pair as an option.

## Results

- The truth value integrates the measured spectrum, the illuminant, and the film sensitivity. It is normalized at the declared mid-gray anchor (0.184).
- The reconstruction runs the product path over the same spectrum and stops at the film raw exposure: XYZ, RGB, the reference-illuminant adaptation (CAT16), the hanatos2025 upsampling with the chain's resolved window, surface, and input-gamut settings, and the sensitivity integrals. The scene RGB is reconstructed; camera metering, exposure compensation, and optical scatter stay outside the evaluated boundary.
- It is normalized at the same anchor.
- Each sample reports its reflectance ρ, `max_abs_ev`, `common_ev`, `balance_rms_ev`, `ΔEV_R/G/B`, `rb_ev`, `gb_ev`, and a validity flag.
- A non-positive response is invalid. Samples outside the valid band are invalid.
- The evaluator MUST NOT clip, extrapolate, or repair them.
- The report carries both anchor roles, raw and relative. It MUST NOT judge which one is correct.
- Results group by the partitions the set declares, with ECDF, median, and P90 per group.

## Outputs

- `report.json` records the input hashes, the resolved chain, the tool version, and the per-group summaries.
- `samples.csv` carries one row per sample.
- A run writes no output unless the evaluation completes.

## Acceptance

- On a frozen measured-spectra set, the values MUST agree with the frozen Python evaluation of the calibration method within a relative tolerance of 1e-9.
- On a synthetic chart, film raw log-exposure values MUST cross-check against the `log_e_film` native observation retained by dedicated `experiment run` with optical scatter and camera auto-exposure off. The [native observation contract](../native-observations.md) governs those values and does not add a capture option to ordinary rendering.
- Repeated runs on identical inputs MUST produce identical values.
