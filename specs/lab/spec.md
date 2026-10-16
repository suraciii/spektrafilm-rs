# Lab CLI

The lab CLI (`spektrafilm-lab`) is the development command surface for profile authors and researchers. It supports profile development, calibration, and controlled experiments. Normal image editing and delivery do not need it.

The lab binary must run at f64 reference precision. It is built locally like the other workspace binaries. Release packages must not include it.

## Command surface

- `spektrafilm-lab fit` — calibration of a film profile from reference patch data. The [profile authoring contract](../profile-authoring/spec.md) owns the command.
- `spektrafilm-lab spectral evaluate` — roundtrip evaluation of the product path on measured spectra. Its contract is below.
- `spektrafilm-lab measure run|sample|compare|verify` — measurement of the executed chain, of rendered charts, and of retained readings. Its contract is below.
- `spektrafilm-lab experiment run` and `spektrafilm-lab experiment export` — publish and convert a native stage-evidence bundle for a declared experiment. Their contract is [experiment.md](experiment.md).

The `spectral` group carries spectral diagnostics. `evaluate` is its first command. The `experiment` group carries the development loop for a profile. `run` is its first command.

The `measure` group carries development measurement. It reads declared sources and writes readings.

## Spectral evaluation

`spectral evaluate` reports the roundtrip error of the product path on measured spectra. It computes values and writes evidence. It must not fit parameters, render images, or judge a profile.

### Inputs

- `--spectra <PATH>` — the measured-spectra set. The file is a JSON manifest. The arrays live in `.npz`/`.npy` files beside it. The manifest declares the wavelength grid, the per-sample reflectance arrays, per-sample source and partition labels, provenance hashes, and the valid band.
- `--preset <NAME>` or `--profile <PATH>` — the chain to evaluate, with `--data-dir` and optional `--set` overrides from the process surface.
- `--condition <REFERENCE>=<TARGET>` — the illuminant pair. A second pair is a second run.
- `--out <DIR>` — the report destination. The command must not replace an existing report.

### Results

- The truth value integrates the measured spectrum, the illuminant, and the film sensitivity. It is normalized at the declared mid-gray anchor (0.184).
- The reconstruction runs the product path over the same spectrum and stops at the film raw exposure: XYZ, RGB, the reference-illuminant adaptation (CAT16), the hanatos2025 upsampling with the chain's resolved window, surface, and input-gamut settings, and the sensitivity integrals. The scene RGB is reconstructed; camera metering, exposure compensation, and optical scatter stay outside the evaluated boundary. It is normalized at the same anchor.
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
- On a synthetic chart, the film raw log-exposure values must cross-check against the `log_e_film` buffer of `process --stage-evidence` with the optical scatter stages and camera auto-exposure off.
- Repeated runs on identical inputs must produce identical values.

## Measurement

The `measure` group produces readings from declared sources. Its commands compute values and write artifacts. They must not fit parameters, render images, judge results, or generate pages.

### Stimulus boundaries

A measurement enters the chain at one declared boundary. The supported boundaries are `spectra`, `rgb_in`, `log_e_film`, and `cmy_film`. They reuse the existing tap vocabulary.

The entry must use the canonical `taps.inject` control, and the executed range must end at or after it. The command must not add a second chain-entry mechanism or a second execution path.

The measured range is declared through the same controls the user binaries read: `--enter` becomes the resolved `taps.inject`, and the collection boundary is the chain's own `taps.collect` when it reaches every requested reading, otherwise the last requested reading. A declared `taps.inject` that contradicts `--enter`, a `taps.collect` that stops before a requested reading, and a reading upstream of the entry must all fail before executing, each naming the conflict. The retained effective parameter document therefore describes the range that executed, and `measure verify` re-resolves the same range from the recorded declaration.

A stimulus that enters at `rgb_in` declares its color space and its transfer state, exactly as a raster input to `process` does. A film-domain stimulus declares neither, because its values are already film-domain numbers.

