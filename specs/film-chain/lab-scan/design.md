# Lab scan design

The [readout contract](spec.md) owns behavior and numerical definitions.

## One frame boundary

A point-wise tone cannot provide automatic setup. The scan acquisition boundary already receives a complete developed density frame. Keep the reconstruction per-record and measure the frame at that boundary. Reuse the existing CPU acquisition fence for resident execution. A second GPU statistic implementation would duplicate numerical policy and need separate reduction-order guarantees.

The setup uses one f64 statistics pass in raster order and one fifth-percentile selection. It stores one scalar per pixel for percentile selection. It returns three immutable log-exposure offsets. The pixel interpretation reuses these offsets. Setup does not keep image state in the calibrated pipeline cache.

## Neutral locus

A fixed density-chroma aim treats differences between a stock's neutral curves as scene casts. A single gray patch may pass while a neutral ramp changes color. Compute each pixel's residual from the neutral locus at its reconstructed mean exposure. Share the inverse's regularized axes for forward neutral evaluation and the local slope. This keeps the forward and inverse density origins equal.

This is a declared neutral-locus approximation. It is not the population-average calibration in the Kodak printer patent. The statistical guard does not identify whether a weak cast comes from scene illumination, film response, or intended scene color.

## Exposure and tone

Standard setup estimates exposure from the developed frame. Do not subtract the known simulated camera compensation afterward: that would correct the same exposure twice. Fixed styles retain their historical compensation and tone behavior. The tone family and its constants remain unchanged.

Normalizing the two published weight vectors is a product choice that makes the uniform aim a fixed point. Use each vector's own sum. The high-contrast coefficients sum to 0.94, not 0.98.

## Point-wise consumers

A LUT lattice is not a photograph. Treating it as one frame would make a baked result depend on the lattice size and sampling distribution. Reject automatic negative setup in LUT mode. Do not silently bake a fixed approximation or disable setup. Apply the same range-aware refusal in the instrument; density and exposure taps upstream of setup remain valid.

A fixed user-authored style remains the explicit point-wise alternative. No additional built-in name or setup switch is required.

## Names and provenance

Runtime scanner parameters own canonical style spelling. Deserialization maps `neutral` to `standard`; serialization emits the canonical spelling even for a programmatically assigned alias. The GUI catalog uses the same core catalog owner and hides reserved names. Reports carry the canonical style, the model revision, and the material-category readout selection. Debug tracing records per-frame offsets without adding a public control.

The model revision changes because both readout light and automatic setup change pixels. GUI state migration removes retired lamp keys only for explicitly versioned legacy documents. Current-schema unknown-field validation stays strict.

## Evidence limits

The arithmetic density estimator borrows coefficients from Eastman Kodak's *Adjusting photographic printer color exposure determination algorithms* (US4933709A, 1990). It is not physical LATD. The exposure target uses the normalized 0.184 camera reference and Kodak's 16% LAD scene target. These choices do not calibrate the renderer to a real minilab.

Synthetic tests establish fixed points, neutral-ramp behavior, exposure ordering, dominant-color retention, determinism, and backend integration. They do not establish perceptual quality or algorithmic optimality. The earlier research bake-off contained broadcasting and tone-parameter errors; it is not acceptance evidence for this implementation.

## Sources

- Eastman Kodak, *Adjusting photographic printer color exposure determination algorithms* (1990): https://patents.google.com/patent/US4933709A/en
- Fuji Photo Film, *Automatic photographic printing device simulator* (1990): https://patents.google.com/patent/US4893178A/en
- Kodak, *LAD for KODAK Color Negative Film*, H-61A: https://www.kodak.com/content/products-brochures/Film/LAD-for-KODAK-Color-Negative-Film-H-61a.pdf
