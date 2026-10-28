# Grain

Use this page to select a grain engine and configure its controls. It also describes sampling behavior and execution paths. The [grain specification](../specs/film-grain/spec.md) defines the engine contract.

## Select an engine

`film_render.grain.engine` accepts `spektra`, `procedural`, `dye`, or `fotufilm`. Replace explicit old selectors `v1`, `v2`, and `v3` with `spektra`, `procedural`, and `dye`, respectively. Old selectors are invalid. Existing `v2_*` and `v3_*` control keys, internal module paths, and diagnostic environment variable names remain unchanged.

- Spektra is the upstream experimental original.
- Procedural derives from Dehancer. It provides Analogue/Noise grain with twelve format/speed profiles. Its controls include Size, Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution. Pixel parity with Dehancer is not claimed.
- Dye is the default self-developed experimental film-coordinate dye field. `v3_dye_support_um`, `v3_event_area_um2`, and `channel_correlation` control it.
- Fotufilm Film is an adapted appearance population model fitted to resolved film curves. It is opt-in. `film_render.grain.fotufilm.*` controls it.

Explicit engines in existing looks remain unchanged. To keep the historical engine in a sparse source, set `film_render.grain.engine = spektra`. See [default compatibility](../specs/film-grain/spec.md#defaults-and-compatibility) for omitted-engine and GUI state migration rules.

## Spektra

Select Spektra with `film_render.grain.engine: "spektra"`.

Spektra follows upstream experimental `apply_grain`, pinned to commit `28bf883e1672e884307edc75852549376e13644e`. It splits developed density through the profile's layer curves and samples a Poisson-binomial particle field for each active layer. A profile without layer curves uses its total curve as one layer of the same model.

`rms_granularity` controls particle-population sizing through the profile's peak summed variance at a 48 µm diameter aperture. It is not a direct particle area or a guarantee of final RGB granularity. Stock values come from `data/presets/grain_spektra.toml`. `particle_scale_sublayers`, `density_min`, `uniformity`, dye-cloud blur, lognormal micro-structure, and multiplicative unsharp masking retain their separate roles.

CLI sources and complete looks that select Spektra but omit RMS start with `[6, 8, 10]` before stock defaults apply. Explicit RMS values take precedence. GUI engine changes retain the existing shared material controls rather than reset their strength.

Library callers that construct runtime parameters directly must supply a positive RMS target when selecting Spektra. Shared typed defaults remain Dye defaults.

`settings.use_fast_stats: false` uses the sequential NumPy/scipy sampling regime: all Poisson draws for a plane, then all binomial draws. Micro-structure continues the final plane's NumPy state. Fast-stats uses the reference distribution branches with repeatable per-element streams; it does not reproduce numba's schedule-dependent random realizations.

CPU and GPU use the same prepared model and precision boundaries. Grain working images remain f32 in both precision builds; curve preparation and reference-sensitive filtering use f64. GPU execution stays on the selected device and fails explicitly when the required precision or resources are unavailable. Preview disables active Spektra grain.

Remove the obsolete controls `particle_area_um2`, `particle_scale`, `particle_scale_layers`, `sublayers_active`, and `n_sub_layers` from old parameter files. They are rejected. Use `rms_granularity` for grain strength; layer availability comes from the profile, not a toggle. Dye retains its independent defaults and relative-strength interpretation.

## Procedural

Grain Procedural is selected with `film_render.grain.engine: "procedural"`.

It runs on every scan-output mode. It consumes the display-encoded RGB that the scanner produces, so `direct_scan` and `negative_to_positive` differ only in the image it receives, not in the grain law.

### Profiles and controls

`v2_profile` defaults to `"35mm250"`. Presets are:

- `8mm50`, `8mm250`, and `8mm500`.
- `16mm50`, `16mm250`, and `16mm500`.
- `35mm50`, `35mm250`, and `35mm500`.
- `65mm50`, `65mm250`, and `65mm500`.

Presets supply the grain configuration. Amount remains adjustable through `v2_amount` (0–100).

Select `"custom"` to edit these controls:

- `v2_mode`: `"analogue"` or `"noise"`.
- `v2_size`: 1–48.
- `v2_shadows`, `v2_midtones`, `v2_highlights`, `v2_chroma`, and `v2_resolution_factor`: all 0–100.

`v2_film_type` retains `"negative"` or `"positive"` metadata. Rendering does not consume it, and the ordinary GUI does not expose it as a creative editor.

Custom values omitted or null inherit `35mm250`. Switching to Custom in the GUI copies the current preset and Amount. Selecting a preset restores its Amount. All numeric controls must be finite and within their stated ranges.

Film Resolution 100 preserves detail. Lower values increase resolution blur.

`v2_resolution_type` selects Film Resolution for Analogue only. `0` selects OpticalResolution. `1` selects FastBlur. Noise uses the single-pass image-resolution path and ignores Film Resolution controls.

`v2_resolution_type` and nonzero `v2_timer` are explicit runtime controls. `v2_timer` is a nonzero explicit phase override. Legacy `v2_timer: 0.0` derives the phase from the recipe seed.

### Encoded RGB and sampling

Procedural consumes and returns native display-encoded RGB without an internal transfer curve or primaries conversion. This matches Dehancer's photo `by_pass` contract. The scanner encodes its generated RGB using the selected output color space before Grain. Encoded exports keep that result. Linear exports decode only the selected transfer afterward. Grain receives the same encoded image for both export choices. Spektra's density-domain sampling parameters remain unchanged.

Procedural restores the recovered RGBA content hash, sine permutation, gradient noise, texture sampling, and half-storage boundaries. Positive/Negative Film Type remains profile metadata. It is separate from `resolution_type`. For Analogue, `resolution_type` selects folded Gaussian FastBlur (`1`) or OpticalResolution (`0`). Noise uses the direct image-resolution path.

CPU/WGSL share portable trigonometry. Random realizations can differ from vendor device arithmetic. Amount and tone controls use the reference effective-control polynomial. This includes its nonzero intercept at zero. Disable Grain with `active: false` for an exact bypass.

The unmasked photo path implements no overscan/damage-mask input or device-specific virtual-texture cap.

Noise is a single-pass image-resolution path. It does not consume Film Resolution output. Analogue additionally samples the original encoded source for its virtual grain texture. It composes over the Analogue Film Resolution result.

Noise uses the resolution-scaled sampling denominator `(1 + (Size - 1) / 47) * 2.4 * max(width / 1920, height / 1080)`. It uses half the effective Amount before composition. A recipe seed maps through MT19937 to a repeatable phase. Dehancer's global random stream is not a recipe seed contract.

## Dye

Grain Dye is selected with `film_render.grain.engine: "dye"`.

### Field controls

Dye's grain law is part of the engine: the relative amplitude, the three correlation lengths, the band limit, the event area, the fog baseline, and the development taper are constants, not controls. Recipes that carry `v3_dye_support_um`, `v3_event_area_um2`, `rms_granularity`, `density_min`, `uniformity`, or `channel_correlation` still load; those values do not change the field.

The seed selects the realization. `random_seed` is the only grain input that changes the image, and the same seed, geometry, and executor reproduce the same field.

Dye realizes a band-limited correlated field on the film lattice and reads it out through the requested grid. A finer grid interpolates a lattice cell centre; a coarser grid integrates the cells it covers. Cropping, preview scaling, and stage splitting therefore sample one film rather than starting a second realization.

Expectation mode (`SPEKTRAFILM_GRAIN_V3_EXPECTATION=1`) replaces every draw by its conditional mean and returns the target density. It is a diagnostic; it does not change the recipe and is not a production grain law.

### Black-and-white material

A resolved `channel_model=bw` filming profile uses one effective record: the realized density is carried in three equal lanes. This is not final-RGB desaturation or a calibrated silver-particle simulation.

Both CPU and GPU validate the original BW samples across the full source, including samples outside the requested crop. Invalid samples there still reject the render. A BW profile may contain one or several development-time curves; negative-to-positive scanning accepts the single-time case without requiring three stored columns.

CPU material export retains the formed field in the existing `.sfstage` representation. Continuation does not generate a second field. Material export and continuation remain CPU-only. See the [Dye design](../specs/film-grain/v3/design.md#density-law) for the law.

### Film geometry and routes

The field belongs to film coordinates. The pipeline develops the full frame. The readout produces the requested region at the requested grid. A crop selects the readout region and never restarts texture.

Full-frame and cropped CMY reads use one global field and are compared under the precision budget declared by each acceptance record. A rounded output grid may change footprints. Only that sampling is then comparable.

Dye supports still and cine color filming profiles and representable monochrome filming profiles, with negative or positive polarity. It uses the global `camera.film_format_mm` exposed-image long edge. This value must be finite and strictly positive. Its existing default is `35.0`. It is not a roll-width designation.

Format changes alter physical footprints and workload. Crop/resize determine output dimensions. Support and event area remain in film units. Large requests can fail an explicit resource bound.

The diagnostic film-scan route `input > film > scan` with `io.scan_film = true` runs with `direct_scan` for both negative and positive film, and with `negative_to_positive` for negative film. Positive film retains its native polarity; Dye uses the profile's positive density-curve direction rather than selecting a separate grain model. Another chemistry or an unsupported route fails with an actionable unsupported-condition error.

When downstream spatial processing is active, Dye extends the output grid to cover the full film before those stages. It extracts the requested crop afterward. Physical optical controls use the actual output pitch. Scanner blur and unsharp masking remain pixel controls. See [film geometry](../specs/film-grain/v3/design.md#film-geometry) for the full-film, global-coordinate, and fractional-footprint rules.

### Execution

Dye resolves its field on the selected executor. On the resident route the device evaluates the count law, formation, and readout as bounded per-band passes on the developed full-source film density. Crop and upscale use the resolved readout grid. A crop that needs surrounding film context runs on the enclosing lattice and is extracted on the device after downstream spatial processing. The gated exact-cell spectral response runs between cell formation and fractional readout. It subtracts `density_min` before the response and averages linear exposure or linear scan RGB. The aperture probe runs print exposure on a fine grid and collapses `log10(mean(10^log_exposure))` before paper development. The existing route and optical restrictions of both probes still apply.

A backend without a device field path and the experimental `SPEKTRAFILM_GRAIN_V3_MASS_LAW` selector use the host reference and name that reason. Any selected GPU request fails by name instead of demoting a device-incapable request silently; host-only laws require selecting CPU. Device resource limits also fail by name; they never change cell pitch, support, or the count law. The CLI/GUI report the effective executor. Preview, LUT, stochastic, and spatial deactivation bypass Dye.

Device carriers page full-source and output samples. Dye copies each source window on the device and partitions output rectangles along both axes when binding limits require it. An oversized individual footprint is split into formed-cell rectangles with complete formation halos. Their responses accumulate on the device with the original fractional weights; density readout subtracts its base only once after the final piece. Page and band boundaries do not change film coordinates or the stochastic address.

The CPU reference uses the same partition rule: reduce output rectangles along both axes, then divide an oversized footprint into formed-cell pieces with full halos. Each piece uses the original global count addresses and overlap weights. Optical response remains per formed cell before integration, and partial readouts stay in `f64` until the final output conversion.

When both optical probes are enabled, the exact-cell response reads the fine aperture grid. Linear-light collapse restores the requested raster before paper development. This also corrects the previous combined-probe CPU path, which skipped aperture collapse and delivered the fine raster.

The `SPEKTRAFILM_GRAIN_V3_EXPECTATION=1` diagnostic replaces Poisson counts with their conditional means. It retains field formation and readout, so it is not a grain-off render. It is also not the ensemble mean of final RGB: nonlinear optical conversion, negative interpretation, and encoding can shift the rendered population mean. Dye does not automatically match brightness or normalize RMS from the current image.

See [the Dye specification](../specs/film-grain/v3/spec.md) for experimental readouts and diagnostic controls.

### Cine limits

Cine profiles such as Kodak VISION3 500T can use Dye with their existing stock controls. This availability does not establish cine photographic calibration or optical-readout acceptance. Receiver, aperture, density convention, wavelength range, and viewing condition remain required before a cine optical claim.

### Standalone measurement compatibility

The independent `DyeOpticalOracle` measures declared fixed CMY realizations. Normal rendering does not invoke it. It rejects negative absolute total spectral density, non-finite absorption, and transmission underflow without clamping. Negative CMY coefficients remain valid when the combined total is nonnegative. Receiver density relative to a non-unit reference may also be negative.

Population `measure` now requires at least two realizations. A one-seed protocol still supports `read_realization`. Descriptive covariance and RMS keep divisor `N`; the mean-density SEM now uses `sqrt(C_pop[j,j] / (N - 1))`. For readings `[0, 2]`, the mean, RMS, and SEM are each `1 D`. SEM assumes independent realizations and does not bound systematic error or uncertainty in the RMS. Convert explicitly when comparing with manufacturer granularity in `1000 sigma_D`.

New records automatically carry `dye-measurement-v2:` before the caller's algorithm revision in the existing `protocol.algorithm_revision` field. Old JSON fields remain readable without changing their stored revision or uncertainty. The prior estimator was `rms_density / sqrt(N)`; old records must retain that interpretation according to their generating version. No measurement field, runtime parameter, preset value, formation law, or random stream changes in this repair. See [the measurement contract](../specs/film-grain/v3/design.md#measurement-statistics-and-revision).


## Fotufilm Film

Select Fotufilm Film with `film_render.grain.engine: "fotufilm"`. This adapted appearance population model fits resolved film curves. It is opt-in and uses `film_render.grain.fotufilm.*` controls.

Fotufilm Film resolves on the selected executor. It prepares its fitted population before the first readout of a population identity. CPU rendering reuses the prepared host bank; device rendering forms and reads its bank without a host image intermediate.

The device implementation retains crystal deposits, dye-cloud sums, per-sublayer capacity accumulation, tile prefix sums, and footprint probe statistics in device buffers. Tile mean transmittance reconstructs integrated light; the separate probe mean density centers the residual. A single global bank serves every bounded output window, with source and output pages addressed in full-image coordinates. Only a four-byte execution-status word is read during execution; image samples return at explicit materialization. Full-path comparisons against the CPU bank cover three raster sizes and fractional geometry with nondefault look controls; source-page, output-page, and readout-window partitions preserve the sampled field.

Each Gaussian cloud term prepares its normalized binary32 taps once on the device, in the reference's ascending order. Both separable passes reuse those taps rather than regenerating the same exponential and normalization at every texel; the support, accumulation order, and four-term cloud law are unchanged.

## See also

- [Presets](presets.md)
- [Rendering pipeline](pipeline.md)
- [Backend and parity evidence](parity.md)
- [Attribution](attribution.md)
- [Grain specification](../specs/film-grain/spec.md)
- [Dye design](../specs/film-grain/v3/design.md)
