# Lab scan design

The [readout contract](spec.md) owns behavior and numerical definitions.

## One frame boundary

A point-wise tone cannot provide automatic setup. The scan acquisition boundary already receives a complete developed density frame. Keep the reconstruction per-record and measure the frame at that boundary. Reuse the existing CPU acquisition fence for resident execution. A second GPU statistic implementation would duplicate numerical policy and need separate reduction-order guarantees.

The setup uses one f64 statistics pass in raster order and one fifth-percentile selection. It stores one scalar per pixel for percentile selection. Prepared reconstruction and the three setup offsets form an immutable full-frame baseline. Operator edits reuse that baseline without remeasuring the image. The calibrated pipeline cache does not own frame offsets.

## Neutral locus

A fixed density-chroma aim treats differences between a stock's neutral curves as scene casts. A single gray patch may pass while a neutral ramp changes color. Compute each pixel's residual from the neutral locus at its reconstructed mean exposure. Share the inverse's regularized axes for forward neutral evaluation and the local slope. This keeps the forward and inverse density origins equal.

This is a declared neutral-locus approximation. It is not the population-average calibration in the Kodak printer patent. The statistical guard does not identify whether a weak cast comes from scene illumination, film response, or intended scene color.

## Exposure and tone

Standard setup estimates exposure from the developed frame. Do not subtract the known simulated camera compensation afterward: that would correct the same exposure twice. Fixed styles retain their historical compensation and tone behavior. The tone family and its constants remain unchanged.

Normalizing the two published weight vectors is a product choice that makes the uniform aim a fixed point. Use each vector's own sum. The high-contrast coefficients sum to 0.94, not 0.98.

## Operator coordinates and source primaries

The [operator equations](spec.md#operator-correction) separate common exposure from zero-sum channel differences. Independent per-channel contrast would amplify channel imbalance and move a corrected gray off neutral. Common-exposure contrast avoids that failure without adding a second tone family. The neutral operator path bypasses coordinate arithmetic to preserve the sRGB baseline exactly. Host-side controls and prepared exposure data retain f64 values; a selected lower rendering precision does not change the saved parameter representation.

The tone defines a synthetic fixed linear-sRGB response. Treating its unchanged numbers as each selected output space changes their colorimetric meaning. A single source-to-output transform before the shared output tail corrects that boundary for CPU and resident execution. An exact sRGB identity avoids a rounding-only default regression. Saving-space conversion and display ICC processing must not substitute for this transform.

## Full-grid reuse and previews

Preparing a downsampled input changes density statistics, reconstruction, and spatial sampling. Sharing only offsets from a full frame with a separately simulated small frame is also insufficient. Render standard negative-to-positive on the exact working grid, then reduce terminal RGB for a requested preview. The GUI retains the full-grid output and reduces only its display raster. Preview digesting must not disable spatial effects on this path. Diagnostic taps keep the full working grid.

The core frame cache shares preparation between preview and scan requests when the upstream route exposes it. Optical Dye/Fotufilm grain may use exact full-route recomputation to retain optical context. Its identity combines the live input image identity, source material and backend identity, and all effective parameters except operator and preview settings. The cache must not retain an input through a strong identity reference. Crop, upscale, material, random seed, upstream controls, backend, and precision distinguish incompatible preparation. Pan, zoom, and operator edits do not. Replacing the input invalidates reuse even when dimensions match. Publish preparation only after successful completion; request epochs and revisions still guard output acceptance.

This cache is exact reuse, not a photometry lock. A target frame must prepare its own setup when corrections are copied. No GUI setup or tone implementation, separate approximate engine, or hidden f32 exposure cache is permitted. Full-grid buffers can be large; allocation failure must not silently change the grid or arithmetic policy.

## Saved and effective correction

Core runtime parameters own correction values and route/style validation. Presets and recipes carry explicit intent and must reject unsupported nondefault combinations. GUI state separately retains inactive editing intent; rendering receives neutral correction on an inapplicable route or style. Both carriers use the same typed operator values and reject unknown keys. No compatibility shim maps retired fields onto the new controls. Existing schema versions admit this additive neutral-defaulted group; the model version records the changed non-sRGB interpretation.

## Point-wise consumers

A LUT lattice is not a photograph. Treating it as one frame would make a baked result depend on the lattice size and sampling distribution. Reject automatic negative setup in LUT mode. Do not silently bake a fixed approximation or disable setup. Apply the same range-aware refusal in the instrument; density and exposure taps upstream of setup remain valid.

A fixed user-authored style remains the explicit point-wise alternative. No additional built-in name or setup switch is required.

## Names and provenance

Runtime scanner parameters own canonical style spelling. Deserialization maps `neutral` to `standard`; serialization emits the canonical spelling even for a programmatically assigned alias. The GUI catalog uses the same core catalog owner and hides reserved names. Reports carry the canonical style, the model revision, and the material-category readout selection. Debug tracing records per-frame offsets without adding a public control.

The model revision records pixel-changing readout, setup, and source-primary definitions. GUI state migration removes retired lamp keys only for explicitly versioned legacy documents. Current-schema unknown-field validation stays strict. Complete legacy documents preserve acquisition light under the Convert owner. Scanner-only presets discard the removed readout leaf without adding another section. Recipe, preset, and GUI adapters share the core cutover and provenance validators.

## Evidence limits

The arithmetic density estimator borrows coefficients from Eastman Kodak's *Adjusting photographic printer color exposure determination algorithms* (US4933709A, 1990). It is not physical LATD. The exposure target uses the normalized 0.184 camera reference and Kodak's 16% LAD scene target. These choices do not calibrate the renderer to a real minilab.

Synthetic tests establish fixed points, neutral-ramp behavior, exposure ordering, dominant-color retention, determinism, and backend integration. They do not establish perceptual quality or algorithmic optimality. The earlier research bake-off contained broadcasting and tone-parameter errors; it is not acceptance evidence for this implementation.

## Sources

- Eastman Kodak, *Adjusting photographic printer color exposure determination algorithms* (1990): https://patents.google.com/patent/US4933709A/en
- Fuji Photo Film, *Automatic photographic printing device simulator* (1990): https://patents.google.com/patent/US4893178A/en
- Kodak, *LAD for KODAK Color Negative Film*, H-61A: https://www.kodak.com/content/products-brochures/Film/LAD-for-KODAK-Color-Negative-Film-H-61a.pdf
