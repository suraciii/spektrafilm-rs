# Dye grain

Dye is a film-coordinate conditional record-field engine. This specification defines its product boundary, observable behavior, and acceptance gates. It does not lock calibration values.

## Goal

Dye MUST form convincing color-film grain by conditioning a stochastic dye field on developed film record density. Real film scans are evidence for photographic validation. They are not a substitute for a declared optical measurement protocol.

For monochrome profiles, Dye supplies a single-record BW effective model. Its availability MUST NOT be presented as calibrated silver-material fidelity.

Subject shape, tone, color, and fine detail MUST pass through the same field formation and readout as the grain realization. A larger canvas, a softer source, post-encoded noise, or a separate clean-detail path does not establish success.

## Image-forming contract

Dye MUST consume the developed record-density target at the film-density boundary. It MUST allocate the target across the selected film profile's three numerical curve components, realize conditional mass in global film coordinates, form the complete realization with the declared support operator, and read it through exact output footprints. Numerical components MUST NOT be described as measured silver populations.

Dye MUST NOT retain an independent clean-detail image and add grain after it. A mean-plus-residual implementation is valid only when the mean and residual represent the same conditional field and pass through identical formation and readout.

A feature narrower than the effective support MUST condition the field. It MUST NOT bypass the field as separately preserved detail. The output pixel is a readout sample, not a grain-generation unit.

Visible one-pixel impulses, repeated stamps, seams, square-cell grids, and crop-restarted texture fail acceptance.

The film, print, and scan chain owns grain polarity and color. Dye MUST NOT force white grain, neutral grain, a fixed shadow-to-highlight trend, or a universal inter-record correlation. Scanner chroma reduction, display sharpening, JPEG tuning, and final-RGB noise remain outside Dye.

## Supported product boundary

Dye supports color and monochrome filming profiles only when the loaded profile declares the material and workflow conditions required by the film-chain contract. The implementation MUST use the caller's resolved film geometry and MUST NOT add a second stock-name or format whitelist. Printing materials MUST NOT be admitted as filming stocks.

The product resolver MUST reject an unsupported material, chemistry, route, or missing profile data before output publication. It MUST report the unsupported condition and the reason. Availability of a profile is not proof of stock calibration, optical-readout acceptance, or photographic acceptance.

The implementation MUST run the same conditional law for negative and positive film profiles within each record layout. Polarity belongs to the resolved film-density curves and downstream interpretation. A negative/positive comparison is valid only when the interpretation, receiver, and output condition are recorded.

Changing film geometry MUST preserve requested output raster dimensions while changing physical source and output footprint sizes. It MUST NOT silently rescale support, event area, stock strength, field resolution, or count addressing. A geometry or resource request that cannot be represented MUST fail before publication.

## Controls and resolved inputs

Dye exposes only algorithm inputs that have an explicit owner and evidence contract. Support, event area, stock strength, sublayer response, occupancy, channel coupling, and field resolution have distinct meanings. The product MUST NOT expose independent particle-count, particle-mass, sparsity, and RMS controls that can contradict one another.

The resolved film profile supplies its sublayer response and stock-relative strength. Dye consumes that result; it does not reinterpret unrelated Spektra or Procedural controls. A control owned by another engine MUST NOT silently affect Dye.

The resolved profile MUST determine record layout. A monochrome profile MUST NOT enable an editable layout switch, a different grain engine, or an implicit silver-chemistry claim.

`channel_correlation` MUST remain stored but inactive for a single-record material. CLI reports and GUI controls MUST explain its inactivity without discarding the requested value. Selecting a color profile MUST restore its applicability.

Existing parameter names, ranges, defaults, and recipe syntax remain unchanged.

RMS is not a per-image normalization target. The formation path MUST NOT inspect the current image, crop, ROI, or seed realization to retune event area or any other parameter. Fixed-reference RMS behavior, if later offered, MUST be an explicit calibration operation with a declared reference condition and an infeasibility error.

It is not a hidden rendering step.

The product MUST record effective algorithm inputs and their provenance in acceptance output. A parameter value without a stock/profile, algorithm revision, and evidence status is incomplete evidence.

## Execution and repeatability

Active Dye MUST use the formation, exact Poisson count law, global-coordinate random addressing, and rectangular readout defined by the [Dye design](design.md). CPU and GPU executors MUST implement the same model. A GPU implementation MUST NOT substitute a CPU-prepared image-sized field or a different count distribution.

The pipeline MUST apply named inactive, preview, LUT, spatial-deactivation, and stochastic-deactivation bypasses before Dye field preparation. Active CLI Scan/Export and GUI Scan/Export MUST exercise Dye. The expectation diagnostic MUST NOT change the user's recipe or become a production bypass.

The same source, resolved inputs, geometry, seed, precision, and implementation MUST reproduce the same result on the same executor. Global cell coordinates MUST remain stable under crop and tile partitioning. Different seeds MUST alter the realization without systematically changing its target mean or color.

Different output dimensions MAY produce different footprints and therefore different samples; that difference MUST be reported as readout sampling, not as random drift.

## Acceptance

Acceptance is gated. A later gate MUST NOT compensate for an earlier gate that failed.

