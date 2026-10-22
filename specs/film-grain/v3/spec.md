# Dye grain

Dye is a film-coordinate conditional dye-field engine. This specification defines its product boundary, observable behavior, and acceptance gates. It does not define compatibility with an earlier Dye implementation and does not lock calibration values.

## Goal

Dye must form convincing color-film grain by conditioning a stochastic dye field on developed film record density. Real film scans are evidence for photographic validation. They are not a substitute for a declared optical measurement protocol.

Subject shape, tone, color, and fine detail must pass through the same field formation and readout as the grain realization. A larger canvas, a softer source, post-encoded noise, or a separate clean-detail path does not establish success.

## Image-forming contract

Dye must consume the developed CMY record-density target at the film-density boundary. It must allocate the target across the selected film profile's three dye-forming sublayers, realize conditional dye mass in global film coordinates, form the complete realization with the declared support operator, and read it through exact output footprints.

Dye must not retain an independent clean-detail image and add grain after it. A mean-plus-residual implementation is valid only when the mean and residual represent the same conditional field and pass through identical formation and readout.

A feature narrower than the effective support must condition the field. It must not bypass the field as separately preserved detail. The output pixel is a readout sample, not a grain-generation unit. Visible one-pixel impulses, repeated stamps, seams, square-cell grids, and crop-restarted texture fail acceptance.

The film, print, and scan chain owns grain polarity and color. Dye must not force white grain, neutral grain, a fixed shadow-to-highlight trend, or a universal inter-record correlation. Scanner chroma reduction, display sharpening, JPEG tuning, and final-RGB noise remain outside Dye.

## Supported product boundary

Dye supports a color filming profile only when the loaded profile declares the material and workflow conditions required by the film-chain contract. The implementation must use the caller's resolved film geometry and must not add a second stock-name or format whitelist.

The product resolver must reject an unsupported material, chemistry, route, or missing profile data before output publication. It must report the unsupported condition and the reason. Availability of a profile is not proof of stock calibration, optical-readout acceptance, or photographic acceptance.

The implementation must run the same Dye algorithm for negative and positive color film profiles. Polarity belongs to the resolved film-density curves and downstream interpretation. A negative/positive comparison is valid only when the interpretation, receiver, and output condition are recorded.

Changing film geometry must preserve requested output raster dimensions while changing physical source and output footprint sizes. It must not silently rescale support, event area, stock strength, field resolution, or count addressing. A geometry or resource request that cannot be represented must fail before publication.

## Controls and resolved inputs

Dye exposes only algorithm inputs that have an explicit owner and evidence contract. Support, event area, stock strength, sublayer response, occupancy, channel coupling, and field resolution have distinct meanings. The product must not expose independent particle-count, particle-mass, sparsity, and RMS controls that can contradict one another.

The resolved film profile supplies its sublayer response and stock-relative strength. Dye consumes that result; it does not reinterpret unrelated Spektra or Procedural controls. A control owned by another engine must not silently affect Dye.

RMS is not a per-image normalization target. The formation path must not inspect the current image, crop, ROI, or seed realization to retune event area or any other parameter. Fixed-reference RMS behavior, if later offered, must be an explicit calibration operation with a declared reference condition and an infeasibility error. It is not a hidden rendering step.

The product must record effective algorithm inputs and their provenance in acceptance output. A parameter value without a stock/profile, algorithm revision, and evidence status is incomplete evidence.

## Execution and repeatability

Active Dye must use the formation, exact Poisson count law, global-coordinate random addressing, and rectangular readout defined by the [Dye design](design.md). CPU and GPU executors must implement the same model. A GPU implementation must not substitute a CPU-prepared image-sized field or a different count distribution.

The pipeline must apply named inactive, preview, LUT, spatial-deactivation, and stochastic-deactivation bypasses before Dye field preparation. Active CLI Scan/Export and GUI Scan/Export must exercise Dye. The expectation diagnostic must not change the user's recipe or become a production bypass.

The same source, resolved inputs, geometry, seed, precision, and implementation must reproduce the same result on the same executor. Global cell coordinates must remain stable under crop and tile partitioning. Different seeds must alter the realization without systematically changing its target mean or color. Different output dimensions may produce different footprints and therefore different samples; that difference must be reported as readout sampling, not as random drift.

## Acceptance

Acceptance is gated. A later gate must not compensate for an earlier gate that failed.

