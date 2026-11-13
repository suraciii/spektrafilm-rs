# Grain

Use this page to select a grain engine and configure its controls. It also describes sampling behavior and execution paths. The [grain specification](../specs/film-grain/spec.md) defines the engine contract.

## Select an engine

`film_render.grain.engine` accepts `spektra`, `procedural`, `dye`, `fotufilm`, or `exposure`.
Use `procedural_*` for Procedural controls, `dye_strength` / `dye_scale` for Dye look controls, and `film_render.grain.exposure.*` for Exposure Grain controls.

Historical numbered control keys remain readable; saved documents and parameter descriptions emit only canonical names.
Supplying both spellings of one control in the same grain object is an error.
For GUI imports, an authored top-level control retains precedence over the runtime extension after each carrier is normalized.
Historical numbered engine selectors remain invalid.

Dye diagnostics use `SPEKTRAFILM_GRAIN_DYE_*`. Historical numbered diagnostic names remain readable when the canonical variable is absent; the canonical variable takes precedence when both are set.


- Spektra is the upstream experimental original.
- Procedural derives from Dehancer. It provides Analogue/Noise grain with twelve format/speed profiles. Its controls include Size, Amount, Shadows, Midtones, Highlights, Chroma, and Film Resolution. Pixel parity with Dehancer is not claimed.
- Dye is the default self-developed experimental film-coordinate dye field. `dye_strength` and `dye_scale` control it.
- Fotufilm Film is an adapted appearance population model fitted to resolved film curves. It is opt-in. `film_render.grain.fotufilm.*` controls it.
- Exposure Grain is a reconstructed post-scan encoded-RGB gradient-noise appearance model. It is opt-in. `film_render.grain.exposure.*` controls it.