1. **Contract and preparation:** Prove supported-condition resolution, full-film target preparation, source reconstruction, three-sublayer allocation, target conservation, endpoint behavior, duplicate-knot behavior, zero weights, zero-capacity layers, and unsupported targets.
2. **Count law:** Prove the declared Poisson law, zero-rate behavior, rate boundaries, tail behavior, coordinate-addressed streams, and an explicit representability failure. A deliberately wrong-law control MUST fail predeclared statistical checks.
3. **Formation and readout:** Prove support normalization, area-integrated cell averaging, constant preservation, impulse and edge response, fractional footprints, noninteger ratios, film boundaries, and convergence against an independent rectangle-integration oracle.
4. **Partition invariance:** Prove same-canvas full-frame, crop, tile, band, and dispatch-shape invariance. Resource checks MUST reject oversized work before allocation.
   The implementation MUST NOT silently coarsen sampling or change the law.
5. **Optical boundary:** Run the independent spectral measurement oracle on constant, varying, edge, line, and representative stochastic fields. Declare receiver, aperture, density definition, wavelength range, source provenance, output footprint, and viewing condition.
   Compare transmission-first optical readout with production density readout before any photographic claim.
6. **Calibration:** Fit each stock/profile separately with fixed development and optical conditions. Use separate fit and holdout stimuli or seeds.
   Evaluate mean density, optical RMS, covariance or spectrum, tone bias, and color bias together. Mark evidence as `measured`, `estimated`, `proxy`, `uncalibrated`, or `rejected`.
7. **Executor and photographic validation:** Pair CPU f32/f64 with a usable hardware GPU where available. Exercise the real CLI and GUI Scan/Export paths.
8. Use lossless outputs and a declared reference set containing portraits, smooth fields, dark and bright regions, saturated colors, fine detail, and a color chart. Record the intended viewing size and the limits of the comparison.

Numerical proof and photographic proof are separate. A local synthetic probe, one seed, one crop, one stock, a software adapter, or a visually similar output MUST NOT be presented as universal algorithm or stock acceptance.

Every optical grain measurement MUST retain:

- stock/profile identity and source provenance;
- developed record-density and density-to-transmission convention;
- receiver spectrum, aperture or point-spread function, wavelength range, and sampling;
- stimulus geometry, output footprint, and viewing condition;
- seed population and realization count;
- algorithm revision, effective parameters, precision, and executor;
- mean density, variance/RMS, covariance or spectrum, tone bias, color bias, uncertainty, and artifact references;
- calibration status and explicit missing or unknown conditions.

The standalone `spektrafilm_core::dye_measurement::DyeOpticalOracle` validates this declared protocol, reads fixed CMY realizations through a normalized aperture kernel, integrates wavelength-dependent transmission before optical-density conversion, and produces `DyeMeasurementRecord` population statistics. It does not invoke Dye formation or inspect a current image.

The oracle MUST reject negative or non-finite absolute total spectral density before transmission conversion. It MUST report transmission underflow separately. It MUST NOT clamp invalid absorption.

A negative CMY coefficient MUST remain valid when the combined absolute spectral density is nonnegative. Receiver density relative to a declared reference MAY be negative.

Single-realization reading MUST remain available with a one-seed protocol. Population measurement MUST require at least two realizations. It MUST preserve the declared seed order, reject duplicate seeds, and enforce the declared aperture and footprint.

Population covariance and RMS MUST describe the supplied population with divisor `N`. Mean-density uncertainty MUST estimate the standard error of the mean under an independent-seed assumption. Unique seeds alone MUST NOT establish independence.

RMS and uncertainty MUST use optical-density units `D`, not `1000 D`. See [the measurement statistics contract](design.md#measurement-statistics-and-revision) for the estimators and record identity.

New population records MUST identify the measurement revision as well as the caller's algorithm revision. Reading an older record MUST preserve its recorded values and revision. It MUST NOT reinterpret an older uncertainty estimate as the current estimator.

RMS values from different receivers, apertures, density definitions, wavelengths, viewing sizes, or processing paths are not interchangeable. An RMS result MUST NOT be promoted to a runtime control or universal stock property without a new contract and evidence.

## Photographic review

The review set MUST compare grain-off, the current accepted baseline where applicable, Dye expectation, and stochastic Dye with identical non-Dye settings. The expectation MUST share target preparation, allocation, support, formation, readout, and downstream settings with stochastic Dye. Review MUST separate processing-order changes, deterministic detail transfer, and stochastic grain.

Inspect uniform patches, gradients, slanted edges, narrow and broad lines, skin, saturated colors, and high-contrast detail at the intended viewing size and at 100%. Grain MUST be visible where the reference makes it visible, while broad smears, artificial halos, color blotches, grid locking, seams, and arbitrary detail removal fail review.

The reference record MUST identify capture source, stock, scanner or receiver where known, processing, film geometry, output dimensions, viewing condition, and unknown provenance. Negative and positive references MUST NOT be compared without recording interpretation. Visual resemblance does not replace numerical or optical gates.

## Exclusions

Dye does not require microscopic crystal reconstruction, explicit particle enumeration, multiple texture scales, encoded-RGB noise, scanner noise reduction, or a universal grain-diameter cutoff. These are separate model decisions.

Dye does not promise pixel parity with Spektra, Procedural, Dehancer, a commercial simulator, or a scan receiver. It does not inherit another engine's controls, defaults, calibration, or evidence.

The BW extension preserves the existing parameter documents and color execution behavior. It defines no historical BW pixel compatibility and does not import another engine's calibration or feature-profile catalog.

## References

- [Dye design](design.md)
- [Film grain overview](../spec.md)
- [Spektra contract](../v1/spec.md)
- [Procedural contract](../v2/spec.md)
