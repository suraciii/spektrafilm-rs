# Grain

Use this page to select a grain engine and configure its controls. It also describes sampling behavior and execution paths. The [grain specification](../specs/film-grain/spec.md) defines the engine contract.

## Select an engine

`film_render.grain.engine` accepts `spektra`, `procedural`, `dye`, or `fotufilm`. Replace explicit old selectors `v1`, `v2`, and `v3` with `spektra`, `procedural`, and `dye`, respectively. Old selectors are invalid. Existing `v2_*` and `v3_*` control keys, internal module paths, and diagnostic environment variable names remain unchanged.

- Spektra is the default upstream experimental original.
- Procedural derives from Dehancer. It provides Analogue/Noise grain with twelve format/speed profiles. Its controls include Size, Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution. Pixel parity with Dehancer is not claimed.
- Dye is a self-developed experimental film-coordinate dye field. It is opt-in. `v3_dye_support_um`, `v3_event_area_um2`, and `channel_correlation` control it.
- Fotufilm Film is an adapted appearance population model fitted to resolved film curves. It is opt-in. `film_render.grain.fotufilm.*` controls it.

## Spektra

Select Spektra with `film_render.grain.engine: "spektra"`.

Spektra grain dispatch mirrors upstream `apply_grain`. `sublayers_active` runs the layered model by default, matching 0.3.4. The model splits the composite density through `density_curves_layers` into per-sublayer densities. Each sublayer samples its own Poisson-binomial particle field. Sampling uses per-layer density maxima/fractions, particle scaling, dye-cloud blur, and lognormal micro-structure. `sublayers_active: false` keeps the single composite-density sampler.

`settings.use_fast_stats` switches the layered sampler to the `fast_stats` kernels like upstream. The micro-structure clumping field always uses them, as upstream does. This field uses a documented deterministic seed because Python draws it from numba's unseeded thread-global RNG.

Spektra resolves on the selected executor in both sampling regimes. `settings.use_fast_stats` selects the per-element `fast_stats` streams; the composite path always samples the sequential scipy MT19937 regime, and the layered path does so whenever that setting is off. The device sampler reproduces the reference draw order exactly — all Poisson draws across the plane first, then all binomial draws — so it never substitutes a different noise law. A film profile without resolvable sublayer density curves keeps the faithful per-stage path. Preview disables active Spektra grain. The separate Grain Procedural implementation remains available.

## Procedural

Grain Procedural is selected with `film_render.grain.engine: "procedural"`.

### Profiles and controls

`v2_profile` defaults to `"35mm250"`. Presets are:

- `8mm50`, `8mm250`, and `8mm500`.
- `16mm50`, `16mm250`, and `16mm500`.
- `35mm50`, `35mm250`, and `35mm500`.
- `65mm50`, `65mm250`, and `65mm500`.

Presets supply the grain configuration. Amount remains adjustable through `v2_amount` (0–100).

Select `"custom"` to edit these controls:

- `v2_film_type`: `"negative"` or `"positive"`.
- `v2_mode`: `"analogue"` or `"noise"`.
- `v2_size`: 1–48.
- `v2_shadows`, `v2_midtones`, `v2_highlights`, `v2_chroma`, and `v2_resolution_factor`: all 0–100.

Custom values omitted or null inherit `35mm250`. Switching to Custom in the GUI copies the current preset and Amount. Selecting a preset restores its Amount. All numeric controls must be finite and within their stated ranges.

Film Resolution 100 preserves detail. Lower values increase resolution blur.

`v2_resolution_type` selects Film Resolution for Analogue only. `0` selects OpticalResolution. `1` selects FastBlur. Noise uses the single-pass image-resolution path and ignores Film Resolution controls.

`v2_resolution_type` and nonzero `v2_timer` are explicit runtime controls. `v2_timer` is a nonzero explicit phase override. Legacy `v2_timer: 0.0` derives the phase from the recipe seed.

### Encoded RGB and sampling

