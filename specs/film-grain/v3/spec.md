# Film grain V3

## Status

V3 is an opt-in engine. This specification defines its target support and the reference validation condition. Implementation gaps are recorded in the [design](design.md#current-implementation-gap). Acceptance evidence belongs to the delivering pull request.

## Goal

V3 must simulate the grain texture of real color-negative film as closely as the model allows. Real film scans are the reference for its texture and for its relation to subject detail, tone, and color.

V3 must correct the reported V1 defects: dirty color variation, blurred grain, and damaged skin texture.

Subject shape, tone, color, and fine detail must pass through the grain-bearing image formation. A larger canvas, a softer source, or more simulated processes does not establish success.

## Image-forming grain

V3 must realize a conditional dye field in film coordinates. The developed film density sets the local conditional response. Output pixels must read that realization through the [design's formation and readout](design.md#conditional-dye-mass).

V3 must not keep an independent clean-detail image and add grain after it. A mean-plus-residual calculation is valid only when both terms describe the same conditional field and pass through the same formation and readout.

A feature narrower than the effective dye support must condition that field. It must not bypass the field as separately preserved clean detail. This rule does not require the feature to vanish or break into dots.

The output pixel is a readout sample. It is not a grain-generation unit. At a viewing scale where grain is resolved, irregular grain structure must span neighboring output samples. One-pixel impulses, square cells, repeated stamps, seams, and visible sampling grids must fail review.

## Supported condition

V3 supports still color-negative film profiles (`info.type = "negative"`, `info.use = "still"`) at the resolved global `camera.film_format_mm` film long edge. V3 must consume this parameter as the only film-format source and must not impose a second format whitelist.

Changing film format must keep the requested output raster dimensions. It may change visible grain through the physical sampling defined in the [film geometry](design.md#film-geometry). V3 must not compensate this change with an automatic grain-size or strength adjustment.

The reference validation condition is a 36 mm long edge on the print route `input > film > print > scan` with `io.scan_film = false`. The film forms the grain. The existing enlarger, print paper, and print scan read it. The reference stock and the default controls are validated on this route. The 36 mm value is a validation reference, not a V3 format restriction.

V3 must also run on the film-scan route `input > film > scan` with `io.scan_film = true`, for `scanner.scan_output = positive_scan` and `direct_scan`. This route is a diagnostic condition. It must not be described as photographically accepted until it passes the [spectral readout gate](design.md#readout-and-detail) and the acceptance procedure below. A negative capture must not be compared with a positive reference without recording the interpretation.

Each stock must resolve its grain through its granularity preset, so that a coarser stock renders stronger grain than a finer one at the same density. The reference stock `kodak_portra_800` keeps the absolute event area validated for it. Other stocks move relative to that anchor.

Active V3 on another film chemistry, another usage (including `cine`), the convert-film route, or the passthrough route must fail with an actionable unsupported-condition error. A different `camera.film_format_mm` value must not fail only because it differs from 36 mm. Each additional format requires numerical, resource, and photographic evidence before it is described as validated support.

The acceptance record must identify exposure where known, development, scanner or scan source, processing, output dimensions, film format, film area, film pitch, field pitch, and viewing size. Unknown conditions must remain explicit. Incomplete references may guide appearance. They must not establish stock-specific physical calibration.

## Controls and compatibility

`film_render.grain.engine = v3` must select V3 explicitly. V1 must remain the default for existing recipes. V1 and V2 must keep their existing behavior and defaults.

V3 must use the current `camera.film_format_mm` value without replacing it when the engine changes. Its existing default of `35.0` must remain unchanged. A saved value of `35.0` must not be normalized to `36.0`. CLI parameter edits, presets, recipes, and GUI state must preserve the format through their existing parameter ownership and serialization rules. This change must not add a grain-specific format field or require a state-schema migration.

V3 owns the film-space dye support `v3_dye_support_um`, with default `3.5`, and the base statistical event area `v3_event_area_um2`, with default `1.28`. Its [design](design.md#parameters) defines both controls.

Effective event area must control statistical strength independently of the support. The stock granularity preset `rms_granularity` must set the strength ratio between channels and between stocks. V3 must also use the resolved stock controls `particle_scale_sublayers`, `density_min`, and `uniformity`. It must always use the film's three sublayers. All V3 controls and defaults must survive recipe and GUI state round trips.

V3 must not read `particle_area_um2` or `particle_scale`. V1 final blur, dye-cloud blur, microstructure, density sharpening, and composite-layer selection must not silently control V3. V2 controls must not control V3. Fields of inactive engines remain available to their owning engines.

`active = false` must produce the existing grain-off result exactly. Preview mode, stochastic deactivation, spatial deactivation, and LUT mode must bypass V3 before its field parameters are resolved. Preview must be identified as grain-free. Scan and Export must execute active V3.

Timing, diagnostic taps, and output dumps must continue to exercise active V3. Only the named bypass modes may suppress its allocation.

V3 must use CPU execution. A WGPU request must keep the same semantics through the CPU route and report the effective backend. Resident execution must not omit or replace V3. Precision follows the actual executable; the f64 CLI is the numerical reference.

## Appearance

Grain must remain visible where the reference and viewing size make it visible. Its apparent size, softness, irregularity, and tonal behavior must resemble the reference. Grain need not consist of sharp dots or connected clusters.

Skin, gradients, dark and bright regions, saturated colors, and fine detail must remain credible with grain present. Detail loss must follow the declared formation and readout. Broad smears, artificial halos, and color blotches absent from the reference must fail review.

The film, print, and scan chain must determine grain polarity and color. V3 must not force white grain, neutral grain, a fixed shadow-to-highlight trend, or one channel correlation for all conditions.

Lab scans can apply chroma noise reduction to film grain. That reduction belongs to the scanner, not to the film. V3 must not imitate it by correlating the dye records or by removing grain chroma. A comparison with a lab scan must record that the reference color can include this processing.

Extra blur must not conceal the grain or its grid. Sharpening must not manufacture bright outlines as apparent grain improvement.

## Scale and repeatability

The same film area must keep its effective grain scale when output dimensions change. A crop must keep its full-film origin and surrounding field support. A crop must not restart texture or change grain size.

At fixed internal sampling and identical output footprints, overlapping full-frame, cropped, and tiled CMY reads must agree within the numerical tolerance in the [design](design.md#numerical-certification). Final RGB crop checks must keep the full-film spatial boundary of every downstream spatial operation. Different footprints caused by output rounding need not produce identical pixels. Lower output resolution may reduce visible grain. Higher resolution may reveal additional texture.

The same source, recipe, geometry, seed, precision, and implementation must reproduce the same result. Different seeds must not systematically change target tone or color.

Refining internal sampling must converge toward the same declared response. It must not act as an undocumented strength, size, or sharpness control. The supported output range must be recorded from exercised evidence. This specification does not establish a universal output range or visible-grain equivalence.

## Acceptance

Acceptance must use lossless output from the actual Scan or Export path on the reference condition. The set must contain at least two photographs and one color chart. Together they must include a portrait, a smooth region, dark and bright regions, saturated color, and fine subject detail.

Each photograph must show existing grain off, V1, V3's [deterministic expectation](design.md#deterministic-comparison), and stochastic V3 with the same source and non-grain settings. The V3 expectation must share development, target allocation, reconstruction, support, and readout with stochastic V3. It is a CMY expectation diagnostic, not the ensemble mean of final RGB. This comparison must separate changed processing order and deterministic detail transfer from random grain.

The V1 set must isolate its final blur, dye-cloud blur, microstructure, and density sharpening before an improvement is attributed to V3. Comparisons must record parameter differences and use comparable apparent grain strength.

The real-scan comparison must show high-speed color-negative scans at native resolution. Each simulated patch must match the reference window's mean color and use the same film-space pitch as that scan. The record must give each reference's source, license, stock, scanner where known, and assumed film pitch.

The [small-region optical gate](design.md#readout-and-detail) must pass before full-frame GUI integration. Actual CLI and GUI Scan/Export must then exercise the full frame and record runtime and peak memory. A cropped-only demonstration must not establish full-frame support.

Review must include the intended viewing size and 100% output. Enlargements may expose artifacts. Arbitrary magnification must not define success. JPEG output is secondary and must not conceal defects present in lossless output.

V3 must pass the numerical certification and the following image-forming procedure:

1. Record film geometry, source reconstruction, effective dye support, event area, field pitch, output footprint, seed, precision, and density-average readout approximation.
2. Render a uniform patch, a slanted edge, and lines narrower than, comparable to, and wider than the effective support. Use low, middle, and high developed-density conditions. Inspect the expectation, realization, and final positive output.
3. Translate the patterns by fractions of the effective support. Confirm that detail conditions the field and follows its response. Reject a clean-detail bypass, grid locking, repeated stamps, bright outlines, and arbitrary detail removal.
4. Read the same area at two output sizes and as a crop. Include a noninteger field-to-output ratio. Check fractional integration, film and downstream spatial boundaries, tile seams, and footprint alignment before claiming overlap equality.
5. Repeat fixed seeds and use different seeds. Refine internal pitch independently of source and output size on small regions. Check independent impulse, edge, and covariance oracles, and count probabilities as well as implementation moments.
6. Compare the photograph set and the matched patches with real scans at the recorded viewing sizes. Record the user's assessment of grain structure, skin texture, color, and detail balance.

Both outcomes are required. The real-scan comparison must support the claimed behavior within its known conditions. User review must find V3 more convincing than the V1 baseline, without a compensating loss of grain visibility or subject quality.

Measurements must answer a specific discrepancy. RMS, covariance, spatial spectra, and edge response are diagnostic tools. Numerical certification verifies the specified algorithm. It does not establish Portra equivalence. Stock-specific thresholds require an appropriate reference and measurement protocol.

### Film-format acceptance

The format extension must pass this procedure in addition to the reference-condition acceptance:

1. Exercise long-edge values `8`, `16`, `24`, `35`, `36`, `56`, `65`, and `120` mm through the existing global parameter. These are numerical long-edge probes, not a format whitelist or names of calibrated emulsions. Include a fractional value, a square source, and a portrait source.
2. At fixed source dimensions, crop, and resize, confirm that changing the format preserves output dimensions and scales source pitch and output footprints by the long-edge ratio. Confirm that resolved stock controls, support, event area, and field pitch stay unchanged.
3. At smaller and larger formats, exercise uniform patches, edges, narrow lines, repeated seeds, and overlapping crop and tile reads. Apply the existing numerical tolerances at identical physical footprints. Use translated crops with full-film halos to bound the comparison workload.
4. Compare a uniform film region at the same physical source pitch and output footprints across two canvas extents. The overlapping CMY reads must agree within the existing crop tolerance when target conditions, global positions, support, seed, and boundary context agree. Changing only canvas extent must not change the local count law or grain covariance.
5. Render the complete photographic set at `24`, `36`, and `65` mm through the actual CLI and GUI Scan/Export paths. Inspect fixed output dimensions and a separate comparison with equal physical output pitch. Record runtime, peak memory, grain visibility, color, detail, and sampling differences. A cropped render must not substitute for this full-frame gate.
6. Exercise zero, negative, and non-finite format values through the shared parameter validator. Exercise a request that exceeds an existing coordinate or allocation limit. Each must fail with the parameter or exceeded limit identified before publication. Preview and the named bypass modes must retain their existing precedence and must not allocate a V3 field.

The record must distinguish exercised geometry, full-frame execution, and photographic validation. The `120` mm probe is a long-edge stress case; it does not designate 120 roll film. Evidence for one format must not be described as calibration for every film stock or format.

## Exclusions

This release does not require microscopic crystal reconstruction, correlated or shared CMY fields, multiple texture scales, Dehancer Film Resolution, or a hard grain-diameter resolution cutoff. It does not promise V1 or Dehancer pixel parity.

Print-paper grain, scanner chroma noise reduction, cine stocks, semantic skin protection, and GPU optimization are outside this release. Arbitrary final-RGB noise, JPEG tuning, and display sharpening must not substitute for the declared model.

## References

- [V3 design](design.md)
- [Film grain overview](../spec.md)
- [V1 contract](../v1/spec.md)
- [V2 contract](../v2/spec.md)
