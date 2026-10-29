# Film chain design

- Scanner vocabulary follows [Scanner language](spec.md#scanner-language). The canonical scan output mode is `negative_to_positive`; scan output validation owns its accepted values.
- CLI, presets, recipes, GUI state, and diagnostics consume that vocabulary without aliases. The UI separates display labels from serialized values.
- The [lab scan design](lab-scan/design.md) owns frame setup, style naming, point-wise rejection, and shared CPU/resident acquisition. The model version identifies the numerical cutover; legacy style aliases do not preserve fixed-curve pixels.
- The scan model retains the existing runtime field names, document schema versions, defaults, calibration, processing order, and numerical operations for the controls it keeps. Telemetry uses the scan output name for its stage and execution reasons.
- The readout illuminant participates in the prepared scan identity and its cache key.
- `core::runtime` is the production entry point. `Pipeline` owns stage topology and state.
- Pipeline entry points share optical diagnostic tap admission and selector-conflict rules. Each entry point retains its validation order and topology-specific guards.
- Preparation and parameter-update ownership follows [Prepared runtime state](#prepared-runtime-state).
- `ComputeBackend` separates stage orchestration from CPU and WGPU execution. Both MUST implement the [target optical order](spec.md#scan-optics-and-print-viewing-glare) and [GPU numerical alignment](../gpu-execution/design.md#numerical-alignment).
- WGPU owns validated device representations, resources, and dispatch.
- Scan preparation resolves one colorimetric sensor: the CIE 1931 color-matching functions over the production wavelength grid. No receiver profile is selectable, and no receiver calibration matrix is applied.
- The scan matrix is the identity.
- The readout illuminant is a product constant selected by the material category. It resolves through the shared illuminant service that `enlarger.illuminant` uses, so one service owns every light-source name.
- Its spectral power distribution is prepared once per render and participates in the prepared scan identity and its cache key.
- Scan acquisition is one spectral integration of the scanned medium under the readout illuminant, taken against the clear film base of that medium as the white reference. The acquisition chromatically adapts from that clear-film white to the output color space's white and emits output-space linear RGB.
- Bulk scanning and scalar forward calibration share log-XYZ LUT construction and density-coordinate scaling. Each path retains its own sample precision, interpolation, correction, and color-transform order.
- Negative conversion has one preparation-time interpretation. It reads record densities, as a narrowband scanner does, and derives one private per-record response from the selected profile density curves: monotone density-to-log-exposure knots with finite endpoint slopes.
- One record's reconstructed exposure MUST NOT depend on another record's density, and image pixels reuse the response without any spectral capture.
- The density response uses least-squares isotonic regression on the profile density samples to regularize reversals. Pooled plateaus use their mean log exposure.
- The forward film curves remain unchanged.
- Every exposure-reconstruction path reads the profile density curves in the domain the film stage emits: each record measured over its own minimum. Calibration rows and record axes use that domain, so a record's curve minimum is never read as exposure.
- The scanned medium determines the stage topology and the scanner calibration references.
- Exposure reconstruction MUST NOT estimate its response from image content.
- Scan provenance serializes the neutral scan model identity: the CIE 1931 sensor, the clear-film white reference, the output space white, the identity scan matrix, and the resolved readout illuminant.
- CPU and GPU execute one photographic stage contract under [GPU execution](../gpu-execution/design.md#ownership). A resident chain is an execution strategy, not a second model or permission for CPU image-stage fallback.
- Runtime owns complete-operation capability validation, preparation, stage order, and output finalization. Known GPU limitations MUST be checked before dispatch; an explicit GPU request MUST fail rather than use a hidden CPU stage.
- The WGPU backend owns device-resident stage execution and resource lifetimes. Multiple submissions or bounded tiles MUST preserve the same domains and do not require a second public pipeline.
- Filming metering owns the downsample, decode, luminance measurement, and exposure calculation. The pipeline meters the full input before crop and resize.
- Private Jzazbz and CAM16 modules own their appearance-model formulas and viewing adaptation. Output compression retains destination policy and the shared table cache.
- The WGPU blur module owns standalone Gaussian blur execution and shared blur limits remain backend policy. Resident stage-specific blur dispatch remains with its owning stage.
- Scanner preparation MUST place the optical kernel at the linear readout boundary before any inverse. Constant linear sensor matrices MAY commute with the shared achromatic blur only with equivalent boundary handling and measured precision.
- Nonlinear transforms MAY not commute with blur. A destination-encoded or gamut-processed buffer is not an optical input.
- Print viewing glare MUST be added after optical blur in that linear readout boundary. A fixed linear color transform MAY carry its illuminant-colored offset into another linear coordinate system; nonlinear correction MUST remain downstream.
- Its `print_render` storage and preset ownership do not make it print development.
- Resident dispatch MUST include optical order in capability decisions and prepared identity. Diagnostic final-RGB paths MUST share the order; material-density taps MUST remain before scanner optics.
- The historical late-blur shortcut MUST be removed, not preserved under identity points or a backend-specific branch.
- Working pixel pitch is carried with the image through crop and resize and is consumed by spatial stages; stages do not infer it from the current raster size.
- Color-domain transitions are explicit. The chain does not apply an implicit transfer curve or primaries conversion inside an effect.
- Stage preparation resolves the capture illuminant once per render from the selected film profile's `reference_illuminant` and the explicit `camera.capture_illuminant` leaf, under the [capture light boundary](spec.md#capture-light-and-scan-light). It reuses the shared illuminant selection and validator that the enlarger illuminant already uses, so one service owns every light-source name.
- A resolved capture illuminant participates in the prepared render identity and its cache key; an unset leaf does not.
- No generic graph or backend-specific public parameter schema is required; runtime parameters remain the single caller-facing model.
- [Output mix and material controls](../output-mix/design.md) owns preflash exposure composition, Output lightness retirement, and final mixing. Per-stage, resident, converted-film, and diagnostic paths execute only material operations present in their route.
- Existing chemistry and scanner reference ownership remain unchanged.
- The shared output finishing boundary delegates CMYK ink style to [CMYK ink style](../cmyk-style/design.md). It MUST also serve the film-free `input` workflow without preparing a spectral chain.

## Prepared runtime state

`Runtime` MUST remain the production facade. Preparation MUST use explicit calls inside the existing owners, not a generic graph scheduler, service registry, or second public parameter model.

### Inputs and identity

- Each preparation component MUST define one typed input projection. Construction and reuse decisions MUST consume that projection rather than maintain separate lists of parameter dependencies.
- Component builders MUST receive their consumed controls and source data explicitly. They MUST NOT read unrelated fields through the complete `RuntimeParams` aggregate.
- Parameter digestion, topology normalization, and validation MUST retain their existing owners and precedence.
- Material preparation MUST start from the original source profiles and include the consumed base, chemistry, and development-time controls.
- Film curve gamma MUST belong to reference preparation, not to repeated material mutation.
- Preparation MUST preserve original film and print identities for grain, frame-cache, and material-artifact consumers. A new source snapshot MUST NOT reuse a result solely because its stock name or path matches an old snapshot.
- Identity checks MUST use the actual preparation device, not a backend wrapper's address. A device change MUST invalidate dependent numerical references as well as device tables.
- Input projections SHOULD borrow existing values. Reuse MUST NOT require copying profiles, hashing complete tables on each edit, or allocating a second parameter document for each component.
- This boundary MUST NOT add file watching, a persistent preparation cache, or a per-field version registry. A runtime retains its accepted source snapshot; constructing a runtime from newly loaded sources starts a new preparation identity.

### Component ownership

- Material preparation, the spectral front end, the convert-film inverse, middle-gray references, print calibration, and output gamut preparation MUST own separate reusable results where their dependencies differ.
- Cold construction and parameter updates MUST use the same component builders and reuse rules.
- Material-only and exposure-reference updates MUST retain compatible spectral tables. Spectral-input, route, or device changes MUST rebuild the affected preparation.
- The spectral representation MUST distinguish TC and Mallett algorithms. TC preparation MUST distinguish host samples from a device table and its descriptor; an absent optional update MUST NOT double as an algorithm choice.
- The converted-film inverse MUST remain independent of the spectral algorithm choice. A converted-film route MAY still consume the spectral front end for its reference calibration.
- The inverse's material view MUST contain the dye spectra, base density, and density-curve range it consumes. Its control view MUST contain conversion illumination, calibration, exposure, input color space, and transfer decoding.
- Middle-gray and print-reference preparation MUST preserve current arithmetic order, f64 neutral-filter values, exposure-tracking semantics, and backend-specific numerical boundaries.
- Updates on `input` and `input > film` MUST NOT prepare or validate unused print density-curve models. Input-only preparation MUST NOT construct unused film spectral tables or gray-card references.
- Print curve morphing and model-less print gamma MUST retain their render-time ownership.

### Commit and lifetime

- A parameter-update candidate MUST describe the complete resulting prepared state. Absence MUST mean that a component is not used, not that a caller retains unspecified old fields.
- Unchanged immutable preparation MUST be shared rather than deep-copied. A rejected candidate MUST NOT mutate or destroy the committed state.
- All fallible preparation, including grain record configuration, MUST finish before commit. Commit MUST perform no fallible work.
- Replacing committed preparation MUST NOT invalidate resources still owned by an in-flight operation. Rust ownership and shared immutable handles MUST carry this lifetime; image operations MUST NOT acquire generic undo callbacks.
- Prepared-state reuse and image-prefix reuse MUST remain separate. Image caches retain their source, geometry, seed, and operation-specific dependency checks.
- Request scheduling and image publication remain frontend responsibilities under [GUI worker ownership](../gui-workflow/design.md). Successful preparation alone MUST NOT mark a new image as published.

