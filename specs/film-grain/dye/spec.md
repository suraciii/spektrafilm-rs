# Dye grain

Dye is a film-coordinate record-field engine. This specification defines its product boundary, observable behavior, and acceptance gates. It does not lock calibration values.

## Goal

Dye MUST form convincing color-film grain by conditioning a stochastic dye field on developed film record density. Real film scans are evidence for photographic validation. They are not a substitute for a declared optical measurement protocol.

For monochrome profiles, Dye supplies a single-record BW effective model. Its availability MUST NOT be presented as calibrated silver-material fidelity.

Subject shape, tone, color, and fine detail MUST pass through the same field formation and readout as the grain realization. A larger canvas, a softer source, post-encoded noise, or a separate clean-detail path does not establish success.

## Image-forming contract

Dye MUST consume the developed record-density target `t` at the film-density boundary and produce a realized density `D` with `E[D] = t` exactly.

The realization MUST be a band-limited correlated field whose relative amplitude follows a granularity curve derived from development statistics. The amplitude MUST vanish at clear base and at saturation of the record.

The field MUST be realized on the film lattice and read out through the requested output geometry. Cropping, preview scaling, tiling, and stage splitting MUST NOT change the realization of a film coordinate.

Dye MUST NOT retain an independent clean-detail image and add grain after it. A mean-plus-residual implementation is valid only when mean and residual describe the same conditional field.

The film, print, and scan chain owns grain polarity and color. Dye MUST NOT force white grain, neutral grain, a fixed shadow-to-highlight trend, or a universal inter-record correlation. Scanner chroma reduction, display sharpening, JPEG tuning, and final-RGB noise remain outside Dye.

## Supported product boundary

Dye supports color and monochrome filming profiles only when the loaded profile declares the material and workflow conditions required by the film-chain contract. The implementation MUST use the caller's resolved film geometry and MUST NOT add a second stock-name or format whitelist. Printing materials MUST NOT be admitted as filming stocks.

The product resolver MUST reject an unsupported material, chemistry, route, or missing profile data before output publication. It MUST report the unsupported condition and the reason. Availability of a profile is not proof of stock calibration, optical-readout acceptance, or photographic acceptance.

The implementation MUST run the same conditional law for negative and positive film profiles. Polarity belongs to the resolved film-density curves and downstream interpretation. A negative/positive comparison is valid only when the interpretation, receiver, and output condition are recorded.

Changing film geometry MUST preserve requested output raster dimensions while changing physical source and output footprint sizes. A geometry or resource request that cannot be represented MUST fail before publication.

## Controls and resolved inputs

| Control | Key | Range | Default | Meaning |
|---|---|---|---|---|
| Grain strength | `film_render.grain.dye_strength` | `[0, 4]` | `1.0` | Relative amplitude of the stochastic field and count deviation |
| Grain scale | `film_render.grain.dye_scale` | `[0.5, 4]` | `1.0` | Relative correlation length and band limit in film coordinates |

`film_render.grain.active` owns the enable switch. No other Dye value is a public control.

Grain strength MUST scale the correlated field's standard deviation and the event count's standard deviation by the same factor. It MUST preserve `E[D] = t`, the derived density taper, and the per-record ratio. It is a relative look control, not a certified optical RMS target or material calibration.

Grain strength `0` MUST return the target density at the Dye boundary with no random field and no residual count noise. `1` MUST leave the delivered law unchanged. Values outside the declared range MUST fail before rendering.

Grain scale MUST multiply the field's correlation lengths and band limit in film coordinates. It MUST NOT change the target density, event area, per-record ratio, or readout geometry.

It is not a crystal diameter, scanner blur, output-pixel size, or magnification. `1` MUST leave the delivered field unchanged. Values outside the declared range MUST fail before rendering.

An omitted strength or scale key MUST resolve to `1.0`.

### Resolved law inputs

Dye's remaining law parameters are engine-derived constants: the per-record relative ratio, record coupling, the delivered correlation lengths, the delivered band limit, the delivered event area, the fog baseline, saturation density, and development uniformity.

The per-record ratio and record coupling are advanced candidates, not public controls. They MUST NOT be exposed until each has a validated direction, range, and interaction with grain strength. Their current delivered values remain engine constants.

The resolved seed selects the realization. The same seed, inputs, geometry, and implementation MUST reproduce the same field on the same executor. The seed MUST NOT be presented as a grain-arrangement or creative control.

### Compatibility

Recipes that carry `dye_support_um`, `dye_event_area_um2`, `rms_granularity`, `density_min`, `uniformity`, `particle_scale_sublayers`, or `channel_correlation` MUST load and save unchanged. Those values MUST NOT change the delivered Dye field and MUST NOT be presented as effective Dye controls.

### Deferred and internal controls

These candidates MUST remain internal or unexposed until a separate contract and evidence exist:

- **Per-record grain ratio:** relative C/M/Y field amplitude. It is not a white-balance control and does not equal final RGB grain amounts.
- **Record coupling:** the shared-field fraction between records. Its output direction and amplitude MUST be validated through the resolved color chain before exposure.
- **Distribution width or roughness:** an internal spectrum-shape parameter. It is not `uniformity` and not an event-contribution width.
- **Tone-zone weights:** record-density zones are not final-image shadows and highlights. A creative control would need its own definition.
- **Fog grain:** a model or stock parameter, not a public knob.
- **Event area, support, numerical grid, octave details, high-pass, Dmax, uniformity, and sublayer scales:** internal law, calibration, or numerical values, not ordinary appearance controls.
- **Film format, scanner blur, and sharpening:** owned by their modules; they do not replace texture scale.