1. **Contract and preparation:** Prove supported-condition resolution, full-film target preparation, source reconstruction, three-sublayer allocation, target conservation, endpoint behavior, duplicate-knot behavior, zero weights, zero-capacity layers, and unsupported targets.
2. **Count law:** Prove the declared Poisson law, zero-rate behavior, rate boundaries, tail behavior, coordinate-addressed streams, and an explicit representability failure. A deliberately wrong-law control must fail predeclared statistical checks.
3. **Formation and readout:** Prove support normalization, area-integrated cell averaging, constant preservation, impulse and edge response, fractional footprints, noninteger ratios, film boundaries, and convergence against an independent rectangle-integration oracle.
4. **Partition invariance:** Prove same-canvas full-frame, crop, tile, band, and dispatch-shape invariance. Resource checks must reject oversized work before allocation. The implementation must not silently coarsen sampling or change the law.
5. **Optical boundary:** Run the independent spectral measurement oracle on constant, varying, edge, line, and representative stochastic fields. Declare receiver, aperture, density definition, wavelength range, source provenance, output footprint, and viewing condition. Compare transmission-first optical readout with production density readout before any photographic claim.
6. **Calibration:** Fit each stock/profile separately with fixed development and optical conditions. Use separate fit and holdout stimuli or seeds. Evaluate mean density, optical RMS, covariance or spectrum, tone bias, and color bias together. Mark evidence as `measured`, `estimated`, `proxy`, `uncalibrated`, or `rejected`.
7. **Executor and photographic validation:** Pair CPU f32/f64 with a usable hardware GPU where available. Exercise the real CLI and GUI Scan/Export paths. Use lossless outputs and a declared reference set containing portraits, smooth fields, dark and bright regions, saturated colors, fine detail, and a color chart. Record the intended viewing size and the limits of the comparison.

Numerical proof and photographic proof are separate. A local synthetic probe, one seed, one crop, one stock, a software adapter, or a visually similar output must not be presented as universal algorithm or stock acceptance.

Every optical grain measurement must retain:

- stock/profile identity and source provenance;
- developed record-density and density-to-transmission convention;
- receiver spectrum, aperture or point-spread function, wavelength range, and sampling;
- stimulus geometry, output footprint, and viewing condition;
- seed population and realization count;
- algorithm revision, effective parameters, precision, and executor;
- mean density, variance/RMS, covariance or spectrum, tone bias, color bias, uncertainty, and artifact references;
- calibration status and explicit missing or unknown conditions.

The standalone `spektrafilm_core::dye_measurement::DyeOpticalOracle` validates this declared protocol, reads fixed CMY realizations through a normalized aperture kernel, integrates wavelength-dependent transmission before optical-density conversion, and produces `DyeMeasurementRecord` population statistics. It does not invoke Dye formation or inspect a current image.

RMS values from different receivers, apertures, density definitions, wavelengths, viewing sizes, or processing paths are not interchangeable. An RMS result must not be promoted to a runtime control or universal stock property without a new contract and evidence.

## Photographic review

The review set must compare grain-off, the current accepted baseline where applicable, Dye expectation, and stochastic Dye with identical non-Dye settings. The expectation must share target preparation, allocation, support, formation, readout, and downstream settings with stochastic Dye. Review must separate processing-order changes, deterministic detail transfer, and stochastic grain.

Inspect uniform patches, gradients, slanted edges, narrow and broad lines, skin, saturated colors, and high-contrast detail at the intended viewing size and at 100%. Grain must be visible where the reference makes it visible, while broad smears, artificial halos, color blotches, grid locking, seams, and arbitrary detail removal fail review.

The reference record must identify capture source, stock, scanner or receiver where known, processing, film geometry, output dimensions, viewing condition, and unknown provenance. Negative and positive references must not be compared without recording interpretation. Visual resemblance does not replace numerical or optical gates.

## Exclusions

Dye does not require microscopic crystal reconstruction, explicit particle enumeration, multiple texture scales, encoded-RGB noise, scanner noise reduction, or a universal grain-diameter cutoff. These are separate model decisions.

Dye does not promise pixel parity with Spektra, Procedural, Dehancer, a commercial simulator, or a scan receiver. It does not inherit another engine's controls, defaults, calibration, or evidence.

This is a clean algorithm contract. Legacy parameter names, defaults, serialized shapes, feature-profile catalogs, migration behavior, and prior pixel identity are not part of this specification. If the product needs any of those behaviors, they require a separate decision and specification.

## References

- [Dye design](design.md)
- [Film grain overview](../spec.md)
- [Spektra contract](../v1/spec.md)
- [Procedural contract](../v2/spec.md)
