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

Both Spektra composite and layered grain use the CPU sampler. No resident WGPU Spektra grain shader is maintained. Active Spektra grain rendering selects the faithful per-stage path. Preview disables active Spektra grain. The separate Grain Procedural implementation remains available.

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

The unmasked photo path implements no overscan/damage-mask input or device-specific virtual-texture cap. See [grain research notes](dehancer-grain-reverse-engineering.md) for measured evidence and coverage limits.

Noise is a single-pass image-resolution path. It does not consume Film Resolution output. Analogue additionally samples the original encoded source for its virtual grain texture. It composes over the Analogue Film Resolution result.

Noise uses the resolution-scaled sampling denominator `(1 + (Size - 1) / 47) * 2.4 * max(width / 1920, height / 1080)`. It uses half the effective Amount before composition. A recipe seed maps through MT19937 to a repeatable phase. Dehancer's global random stream is not a recipe seed contract.

## Dye

Grain Dye is selected with `film_render.grain.engine: "dye"`.

### Field controls

Dye conditions a mixed film-coordinate dye field on `v3_dye_support_um`. This control gives the Gaussian sigma in micrometers. Its default is `3.5`. The field-cell pitch is `support / 2`.

Dye realizes film-space Poisson counts with event area `v3_event_area_um2 × k_c × particle_scale_sublayers`. `v3_event_area_um2` uses square micrometers and defaults to `1.28`. Dye uses the resolved `density_min` and `uniformity`. It forms every channel from the film's three sublayers. Both `v3_dye_support_um` and `v3_event_area_um2` must be finite and strictly positive. Dye sublayer scales must also be positive.

The channel scale is `k_c = (rms_granularity_c / 6)²`. Here, `6` is the preset of the reference stock `kodak_portra_800`. This scale is the only per-channel strength input. Dye does not read Spektra's `particle_area_um2` or `particle_scale`. Their blue-heavy default would double a coarser blue record.

`channel_correlation` defaults to `0.0`. It accepts `[0, 0.45]`: the structural domain is `[0, 1)`, but an enabled value must stay inside the measured inter-record coherence band, so validation rejects anything above `0.45`. It is the fraction of one emulsion layer's per-channel event rate that the layer's active channels share; the shared part correlates those channels while every channel keeps its own marginal count rate, so strength, support, and readout do not move. The configured value is an upper bound on realized correlation, because a layer whose channels have unequal rates shares only the smallest of them. Zero leaves the independent per-channel realization exactly as it was. Correlation of this kind is off by default: the available measurements do not separate the film's own record correlation from the scanner's, and a lab scan's strong coherence belongs to the scanner.

### Film geometry and routes

The field belongs to film coordinates. The pipeline develops the full frame. The readout produces the requested region at the requested grid. A crop selects the readout region and never restarts texture.

Overlapping full-frame and cropped CMY reads agree within the design's **1e-5** (f32) and **1e-10** (f64) tolerances. A rounded output grid may change footprints. Only that sampling is then comparable.

Dye supports still and cine color-negative filming profiles. It uses the global `camera.film_format_mm` exposed-image long edge. This value must be finite and strictly positive. Its existing default is `35.0`. It is not a roll-width designation.

The reference validation remains a 36 mm long edge on `input > film > print > scan` with `io.scan_film = false`. Format changes alter physical footprints and workload. Crop/resize determine output dimensions. Support and event area remain in film units. Large requests can fail an explicit resource bound.

The diagnostic film-scan route `input > film > scan` with `io.scan_film = true` also runs for `negative_to_positive` and `direct_scan`. Another chemistry or an unsupported route fails with an actionable unsupported-condition error.

When downstream spatial processing is active, Dye extends the output grid to cover the full film before those stages. It extracts the requested crop afterward. Physical optical controls use the actual output pitch. Scanner blur and unsharp masking remain pixel controls. See [film geometry](../specs/film-grain/v3/design.md#film-geometry) for rounded and anisotropic output limits.

### Execution

Dye resolves its field on the selected executor. On the resident route the device evaluates the count law, formation, and readout as bounded per-band passes on the developed film density; a crop or upscale the resident raster cannot express keeps the per-stage route and still reads the field from the device. A backend without a device field path, the experimental `SPEKTRAFILM_GRAIN_V3_MASS_LAW` selector, and the gated exact-cell spectral readout resolve on the host and name that reason. An explicit GPU request fails by name instead of demoting a device-incapable request silently. The CLI/GUI report the effective executor. Preview, LUT, stochastic, and spatial deactivation bypass Dye.

See [the Dye specification](../specs/film-grain/v3/spec.md) for experimental readouts and diagnostic controls.

### Cine limits

Cine profiles such as Kodak VISION3 500T can use Dye with their existing stock controls. This availability does not establish cine photographic calibration. The current density-average readout exceeds the spectral aperture gate for the measured 500T-to-2383 condition. See [known optical limits](../specs/film-grain/v3/design.md#evidence-and-limits).

## Fotufilm Film

Select Fotufilm Film with `film_render.grain.engine: "fotufilm"`. This adapted appearance population model fits resolved film curves. It is opt-in and uses `film_render.grain.fotufilm.*` controls.

Fotufilm Film resolves on the CPU. It prepares its population before the first readout of a given population identity. It reuses the prepared bank for later renders of that identity.

## See also

- [Presets](presets.md)
- [Rendering pipeline](pipeline.md)
- [Backend and parity evidence](parity.md)
- [Attribution](attribution.md)
- [Grain specification](../specs/film-grain/spec.md)
- [Dye design](../specs/film-grain/v3/design.md)