Procedural consumes and returns native display-encoded RGB without an internal transfer curve or primaries conversion. This matches Dehancer's photo `by_pass` contract. The scanner encodes its generated RGB using the selected output color space before Grain. Encoded exports keep that result. Linear exports decode only the selected transfer afterward. Grain receives the same encoded image for both export choices. Spektra's density-domain sampling and defaults remain unchanged.

Procedural restores the recovered RGBA content hash, sine permutation, gradient noise, texture sampling, and half-storage boundaries. Positive/Negative Film Type remains profile metadata. It is separate from `resolution_type`. For Analogue, `resolution_type` selects folded Gaussian FastBlur (`1`) or OpticalResolution (`0`). Noise uses the direct image-resolution path.

CPU/WGSL share portable trigonometry. Random realizations can differ from vendor device arithmetic. Amount and tone controls use the reference effective-control polynomial. This includes its nonzero intercept at zero. Disable Grain with `active: false` for an exact bypass.

The unmasked photo path implements no overscan/damage-mask input or device-specific virtual-texture cap.

Noise is a single-pass image-resolution path. It does not consume Film Resolution output. Analogue additionally samples the original encoded source for its virtual grain texture. It composes over the Analogue Film Resolution result.

Noise uses the resolution-scaled sampling denominator `(1 + (Size - 1) / 47) * 2.4 * max(width / 1920, height / 1080)`. It uses half the effective Amount before composition. A recipe seed maps through MT19937 to a repeatable phase. Dehancer's global random stream is not a recipe seed contract.

## Dye

Grain Dye is selected with `film_render.grain.engine: "dye"`.

### Field controls

Dye conditions a mixed film-coordinate dye field on `v3_dye_support_um`. This control gives the Gaussian sigma in micrometers. Its default is `3.5`. The field-cell pitch is `support / 2`.

Dye realizes film-space Poisson counts with event area `v3_event_area_um2 × k_c × particle_scale_sublayers`. `v3_event_area_um2` uses square micrometers and defaults to `1.28`. Dye uses the resolved `density_min` and `uniformity`. It allocates each channel across the profile's three numerical curve components; these components are not identified physical emulsion layers. Both `v3_dye_support_um` and `v3_event_area_um2` must be finite and strictly positive. Dye component scales must also be positive. They change statistical event mass and count rate, not separate component kernel widths. The components share Dye's spatial support.

A profile may give an individual component zero capacity. That component contributes no events. Each record still needs positive total capacity; an all-zero record is rejected by the model.

The channel scale is `k_c = (rms_granularity_c / 6)²`. Here, `6` is the preset of the reference stock `kodak_portra_800`. The existing `rms_granularity` field is a relative Dye strength scale, not a certified optical RMS target. Entering `8` does not request a measured stock granularity of `8`. Zero or a missing stock value retains the existing fallback; use `active: false` to disable grain. Dye does not read Spektra's `particle_area_um2` or `particle_scale`.

Some preset channel ratios come from measured outputs of a commercial film simulator. They remain appearance references, not measurements of film material. The reference stock and its event area define the existing relative scale; they do not establish an absolute material calibration.

`channel_correlation` defaults to `0.0` and accepts `[0, 0.45]`. This is the historical supported and acceptance range, not an identified universal material limit. The public constant `MEASURED_CHANNEL_CORRELATION_CEILING` retains its historical name for compatibility. The parameter sets the shared Poisson-event rate fraction among active records within each numerical component. Each record retains its marginal Poisson count distribution. For positive-rate records `a` and `b` in one component, count correlation is `rho × min_c(lambda_c) / sqrt(lambda_a × lambda_b)`, which is at most `rho`. This is not a bound on final RGB correlation or a guarantee of identical measured variance in a finite realization. Zero retains independent record counts. Processed scans do not, by themselves, separate material correlation from receiver or scanner effects.

### Black-and-white material

A resolved `channel_model=bw` filming profile selects one effective material record. Dye uses record 0's `rms_granularity`, `density_min`, and `uniformity`. `particle_scale_sublayers` still supplies three numerical-component scales, not three record controls. One sampled density is carried in three equal lanes before formation and readout. This is not final-RGB desaturation or a calibrated silver-particle simulation.

