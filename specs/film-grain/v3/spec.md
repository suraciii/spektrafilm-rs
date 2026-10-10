# Film grain V3

## Status

V3 is a planned, opt-in engine. This specification defines an implementable target. It does not establish an implemented or visually accepted result.

## Goal

V3 must produce convincing, visually pleasing color-negative grain in the rendered photograph. Real film scans must guide its texture and its relationship to subject detail, tone, and color.

The first comparison must address the reported V1 defects: dirty color variation, blurred grain, and damaged skin texture. Those observations define the problem. They do not establish its cause.

Subject shape, tone, color, and fine detail must pass through the grain-bearing image formation. A larger canvas, a softer source, or more simulated processes does not establish success.

## Image-forming grain

V3 must realize a conditional dye field in film coordinates. The developed film density sets the local conditional response. Output pixels must read that realization through the [design's formation and readout](design.md#conditional-dye-mass).

V3 must not retain an independent clean-detail image and add grain after it. A mean-plus-residual calculation is valid only when both terms describe the same conditional field and pass through the same formation and readout.

A feature narrower than the effective dye support must condition that field. It must not bypass the field as separately preserved clean detail. This rule does not require the feature to vanish or break into dots.

The output pixel is a readout sample. It is not a grain-generation unit. At a viewing scale where grain is claimed to be resolved, irregular grain structure must span neighboring output samples. One-pixel impulses, square cells, repeated stamps, seams, and visible sampling grids must fail review.

## Initial supported condition

The first implementation must support `kodak_portra_800`, a 35 mm full-film long edge, the `input > film > scan` route, and `scanner.scan_output = positive_scan`. It must use the existing calibrated negative-to-positive interpretation. The delivered positive photograph is the comparison surface.

The implementation must also support `direct_scan` on that route for diagnosis. A negative capture must not be compared with a positive reference without recording the interpretation.

Active V3 on another film, format, print route, convert-film route, or passthrough route must fail with an actionable unsupported-condition error. These restrictions define the first implementation's support, not permanent exclusions. Expansion requires the same validation for the additional condition.

The acceptance record must identify exposure where known, development, scanner or scan source, processing, output dimensions, film area, and viewing size. Unknown conditions must remain explicit. Incomplete references may guide appearance but must not establish stock-specific physical calibration.

## Controls and compatibility

`film_render.grain.engine = v3` must select V3 explicitly. V1 must remain the default for existing recipes. V1 and V2 must retain their existing behavior and defaults.

V3 must use the resolved `particle_area_um2`, `particle_scale`, `particle_scale_sublayers`, `density_min`, and `uniformity` controls defined by its [design](design.md#parameters). It must always use the film's three sublayers.

V3 must expose `v3_dye_support_um` as the film-space support control defined in its [design](design.md#parameters). Effective event area must control statistical strength independently of this support. All controls and defaults must survive recipe and GUI state round trips.

V1 final blur, dye-cloud blur, microstructure, density sharpening, composite-layer selection, and RMS presets must not silently control V3. V2 controls must not control V3. Inactive-engine fields remain available to their owning engines.

`active = false` must produce the existing grain-off result exactly. Preview mode, stochastic deactivation, spatial deactivation, and LUT mode must bypass V3 before its field parameters are resolved. Preview must be identified as grain-free. Scan and Export must execute active V3.

Timing, diagnostic taps, and output dumps must continue to exercise active V3. Only the named bypass modes may suppress its allocation.

V3 must use CPU execution. A WGPU request must retain the same semantics through the CPU route and report the effective backend. Resident execution must not omit or replace V3. Precision follows the actual executable; the f64 CLI is the numerical reference.

## Appearance

Grain must remain visible where the reference and viewing size make it visible. Its apparent size, softness, irregularity, and tonal behavior must resemble the reference. Grain need not consist of sharp dots or connected clusters.

Skin, gradients, dark and bright regions, saturated colors, and fine detail must remain credible with grain present. Detail loss must follow the declared formation and readout. Broad smears, artificial halos, and color blotches absent from the reference must fail review.

The film-to-scan conversion must determine grain polarity and color. V3 must not force white grain, neutral grain, a fixed shadow-to-highlight trend, or one channel correlation for all conditions.

Extra blur must not conceal the grain or its grid. Sharpening must not manufacture bright outlines as apparent grain improvement.

## Scale and repeatability

The same film area must retain its effective grain scale when output dimensions change. A crop must retain its full-film origin and surrounding field support. A crop must not restart texture or change grain size.

At fixed internal sampling and identical output footprints, overlapping full-frame, cropped, and tiled CMY reads must agree within the numerical tolerance in the [design](design.md#numerical-certification). Final RGB crop checks must retain the scanner's full-film spatial boundary. Different footprints caused by output rounding need not produce identical pixels. Lower output resolution may reduce visible grain. Higher resolution may reveal additional texture.

The same source, recipe, geometry, seed, precision, and implementation must reproduce the same result. Different seeds must not systematically change target tone or color.

Refining internal sampling must converge toward the same declared response. It must not act as an undocumented strength, size, or sharpness control. The supported output range must be recorded from exercised evidence. No universal output range or visible-grain equivalence is established by this specification alone.

## Acceptance

Acceptance must use lossless output from the actual Scan or Export path. The set must contain at least two photographs. Together they must include a portrait, a smooth region, dark and bright regions, saturated color, and fine subject detail.

Each photograph must show existing grain off, V1, V3's [deterministic expectation](design.md#deterministic-comparison), and stochastic V3 with the same source and non-grain settings. The V3 expectation must share development, target allocation, reconstruction, support, and readout with stochastic V3. It is a CMY expectation diagnostic, not the ensemble mean of scanned RGB. This comparison must distinguish changed processing order and deterministic detail transfer from random grain.

The V1 set must isolate its final blur, dye-cloud blur, microstructure, and density sharpening before attributing an improvement to V3. Comparisons must record parameter differences and use comparable apparent grain strength.

The [small-region optical gate](design.md#readout-and-detail) must pass before full-frame GUI integration. Actual CLI and GUI Scan/Export must then exercise the full frame and record runtime and peak memory. A cropped-only demonstration must not establish full-frame support.

Review must include the intended viewing size and 100% output. Enlargements may expose artifacts. Arbitrary magnification must not define success. JPEG output is secondary and must not conceal defects present in lossless output.

V3 must pass the numerical certification and the following image-forming procedure:

1. Record film geometry, source reconstruction, effective dye support, event area, field pitch, output footprint, seed, precision, and density-average readout approximation.
2. Render a uniform patch, a slanted edge, and lines narrower than, comparable to, and wider than the effective support. Use low, middle, and high developed-density conditions. Inspect the expectation, realization, and final positive output.
3. Translate the patterns by fractions of the effective support. Confirm that detail conditions the field and follows its response. Reject a clean-detail bypass, grid locking, repeated stamps, bright outlines, and arbitrary detail removal.
4. Read the same area at two output sizes and as a crop. Include a noninteger field-to-output ratio. Check fractional integration, film and scanner boundaries, tile seams, and footprint alignment before claiming overlap equality.
5. Repeat fixed seeds and use different seeds. Refine internal pitch independently of source and output size on small regions. Check independent impulse/edge/covariance oracles and count probabilities as well as implementation moments.
6. Compare the photograph set with real scans at the recorded viewing sizes. Record the user's assessment of grain structure, skin texture, color, and detail balance.

Both outcomes are required: the real-scan comparison supports the claimed behavior within its known conditions, and user review finds V3 more convincing and pleasing than the V1 baseline without compensating loss of grain visibility or subject quality.

Measurements must answer a specific discrepancy. RMS, covariance, spatial spectra, and edge response are diagnostic tools. Numerical certification verifies the specified algorithm. It does not establish Portra equivalence. Stock-specific thresholds require an appropriate reference and measurement protocol.

## Exclusions

The first implementation does not require microscopic crystal reconstruction, shared CMY fields, multiple texture scales, Dehancer Film Resolution, or a hard grain-diameter resolution cutoff. It does not promise V1 or Dehancer pixel parity.

Print-paper grain, semantic skin protection, and GPU optimization are outside the first implementation. Arbitrary final-RGB noise, JPEG tuning, and display sharpening must not substitute for the declared model.

## References

- [V3 design](design.md)
- [Film grain overview](../spec.md)
- [V1 contract](../v1/spec.md)
- [V2 contract](../v2/spec.md)