The expectation diagnostic replaces every draw by its conditional mean and MUST remain a diagnostic. It MUST NOT change the user's recipe or become a production bypass.

The resolved film profile determines record layout and per-record response. Dye consumes that result; it does not reinterpret unrelated Spektra or Procedural controls. A control owned by another engine MUST NOT silently affect Dye.

## Execution and repeatability

Active Dye MUST use the continuous law and rectangular readout defined by the [Dye design](design.md). CPU and GPU executors MUST implement the same law, the same seed and stream addressing, and the same film-coordinate geometry.

Both executors MUST draw the count from the same law: exact Poisson by inversion below thirty events and the PTRS rejection method above it up to the declared large-rate threshold of `10^9` events per cell. Above that threshold both executors MUST use the same coordinate-addressed normal deviation.

The distributions agree to the declared precision, and the branch prevents a tiny mapped event area from overflowing the rate or losing the mean. A normal approximation below the threshold is inadmissible: it changes the distribution once the rate is small, and a heavily oversampled geometry reaches that regime.

Both executors MUST also address the underlying uniform by global cell coordinate and sample ordinal, never by traversal order, so one seed selects one realization on either backend. A sequential stream is not admissible: a device runs its invocations in an order it does not control. Executors MAY differ only in arithmetic precision.

The pipeline MUST apply named inactive, preview, LUT, spatial-deactivation, and stochastic-deactivation bypasses before field preparation. Active CLI Scan/Export and GUI Scan/Export MUST exercise Dye.

Global film coordinates MUST remain stable under crop and tile partitioning. Different seeds MUST alter the realization without systematically changing its target mean or color.

Different output dimensions MAY produce different footprints and therefore different samples. That difference MUST be reported as readout sampling, not as random drift.

## Acceptance

Acceptance is gated. A later gate MUST NOT compensate for an earlier gate that failed.

1. **Law and mean:** Prove `E[D] = t` for uniform, graded, and saturated targets, and prove the amplitude follows the derived granularity curve. The expectation realization MUST equal the target.
2. **Partition invariance:** Prove same-canvas full-frame, crop, tile, band, and stage-split agreement at fixed seed and geometry. Resource checks MUST reject oversized work before allocation.
3. **Table partitioning:** A table larger than one binding MUST be partitioned by rows, each partition carrying the margins its read footprint needs, and the layout MUST NOT change the composed image.
4. **Executor parity:** Prove the CPU and GPU realizations agree in expectation exactly, and that stochastic realizations agree within the declared count-term bound.
5. **Optical measurement:** Run the independent spectral measurement oracle with a declared receiver, aperture, density definition, wavelength range, provenance, footprint, and viewing condition.
6. **Photographic validation:** Use lossless outputs and a declared reference set containing portraits, smooth fields, dark and bright regions, saturated colors, fine detail, and a color chart. Record the intended viewing size and the limits of the comparison.

### Look-control acceptance

- **Defaults:** An omitted or `1.0` strength and scale MUST produce the delivered field bit for bit on each executor.
- **Zero strength:** Strength `0` MUST return the target density at the Dye boundary. A final encoded comparison MUST state its downstream route and settings.
- **Amplitude:** At fixed seed, geometry, and output footprints, strengths `0.5`, `1`, and `2` MUST produce monotonically increasing realized standard deviation while the mean stays at the target within the declared tolerance.
- **Scale:** At fixed seed, geometry, and output footprints, scales `0.5`, `1`, and `2` MUST produce monotonically coarser texture under a declared spectral or adjacent-sample statistic.
- **Handoff:** The CPU and device MUST receive the same mapped field amplitude, event area, and correlation scale. A scale and pitch whose Gaussian exceeds the supported device tap radius MUST fail before allocation; the kernel MUST NOT be truncated.
- **Validity:** Every output MUST be finite. The acceptance record MUST report the tested stock, route, executor, target mean, channel bias, and spatial statistic.

Scale acceptance MUST state that correlation length is in film coordinates and that the statistic is not a crystal-size estimate.

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

The review set MUST compare grain-off, Dye expectation, and stochastic Dye with identical non-Dye settings. The expectation MUST share target, geometry, and downstream settings with stochastic Dye. Review MUST separate processing-order changes, deterministic detail transfer, and stochastic grain.

Inspect uniform patches, gradients, slanted edges, narrow and broad lines, skin, saturated colors, and high-contrast detail at the intended viewing size and at 100%. Grain MUST be visible where the reference makes it visible, while broad smears, artificial halos, color blotches, grid locking, seams, and arbitrary detail removal fail review.

The reference record MUST identify capture source, stock, scanner or receiver where known, processing, film geometry, output dimensions, viewing condition, and unknown provenance. Negative and positive references MUST NOT be compared without recording interpretation. Visual resemblance does not replace numerical or optical gates.

## Exclusions

Dye does not require microscopic crystal reconstruction, explicit particle enumeration, multiple texture scales, encoded-RGB noise, scanner noise reduction, or a universal grain-diameter cutoff. These are separate model decisions.

Dye does not promise pixel parity with Spektra, Procedural, Dehancer, a commercial simulator, or a scan receiver. It does not inherit another engine's controls, defaults, calibration, or evidence.

The BW extension preserves the existing parameter documents and color execution behavior. It defines no historical BW pixel compatibility and does not import another engine's calibration or feature-profile catalog.

## References

- [Dye design](design.md)
- [Film grain overview](../spec.md)
- [Spektra contract](../spektra/spec.md)
- [Procedural contract](../procedural/spec.md)