Both CPU and GPU validate the original BW samples across the full source, including samples outside the requested crop and its formation halo. Invalid samples there still reject the render. A BW profile may contain one or several development-time curves; negative-to-positive scanning accepts the single-time case without requiring three stored columns.

`channel_correlation` has no effect for this layout. Its requested value remains stored. CLI edit feedback and the GUI explain the inactive control; selecting a color profile restores its applicability. Dry-run `film_grain_dye` reports `record_layout`, `calibration_status`, and requested/effective correlation without claiming a checked compute device.

Resetting Grain restores its editable controls to factory values, but keeps the profile-derived record layout. This derived layout does not mark the section as modified.

The bundled initial BW profiles are `kodak_doublex` (5222 negative, with selectable development times) and `kodak_trix` (7266 reversal, not still-film 400TX). Existing controls and presets remain uncalibrated for this effective model. A profile's monochrome identity does not identify silver versus chromogenic chemistry. The BW printing profile `kodak_2302` is not a filming stock.

CPU material export retains the formed field in the existing `.sfstage` representation. Continuation does not generate a second field. Material export and continuation remain CPU-only; the existing exact-cell and aperture persistence restrictions still apply. See the [record-layout design](../specs/film-grain/v3/design.md#record-layout) for validation and ordering.


### Scalar-record reference

Set `SPEKTRAFILM_GRAIN_RECORD_Q` to a finite positive record contribution in `D·µm²` to select the scalar fixed-contribution experiment. Select active Dye, a BW filming stock, `--backend cpu` or `--backend gpu`, `--route 'input > film > scan'`, and `--scan-output direct_scan`. Without this variable, Dye retains its existing capacity law. There is no calibrated default q; `0.2` and `0.6` are experimental values, not stock presets.

The new law uses one scalar image-forming density target. It does not use `v3_event_area_um2`, `rms_granularity`, `particle_scale_sublayers`, `uniformity`, `density_min`, or `channel_correlation`. It retains `v3_dye_support_um`, the seed, and the explicit cell-pitch and expectation diagnostics. A record contribution is not a grain area. See the [scalar-record design](../specs/film-grain/v3/design.md#scalar-fixed-contribution-law) for the count law and memory limits.

This route automatically enables exact-cell optical response: form density, convert each cell with the scanner's linear spectral response, then integrate the output footprint. It rejects scanner/enlarger LUTs, incompatible spatial finishing, and aperture or legacy mass-law experiments. `SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL` must be absent or `1`. An invalid q, cell pitch, or expectation flag fails instead of falling back.

Actual `--diagnostics` reports retain the effective law, q, support, cell pitch, full seed, expectation flag, and optical order in `configuration.render.grain.record`. Preserve the input, profile data, command, binary identity, and report to reproduce a comparison. The q environment variable is not saved in a recipe. Color, print, material export, and GUI controls are not part of this experiment.

CPU execution reuses scan preparation and bounded tile scratch. A proven scalar spectral basis avoids repeated wavelength integration; wavelength-dependent bases and unsafe exponent ranges retain generic integration. These optimizations do not change q, support, cell pitch, or optical ordering.

GPU execution forms the record directly on the selected device. It uses one density lane until the RGB optical response, reuses bounded band buffers, and does not upload a CPU-generated field or download intermediate bands. The certified f32 count-law rate limit is `65536`; unsupported representation fails explicitly rather than changing the model.

CPU and GPU share the conditional law but not individual stochastic draws under the Dye executor contract. Use expectation-mode comparisons and independent count/formation checks for numerical alignment, plus per-executor seed and crop repeatability. A software Vulkan adapter is portability evidence, not hardware speed evidence.

Compare the legacy law with and without `SPEKTRAFILM_GRAIN_V3_CELL_SPECTRAL=1` to isolate optical ordering. Compare that exact-cell legacy result with the record law to isolate the event law. Keep all other settings and physical geometry fixed. Encoded-RGB differences do not establish optical-density calibration or physical stock fidelity.

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