The `spectra` entry uses the same front half as [spectral evaluation](#spectral-evaluation). `spectral evaluate` remains the roundtrip-error report against truth. `measure run` reports readings and must not compute a truth value.

### Point-wise requirement

Readings are defined only for a point-wise chain: no operation in the measured range mixes samples, measures the whole frame, resizes it, or draws stochastic values.

An operation lies inside the measured range when it executes and can change a requested reading. Both are decided by the stage the operation first changes, which the lab must establish by measurement rather than by the flag's name:

| Operation | First stage it changes |
|---|---|
| `io.upscale_factor`, `io.crop` | `log_e_film` |
| `camera.auto_exposure`, `camera.diffusion_filter.active`, `camera.lens_blur_um` | `log_e_film` |
| `film_render.halation.active` | `log_e_film` |
| `film_render.dir_couplers.active` | `cmy_film` |
| `film_render.grain.active` | `cmy_film` |
| `enlarger.diffusion_filter.active` | `log_e_print` |
| `print_render.glare.active`, `scanner.lens_blur`, `scanner.unsharp_mask`, `scanner.auto_setup` | `rgb_out` |

An operation executes when the range passes through it: the entry boundary is where the stimulus enters, so an operation whose first changed stage is at or before the entry cannot reach a reading, and an operation whose first changed stage is after the last requested reading has not changed one yet. An operation is listed when both hold.

Which stages execute follows the executed path, not the route alone. The whole chain (the stimulus injected at `rgb_in` and collected at `rgb_out`) runs the route's own stage list: `input` returns after the crop and rescale, the convert routes build the film density by inverting the scan model and never run the metering or the development stages, the print stage needs a print route, `io.scan_film` scans the developed film directly instead, and the scanner runs on every route but `input`. A narrower range walks the tap topology from the entry to the collection boundary instead, and the nodes inside that range fire whatever the route says: only the metering stays skipped on the convert routes, and only `io.scan_film` still leaves the print taps out. That is why the route alone cannot decide eligibility: measured on the 12-sample chart, the whole chain with `--read rgb_out` leaves the collected buffer byte-identical while toggling `camera.auto_exposure`, `film_render.halation.active`, `film_render.dir_couplers.active`, `film_render.grain.active`, or `print_render.glare.active` on `input > convert-film > scan`, `input`, and the camera and development controls on `input > convert-film > print > scan`; and `input` with `--enter rgb_in --read log_e_film` moves every `log_e_film` reading by the metered stop (0.1618 in log10) when `camera.auto_exposure` is disabled, because that range runs the metering node.

An operation the executed path skips is not listed; neither is a controller whose active condition the core does not read (`print_render.glare.percent` at zero) nor one with no reader at all (`enlarger.lens_blur` is applied by no stage, see the parity matrix). The measured table is evidence, not a style choice: on a 12x4 chart, halation moves `log_e_film` by 1.9e-02 and the DIR couplers move `cmy_film` by 8.5e-01 while leaving `log_e_film` exact.

The command must resolve the chain before executing it. When one of these operations is active in the measured range, the command must fail before executing, and it must name the `--set` edits that disable it. The command must not disable an operation on the caller's behalf.

### `measure run`

```text
spektrafilm-lab measure run --stimuli PATH --enter BOUNDARY [--read STAGE,…] [--dry-run] [CHAIN OPTIONS] --out DIRECTORY
```

- `--stimuli` declares an array document: the sample arrays, the sample labels and groups, the declared domain or band, and provenance hashes.
- CHAIN OPTIONS are the parameter surface of `process`: `--preset`, `--film`, `--paper`, `--set`, `--route`, `--scan-output`, and `--data-dir`. The command must resolve them with the shared resolver. It must not accept a stored parameter document as the chain declaration.
- `--enter` declares the stimulus boundary.
- `--read` selects stages from the existing tap vocabulary: `rgb_in`, `log_e_film`, `cmy_film`, `log_e_print`, `cmy_print`, and `rgb_out`. Omitting `--read` must read every available stage.
- The command must execute the core-owned pipeline path of the resolved chain. It must not walk the stages beside an executed run or reconstruct pipeline mathematics.
- The command must not write image output.
- `--dry-run` must resolve the chain, evaluate the point-wise requirement, and write `measure.json` with the identity record only. It must not execute the chain, write `points.csv`, or publish a reading.

### `measure sample`

```text
spektrafilm-lab measure sample --source IMAGE [INPUT DECLARATION] --plan PATH --out DIRECTORY
spektrafilm-lab measure sample --from-evidence DIRECTORY --stage STAGE --plan PATH --out DIRECTORY
```

- `--source` declares a raster image. It must require the same input declaration as `process`: a source color space and a transfer state.
- `--from-evidence` reads one stage of a retained render-evidence bundle. The coordinate space, channel order, and units of that stage must come from the bundle manifest.
- `--plan` declares sample geometry with stable sample identities: a grid or explicit windows, each with a label, in the declared coordinate space.
- A sample window that leaves the source bounds must fail with the sample identity. The command must not clamp, pad, extrapolate, or repair a sample.
- The command must not resize, re-encode, or convert the source beyond the declared input declaration.
- A reading describes the values it stores: applying the declared transfer decoding makes the reading linear, and its record must say so while the source block keeps the source's own declaration. A stage that declares no transfer state records none.
- Every record carries the common fields, including `executed`: a sample never executes a chain.

### `measure compare`

```text
spektrafilm-lab measure compare --baseline DIRECTORY --candidate DIRECTORY [--reference PATH] --out DIRECTORY
```

- The command must match readings by composite sample and stage identity. Two sides that disagree on the domain, unit, or channel order of a shared key are an incompatible identity set: the command must fail and name the mismatch instead of truncating the join. Ambiguous identities, a repeated key inside one side, must fail as well. Keys that only one side carries are reported as unmatched with the side that carries them.
- It must compute per-sample deltas and group summaries through the shared metric implementation. It must not add a second color-difference, equal-lightness, or region-aggregation algorithm.
- Channel deltas are published only for pairs whose color space and transfer state agree, including two absent declarations. A pair whose declarations differ reports no channel deltas and an unavailable reason. The color-distance metric converts through linear sRGB into the bounded-display Lab domain, states that domain in its records, and marks a reading it had to bound.
- Availability is decided per pair: the baseline/candidate, candidate/reference, and baseline/reference pairs each decide from their own two sides and their own metric. A decision must never be borrowed from another pair.
- Each comparison carries its own name, so a record cannot be read as another comparison: the primary comparison is `candidate_minus_baseline`, and a reference adds `candidate_minus_reference` and `baseline_minus_reference`.
- The group labels of a reading (`source`, `partition`) are provenance, not identity: two sides that label one shared key differently must report the label difference as an identity difference and still compare the values. Only the value's own declaration (domain, units, channel order) makes a shared key incomparable.
- It must read the artifacts the measurement wrote, including their quoting: a sample identity with a comma, a quote, or a line break must survive the round trip.
- An identity difference between the two runs is evidence, not an error. The command must report the difference together with the numeric deltas. It must not refuse a comparison because the two chains differ; detecting that difference is the purpose of a retained reference reading.
- `--reference` adds a third retained reading. Its similarities and differences must be reported under its own role, not relabelled as the baseline or the candidate. A reference that shares no sample and stage with the pair, or whose readings are not eligible for the metric, must report an unavailable distance with a stable reason, and the comparison must still publish its other measurements. An unavailable measurement must never serialize as a distance, as zero, or as an absent row without a reason.

### `measure verify`

```text
spektrafilm-lab measure verify --run DIRECTORY
```

- The command must re-resolve the declaration recorded in the run and re-hash the renderer executable, the selected data inventory, and the profiles.
- It must report every difference from the recorded identity and re-hash every stage buffer the run retained, whether or not the reading requested that stage. It must not render, rewrite, or judge the run. It writes no artifact; it prints its report.

### Guard runs

A guard run is a retained measurement of a frozen stimulus set. It uses the same commands and artifacts as any other measurement. The lab must not add a guard command, flag, or tolerance field. A comparison against retained readings is a `measure compare`.

### Artifacts

- A measurement writes `measure.json` and `points.csv` into `--out`. It must not replace an existing artifact.
- `measure.json` must carry the declared inputs and their hashes, the resolved chain identity, the entry boundary, the read stages, the stimulus identity, and the group summaries.
- `points.csv` must carry one row per sample and stage: the sample identity, group, stage, channel order, values, unit, and validity.
- A run writes no artifact unless the measurement completes, and a failed run leaves nothing behind: the same destination must stay usable for a retry. An executed measurement retains the stage evidence of the run it read in `evidence/`, in the form the user binaries already write.
- A retained bundle is read with its own integrity: a buffer whose bytes no longer match the digest its manifest records must be refused, never attributed to the run.
- Every artifact is a round trip: the command must read back what it wrote, including the quoting of identities that carry separators.

### Identity

- A chain identity must carry the renderer's path, content hash, and version, the selected data inventory digest, the profile identities, the declared parameter sources and the content hash of every parameter file they name, the stock-specific application, the effective parameter document, the execution precision, and the seed. `measure verify` diffs that document leaf by leaf, so a re-verification names the parameter that moved instead of only its digest.
- A declaration must be replayable from any working directory: a declared parameter or preset file is recorded with its canonical path and content hash.
- Identity fields reuse the vocabulary the benchmark pins where the same fact exists (`renderer`, `data_dir`, the backend), rather than inventing a competing name. Facts the benchmark does not pin carry the lab's own names. Where the lab restates a shared convention — the interpolated percentile estimator, the bounded-display color distance — the specification of the shared owner is authoritative and the lab's record names it.

### Acceptance

- Repeated runs on identical inputs must produce identical artifacts.
- On the `log_e_film` stage, a reading must match the `log_e_film` buffer of `process --stage-evidence` within a relative tolerance of 1e-9, with the operations the guard names for that range off.
- Reading the injected boundary itself must need no disabled operation: every listed operation changes a later stage.
- On a default chain, `measure run` must fail before executing and name every active operation in the measured range with its remediation, and a `--read log_e_film` range must list only the operations that reach `log_e_film`.
- With a frozen chart image and a frozen plan, `measure sample` must reproduce the frozen values, and sampling a retained encoded stage must record a linear reading beside the source's own encoded declaration.
- A reference that is unmatched or ineligible must report an unavailable reason per pair, and the comparison must still publish its other measurements.
- A route that skips a boundary must not list that boundary's operations, and a range that executes them must not rely on the route: a chain measured through `input > convert-film > scan` with the camera and development controls left active must complete when it covers the whole chain, while the same chain on a route that runs them must fail and name them, and a narrower range on a skipping route (`input` with `--enter rgb_in --read log_e_film`, `input > convert-film > scan` with `--read cmy_film`) must fail and name the stages its tap topology fires.
- A comparison must publish each comparison's own rows and own availability: every `deltas.csv` row must name its comparison, a shared key whose group labels differ must still compare with the label difference reported, and a key whose units or channel order differ must fail.
- A run that fails after executing, and a sampling of a bundle whose buffer disagrees with its manifest, must leave no artifact behind.