Explicit engines in existing looks remain unchanged. To keep the historical engine in a sparse source, set `film_render.grain.engine = spektra`. See [default compatibility](../specs/film-grain/spec.md#defaults-and-compatibility) for omitted-engine and GUI state migration rules.

## Spektra

Select Spektra with `film_render.grain.engine: "spektra"`.

Spektra follows upstream experimental `apply_grain`, pinned to commit `28bf883e1672e884307edc75852549376e13644e`. It splits developed density through the profile's layer curves and samples a Poisson-binomial particle field for each active layer. A profile without layer curves uses its total curve as one layer of the same model.

The pinned experimental branch is a different grain model from the older 0.3.4 `main` branch: `main` derives particle populations from `particle_area_um2`, while experimental derives them from `rms_granularity` and adds multiplicative unsharp masking. Their stock values and rendered grain are not interchangeable, and a comparison must name the branch or commit.

`rms_granularity` controls particle-population sizing through the profile's peak summed variance at a 48 µm diameter aperture. It is not a direct particle area or a guarantee of final RGB granularity. Stock values come from `data/presets/grain_spektra.toml`. `particle_scale_sublayers`, `density_min`, `uniformity`, dye-cloud blur, lognormal micro-structure, and multiplicative unsharp masking retain their separate roles.

CLI sources and complete looks that select Spektra but omit RMS start with `[6, 8, 10]` before stock defaults apply. Explicit RMS values take precedence. GUI engine changes retain the existing shared material controls rather than reset their strength.

Library callers that construct runtime parameters directly must supply a positive RMS target when selecting Spektra. Shared typed defaults remain Dye defaults.

CPU Spektra uses the original sequential MT19937 regime: all Poisson draws for a plane, then all binomial draws, and micro-structure continues the final plane's MT state. GPU Spektra uses one PCG32 stream per pixel, channel and sublayer with the reference Poisson/binomial branches in f32; GPU micro-structure uses a PCG32 lognormal field. The GPU result is a different random realization, not a bit-identical MT19937 result, and there is no GPU sampling selector.

CPU and GPU use the same prepared model and precision boundaries. Grain working images remain f32 in both precision builds; curve preparation and reference-sensitive filtering use f64. Spatial filters use reflected FIR below 3 px and the reference sample-replicating IIR at or above 3 px; there is no large-sigma box approximation. GPU execution stays on the selected device and fails explicitly when the required precision or resources are unavailable. Preview disables active Spektra grain.

Remove the obsolete controls `particle_area_um2`, `particle_scale`, `particle_scale_layers`, `sublayers_active`, and `n_sub_layers` from old parameter files. They are rejected. Use `rms_granularity` for grain strength; layer availability comes from the profile, not a toggle. Dye retains its independent defaults and relative-strength interpretation.

## Procedural

Grain Procedural is selected with `film_render.grain.engine: "procedural"`.

It runs on every scan-output mode. It consumes the display-encoded RGB that the scanner produces, so `direct_scan` and `negative_to_positive` differ only in the image it receives, not in the grain law.

The composite is tone-neutral: it subtracts the displacement its zoned bases produce at the grain mean, so switching grain on does not shift the tone. Both executors take that mean from the same canvas cells. The canvas is a deterministic virtual table at a fixed 5200x3100 reference scale, so a frame whose canvas exceeds one storage binding is composed in row bands rather than refused; the band layout affects resources only, never the image.

### Profiles and controls

`procedural_profile` defaults to `"35mm250"`. Presets are:

- `8mm50`, `8mm250`, and `8mm500`.
- `16mm50`, `16mm250`, and `16mm500`.
- `35mm50`, `35mm250`, and `35mm500`.
- `65mm50`, `65mm250`, and `65mm500`.

Presets supply the grain configuration. Amount remains adjustable through `procedural_amount` (0–100).

Select `"custom"` to edit these controls:

- `procedural_mode`: `"analogue"` or `"noise"`.
- `procedural_size`: 1–48.
- `procedural_shadows`, `procedural_midtones`, `procedural_highlights`, `procedural_chroma`, and `procedural_resolution_factor`: all 0–100.

`procedural_film_type` retains `"negative"` or `"positive"` metadata. Rendering does not consume it, and the ordinary GUI does not expose it as a creative editor.

Custom values omitted or null inherit `35mm250`. Switching to Custom in the GUI copies the current preset and Amount. Selecting a preset restores its Amount. All numeric controls must be finite and within their stated ranges.

Film Resolution 100 preserves detail. Lower values increase resolution blur.

`procedural_resolution_type` selects Film Resolution for Analogue only. `0` selects OpticalResolution. `1` selects FastBlur. Noise uses the single-pass image-resolution path and ignores Film Resolution controls.

`procedural_resolution_type` and nonzero `procedural_timer` are explicit runtime controls. `procedural_timer` is a nonzero explicit phase override. Legacy `procedural_timer: 0.0` derives the phase from the recipe seed.

### Encoded RGB and sampling

Procedural consumes and returns native display-encoded RGB without an internal transfer curve or primaries conversion. This matches Dehancer's photo `by_pass` contract. The scanner encodes its generated RGB using the selected output color space before Grain. Encoded exports keep that result. Linear exports decode only the selected transfer afterward. Grain receives the same encoded image for both export choices. Spektra's density-domain sampling parameters remain unchanged.

Procedural restores the recovered RGBA content hash, sine permutation, gradient noise, texture sampling, and half-storage boundaries. Positive/Negative Film Type remains profile metadata. It is separate from `resolution_type`. For Analogue, `resolution_type` selects folded Gaussian FastBlur (`1`) or OpticalResolution (`0`). Noise uses the direct image-resolution path.

CPU/WGSL share portable trigonometry, host-resolved canvas dimensions and Analogue generator coefficients, and binary32 coordinate rounding before the clustered-noise hash. The rotation quotient also uses exact binary32 division for normal values within its supported exponent range. Random realizations can differ from vendor device arithmetic. Amount and tone controls use the reference effective-control polynomial. This includes its nonzero intercept at zero. Disable Grain with `active: false` for an exact bypass.

The unmasked photo path implements no overscan/damage-mask input or device-specific virtual-texture cap.

Noise is a single-pass image-resolution path. It does not consume Film Resolution output. Analogue additionally samples the original encoded source for its virtual grain texture. It composes over the Analogue Film Resolution result.

Noise uses the resolution-scaled sampling denominator `(1 + (Size - 1) / 47) * 2.4 * max(width / 1920, height / 1080)`. It uses half the effective Amount before composition. A recipe seed maps through MT19937 to a repeatable phase. Dehancer's global random stream is not a recipe seed contract.

## Dye

Grain Dye is selected with `film_render.grain.engine: "dye"`.

### Field controls

Dye's grain law is part of the engine. Two relative recipe controls tune it:

- **Grain strength** (`dye_strength`, 0..4) scales the correlated field amplitude and the event count together. `1` keeps the delivered look. `0` returns the target density with no random grain, including residual count noise. The density response and the per-record ratio do not change. It is not a certified optical RMS target.
- **Grain scale** (`dye_scale`, 0.5..4) scales the value-noise cell size in film coordinates. `1` keeps the delivered look. It is not a crystal diameter, scanner blur, or output-pixel size.

Omitting either key resolves to `1.0`, so recipes authored before these controls existed keep the delivered look.

The remaining law values — per-record relative amplitude, record coupling, value-noise octave weights, fog baseline, saturation density, and development taper — are engine constants. Recipes that carry `dye_support_um`, `dye_event_area_um2`, `rms_granularity`, `density_min`, `uniformity`, `particle_scale_sublayers`, or `channel_correlation` load and save unchanged; those values do not change the delivered Dye field.

The seed selects the realization. `random_seed`, `dye_strength`, and `dye_scale` are the grain inputs that change the image, and the same seed, inputs, geometry, and executor reproduce the same field.

Dye realizes a coordinate-addressed value-noise field on the film lattice and reads it out through the requested grid. A finer grid interpolates a lattice cell centre; a coarser grid integrates the cells it covers. Cropping, preview scaling, and stage splitting therefore sample one film rather than starting a second realization.

Expectation mode (`SPEKTRAFILM_GRAIN_DYE_EXPECTATION=1`) replaces every draw by its conditional mean and returns the target density. It is a diagnostic; it does not change the recipe and is not a production grain law.

### Black-and-white material

A resolved `channel_model=bw` filming profile uses one effective record: the upstream film chain resolves its target as one record in three equal lanes. This is not final-RGB desaturation or a calibrated silver-particle simulation. A BW profile may contain one or several development-time curves; negative-to-positive scanning accepts the single-time case without requiring three stored columns.

CPU material export retains the realized readout in the existing `.sfstage` representation. Continuation does not generate a second field. Material export and continuation remain CPU-only. See the [Dye design](../specs/film-grain/dye/design.md#density-law) for the law.

### Film geometry and routes

The field belongs to film coordinates. The pipeline develops the full frame. The readout produces the requested region at the requested grid. A crop selects the readout region and never restarts texture.

Full-frame and cropped CMY reads use one global field and are compared under the precision budget declared by each acceptance record. A rounded output grid may change footprints. Only that sampling is then comparable.

Dye supports still and cine color filming profiles and representable monochrome filming profiles, with negative or positive polarity. It uses the global `camera.film_format_mm` exposed-image long edge. This value must be finite and strictly positive. Its existing default is `35.0`. It is not a roll-width designation.

Format changes alter physical footprints and workload. Crop/resize determine output dimensions. Value-noise cell sizes and event area remain in film units. Device resource limits fail by name before allocation; the host realization covers the full developed source.

The diagnostic film-scan route `input > film > scan` with `io.scan_film = true` runs with `direct_scan` for both negative and positive film, and with `negative_to_positive` for negative film. Positive film retains its native polarity; Dye uses the profile's positive density-curve direction rather than selecting a separate grain model. Another chemistry or an unsupported route fails with an actionable unsupported-condition error.

When downstream spatial processing is active, Dye extends the output grid to cover the full film before those stages. It extracts the requested crop afterward. Physical optical controls use the actual output pitch. Scanner blur and unsharp masking remain pixel controls. See [film geometry](../specs/film-grain/dye/design.md#film-geometry) for the full-film, global-coordinate, and fractional-footprint rules.

### Execution

Dye resolves its field on the selected executor. The device evaluates the correlated field, density taper, and conditional count as bounded per-band passes on the developed full-source film density. Crop and upscale use the resolved readout grid. A crop that needs surrounding film context runs on the enclosing lattice and is extracted on the device after downstream spatial processing. The host reference evaluates the same law over the full developed source and resamples the requested readout.

A backend without a device field path uses the host reference and names that reason. An explicit GPU request fails by name instead of demoting a device-incapable request silently. Device resource limits also fail by name; they never change correlation length, event area, or the count law. The CLI/GUI report the effective executor. Preview, LUT, stochastic, and spatial deactivation bypass Dye.

Device carriers page source and output samples. Band and page boundaries affect resources only; they do not change film coordinates or the stochastic address.

The `SPEKTRAFILM_GRAIN_DYE_EXPECTATION=1` diagnostic replaces the material modulation and Poisson counts with their conditional means. It retains the field realization and readout, so it is not a grain-off render. It is also not the ensemble mean of final RGB: nonlinear optical conversion, negative interpretation, and encoding can shift the rendered population mean. Dye does not automatically match brightness or normalize RMS from the current image.

See [the Dye specification](../specs/film-grain/dye/spec.md) for experimental readouts and diagnostic controls.

### Cine limits

Cine profiles such as Kodak VISION3 500T can use Dye with their existing stock controls. This availability does not establish cine photographic calibration or optical-readout acceptance. Receiver, aperture, density convention, wavelength range, and viewing condition remain required before a cine optical claim.

### Standalone measurement compatibility

The independent `DyeOpticalOracle` measures declared fixed CMY realizations. Normal rendering does not invoke it. It rejects negative absolute total spectral density, non-finite absorption, and transmission underflow without clamping. Negative CMY coefficients remain valid when the combined total is nonnegative. Receiver density relative to a non-unit reference may also be negative.

Population `measure` now requires at least two realizations. A one-seed protocol still supports `read_realization`. Descriptive covariance and RMS keep divisor `N`; the mean-density SEM now uses `sqrt(C_pop[j,j] / (N - 1))`. For readings `[0, 2]`, the mean, RMS, and SEM are each `1 D`. SEM assumes independent realizations and does not bound systematic error or uncertainty in the RMS. Convert explicitly when comparing with manufacturer granularity in `1000 sigma_D`.

New records automatically carry `dye-measurement-v2:` before the caller's algorithm revision in the existing `protocol.algorithm_revision` field. Old JSON fields remain readable without changing their stored revision or uncertainty. The prior estimator was `rms_density / sqrt(N)`; old records must retain that interpretation according to their generating version. No measurement field, runtime parameter, preset value, field law, or random stream changes in this repair. See [the measurement contract](../specs/film-grain/dye/design.md#measurement-statistics-and-revision).


## Fotufilm Film

Select Fotufilm Film with `film_render.grain.engine: "fotufilm"`. This adapted appearance population model fits resolved film curves. It is opt-in and uses `film_render.grain.fotufilm.*` controls.

Fotufilm Film resolves on the selected executor. It prepares its fitted population before the first readout of a population identity. CPU rendering reuses the prepared host bank; device rendering forms and reads its bank without a host image intermediate.

The device implementation retains crystal deposits, dye-cloud sums, per-sublayer capacity accumulation, tile prefix sums, and footprint probe statistics in device buffers. Tile mean transmittance reconstructs integrated light; the separate probe mean density centers the residual. A single global bank serves every bounded output window, with source and output pages addressed in full-image coordinates. Only a four-byte execution-status word is read during execution; image samples return at explicit materialization. Full-path comparisons against the CPU bank cover three raster sizes and fractional geometry with nondefault look controls; source-page, output-page, and readout-window partitions preserve the sampled field.

Each Gaussian cloud term prepares its normalized binary32 taps once on the device, in the reference's ascending order. Both separable passes reuse those taps rather than regenerating the same exponential and normalization at every texel; the support, accumulation order, and four-term cloud law are unchanged.

## Exposure Grain

Select Exposure Grain with `film_render.grain.engine: "exposure"`. It is a post-scan appearance model reconstructed from Exposure X7's scalar luma grain, not a film-development or material-grain model. It consumes and returns display-encoded RGB; the film and paper profiles still determine everything upstream.

It runs after scan acquisition, output gamut, unsharp masking, and transfer encoding, before CMYK ink style and output mix. Direct scan and negative-to-positive outputs therefore differ only in the encoded image the model receives. Linear export encodes before Exposure, applies the grain once, then decodes the selected transfer; it does not rerun Exposure in linear RGB. Routes without a scan do not synthesize Exposure Grain.

Controls live in `film_render.grain.exposure`:

- `strength` (0–150, default 100) scales the appearance.
- `size` (0.25–4.5, default 1) sets the relative grain scale.
- `auto_size` (default `true`) derives the absolute grain size from `format` and the actual output dimensions. Setting it to `false` uses `absolute_size_px` (0.5–64, default 1.5).
- `format` chooses `35mm`, `medium_format`, or `large_format` for automatic sizing.
- `roughness`, `shadows`, `midtones`, and `highlights` (each 0–100) set the roughness S curve and zone partition.
- `push` (0–3, default 0) applies the reconstructed push response.
- `color_variation` (0–100, default 20) controls per-channel grain decorrelation; zero keeps the grain monochrome.

The shared `random_seed` drives the reconstruction; Exposure adds no second phase input. CPU and GPU use the reconstructed minstd permutation and gradient-noise tables, with deterministic repeatability at the same seed and declared dimensions. The GPU path runs the complete Exposure stage on the selected device and does not claim pixel parity with Exposure X7 or its presets. Color Variation is a reconstructed per-channel decorrelation control, not a vendor channel-parity claim.

## See also

- [Presets](presets.md)
- [Rendering pipeline](pipeline.md)
- [Backend and parity evidence](parity.md)
- [Attribution](attribution.md)
- [Grain specification](../specs/film-grain/spec.md)
- [Dye design](../specs/film-grain/dye/design.md)
- [Exposure design](../specs/film-grain/exposure/design.md)
