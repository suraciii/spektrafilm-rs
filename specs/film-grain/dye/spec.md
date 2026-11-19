# Dye grain

Dye is a film-coordinate record-field engine. This specification defines its product boundary, observable behavior, and acceptance gates. It does not lock calibration values.

## Goal

Dye MUST form convincing color-film grain by conditioning a stochastic dye field on developed film record density. Real film scans are evidence for photographic validation. They are not a substitute for a declared optical measurement protocol.

For monochrome profiles, Dye supplies a single-record BW effective model. Its availability MUST NOT be presented as calibrated silver-material fidelity.

Subject shape, tone, color, and fine detail MUST pass through the same field realization and readout as the grain realization. A larger canvas, a softer source, post-encoded noise, or a separate clean-detail path does not establish success.

## Image-forming contract

Dye MUST consume the developed record-density target `t` at the film-density boundary and produce a realized density `D` with `E[D] = t` exactly.

The profile's one to three density layers MUST split `t` pointwise into non-negative layer targets whose sum is `t`, using the layer curves as relative shares.

The layer relative amplitude MUST follow the normalized material layer-variance envelope `sqrt(p * (1 - uniformity * p))`. It MUST vanish at clear base and at the asymptotic saturation `p = 1 / uniformity`, above the sampled layer peak; the sampled peak MUST retain a positive residual.

Each layer realizes its own coordinate-addressed value-noise field and conditional count; the realized layer densities MUST sum to `D`. The field MUST be a variance-preserving mixture of a base value-noise lattice and, for positive profiles, an additional microcontrast lattice.

The field MUST be realized on the film lattice and read out through the requested output geometry. Cropping, preview scaling, tiling, and stage splitting MUST NOT change the realization of a film coordinate.

The delivered value-noise base cell MUST depend on profile polarity. Negative profiles retain the delivered negative base; positive profiles use half that base. Positive profiles MUST also carry the delivered microcontrast component; negative profiles MUST NOT.

The delivered positive coarse cell MUST be finer than the negative coarse cell. It MUST occupy at least one source pixel at the 35 mm 1:1 reference geometry with a 3.66 µm source pitch. It MUST NOT be replaced by a subpixel field.

The positive microcontrast cell MUST be fixed in film coordinates at 9.0 µm. It MUST occupy at least one source pixel at the same reference geometry. It carries 60% of the field variance so the coarse cell remains the dominant correlation length at large `dye_size`.

The positive layer carrying the largest normal-density share MUST remain on that base cell, not a finer relative layer scale.

This polarity rule is a texture requirement, not permission for a separate sharpened detail layer. It MUST preserve the mean, layer split, readout geometry, and coordinate addressing defined by this contract.

Dye MUST NOT retain an independent clean-detail image and add grain after it. A mean-plus-residual implementation is valid only when mean and residual describe the same conditional field.

The film, print, and scan chain owns grain polarity and color. Dye MUST NOT force white grain, neutral grain, or a fixed shadow-to-highlight trend. Scanner chroma reduction, display sharpening, JPEG tuning, and final-RGB noise remain outside Dye.

## Supported product boundary

Dye supports color and monochrome filming profiles only when the loaded profile declares the material and route conditions required by the film-chain contract. The implementation MUST use the caller's resolved film geometry and MUST NOT add a second stock-name or format whitelist. Printing materials MUST NOT be admitted as filming stocks.

The product resolver MUST reject an unsupported material, chemistry, route, or missing profile data before output publication. It MUST report the unsupported condition and the reason. Availability of a profile is not proof of stock calibration, optical-readout acceptance, or photographic acceptance.

The implementation MUST run the same conditional law for negative and positive film profiles. Polarity belongs to the resolved film-density curves and downstream interpretation. A negative/positive comparison is valid only when the interpretation, receiver, and output condition are recorded.

Changing film geometry MUST preserve requested output raster dimensions while changing physical source and output footprint sizes. A geometry or resource request that cannot be represented MUST fail before publication.

## Controls and resolved inputs

| Control | Key | Range | Default | Meaning |
|---|---|---|---|---|
| Grain strength | `film_render.grain.dye_strength` | `[0, 4]` | `1.0` | Relative amplitude of the stochastic field and count deviation on the polarity's delivered scale |
| Dye grain size | `film_render.grain.dye_size` | `[1, 16]` | `1.0` | Relative coarse correlation length in film coordinates; scales the delivered base and layer ladder, while the positive microcontrast cell stays fixed |
| Inter-record grain separation | `film_render.grain.dye_color_grain` | `[0, 1]` | `0.75` | Separation of the three color records' grain fields: `0` shares one field, `1` gives each record its own |

`film_render.grain.active` owns the enable switch. No other Dye value is a public control.

Grain strength MUST scale the correlated field's standard deviation and the event count's standard deviation by the same factor. It MUST preserve `E[D] = t`, the derived density taper, and the per-record ratio. It is a relative strength control, not a certified optical RMS target or material calibration.

Dye grain size MUST scale the film-coordinate coarse correlation length relative to the polarity's base cell. It MUST scale the base cell and every active layer's relative cell. It MUST NOT scale the positive microcontrast cell, which stays fixed at 9.0 µm.

It MUST NOT change `E[D]`, the per-sample field variance, the event area, the amplitude mapping, or the count and field streams. At a fixed seed and aperture, the changed correlation length MAY change aperture-integrated granularity and perceived grain; this is not a tone shift.

The control is a relative coarse film-coordinate size, not a crystal diameter, scanner blur, output-pixel size, or magnification.

Inter-record grain separation MUST map to the record coupling as `rho = 1 - color_grain`. It MUST preserve `E[D] = t`, the density taper, the per-record amplitude ratio, and the overall amplitude. It changes the cross-record covariance, not image saturation.

A monochrome profile ignores the distinction and keeps its single effective record and delivered coupling; the GUI hides the control. Values outside `[0, 1]` MUST fail before rendering.

The delivered positive strength scale MUST be twice the delivered negative scale at the same public strength. This polarity constant compensates the density-to-encoded contrast of a direct positive scan; it is engine calibration, not a second control. The positive microcontrast variance fraction is 0.60 and is fixed, not an appearance control.

Grain strength `0` MUST return the target density at the Dye boundary with no random field and no residual count noise. `1` MUST leave the delivered law unchanged. Values outside the declared range MUST fail before rendering.

The polarity's delivered base cell is an engine constant. `dye_size` scales it; the result is not a crystal diameter, scanner blur, output-pixel size, or magnification.

An omitted strength key MUST resolve to `1.0`. An omitted size key MUST resolve to `1.0`.

An omitted color-grain key MUST resolve to `0.75`. An explicit `0.2` MUST preserve the previous delivered look's `rho = 0.8` law.

### Resolved law inputs

Dye's remaining law parameters are engine-derived constants: the per-record relative ratio and the polarity's delivered base cell at relative size 1. They also include the positive microcontrast cell and variance fraction, the delivered positive amplitude scale, event area, fog baseline, development uniformity, and two-component normalization.

The profile's density-layer curves determine the layer targets, the layer count, and each layer/channel sampled density peak `dmax`; they are material data, not appearance controls. A one-layer profile and a profile without layer curves use the composite curve peak, because neither is split into layer targets. The engine fallback applies only when no usable positive curve exists.

The engine owns each active layer's relative correlation scale, the polarity's delivered base cell, and the fixed positive microcontrast cell. `dye_size` scales the coarse base and ladder without exposing a per-layer control or moving the microcontrast cell.

The per-record ratio remains an advanced candidate, not a public control. It MUST NOT be exposed until it has a validated direction, range, and interaction with grain strength. Record coupling is exposed as inter-record grain separation: its direction is monotone, its range is `[0, 1]`, and it preserves the strength mapping and mean.

Its default is `0.75`; `0.2` reproduces the previous delivered law.

The resolved seed selects the realization. The same seed, inputs, geometry, and implementation MUST reproduce the same field on the same executor. The seed MUST NOT be presented as a grain-arrangement or creative control.

### Compatibility

Recipes that carry `rms_granularity`, `density_min`, `uniformity`, or `particle_scale_sublayers` MUST load and save unchanged. Those values MUST NOT change the delivered Dye field and MUST NOT be presented as effective Dye controls.

The historical `dye_support_um`, `dye_event_area_um2`, and `channel_correlation` keys remain readable as retired no-ops so documents saved by earlier builds reopen. Readers MUST discard them, and emitted documents MUST NOT contain them. Their values MUST NOT change the delivered Dye field, and they MUST NOT be presented as effective Dye controls.

### Deferred and internal controls

These candidates MUST remain internal or unexposed until a separate contract and evidence exist:

- **Per-record grain ratio:** relative C/M/Y field amplitude. It is not a white-balance control and does not equal final RGB grain amounts.
- **Absolute base cell size:** the polarity's delivered cell at relative size 1. It is engine data in film coordinates; `dye_size` is the public relative control.
- **Distribution width or roughness:** an internal spectrum-shape parameter. It is not `uniformity` and not an event-contribution width.
- **Tone-zone weights:** record-density zones are not final-image shadows and highlights. A creative control would need its own definition.
- **Fog grain:** a model or stock parameter, not a public knob.
- **Event area, field normalization, profile-derived layer Dmax, uniformity, fog baseline, and the per-record ratio:** internal law, material, calibration, or numerical values, not ordinary appearance controls.
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

1. **Law and mean:** Prove `E[D] = t` for uniform, graded, sampled-peak, and saturated targets, and prove the amplitude follows the derived granularity envelope. The sampled peak MUST retain a positive residual; asymptotic saturation MUST be its only zero above clear base, and expectation MUST equal the target.
2. **Partition invariance:** Prove same-canvas full-frame, crop, tile, band, and stage-split agreement at fixed seed and geometry. Resource checks MUST reject oversized work before allocation.
3. **Table partitioning:** A table larger than one binding MUST be partitioned by rows, each partition carrying the margins its read footprint needs, and the layout MUST NOT change the composed image.
4. **Executor parity:** Prove the CPU and GPU realizations agree in expectation exactly, and that stochastic realizations agree within the declared count-term bound.
5. **Optical measurement:** Run the independent spectral measurement oracle with a declared receiver, aperture, density definition, wavelength range, provenance, footprint, and viewing condition.
6. **Photographic validation:** Use lossless outputs and a declared reference set containing portraits, smooth fields, dark and bright regions, saturated colors, fine detail, and a color chart. Record the intended viewing size and the limits of the comparison.

### Inter-record grain separation acceptance

- **Defaults:** An omitted or `0.75` separation MUST produce the default field bit for bit on each executor. An explicit `0.2` MUST reproduce the previous delivered field bit for bit.
- **Direction:** At fixed seed, geometry, and target, raising separation from `0` through `0.5` to `1` MUST monotonically reduce cross-record correlation while the per-record mean stays at the target within the declared tolerance.
- **Limits:** `0` MUST realize the shared-field limit; `1` MUST realize independent record fields. Both limits MUST preserve the density taper, per-record ratio, and overall amplitude.
- **Strength interaction:** Inter-record grain separation MUST NOT change the strength mapping. At strength `0`, every separation value MUST return the target.
- **Handoff:** The CPU and device MUST receive the same mapped coupling and stream addressing.

### Strength-control acceptance

- **Defaults:** An omitted or `1.0` strength MUST produce the delivered field bit for bit on each executor.
- **Zero strength:** Strength `0` MUST return the target density at the Dye boundary. A final encoded comparison MUST state its downstream route and settings.
- **Amplitude:** At fixed seed, geometry, and output footprints, strengths `0.5`, `1`, and `2` MUST produce monotonically increasing realized standard deviation while the mean stays at the target within the declared tolerance.
- **Handoff:** The CPU and device MUST receive the same mapped field amplitude, event area, delivered correlation scale and relative size mapping, fixed microcontrast cell and weight, and profile layer table. A delivered cell that cannot form a finite positive cell size MUST fail before allocation.
- **Validity:** Every output MUST be finite. The acceptance record MUST report the tested stock, route, executor, target mean, channel bias, and spatial statistic.

### Grain-size acceptance

- **Default identity:** An omitted or `1.0` size MUST produce the delivered field bit for bit on each executor. The default MUST NOT change the existing strength, separation, microcontrast, event-area, or stream mapping.
- **Mapping:** At sizes `1`, `2`, `4`, `8`, `10`, and `16`, the delivered coarse base cell and every active layer's relative cell MUST scale by the size factor; the positive microcontrast cell MUST remain at 9.0 µm. The CPU and device MUST receive the same mapped coarse and fine scales on negative and positive profiles.
- **Mean and amplitude:** At fixed seed, geometry, target, and strength, changing size MUST leave the expectation at the target and MUST NOT change the per-sample field variance, event area, or strength mapping. The realization MAY differ because the film-coordinate lattice changes; that is not reseeding.
- **Correlation:** At fixed seed, geometry, and target, increasing size MUST increase the dominant correlation length. An acceptance record that reports aperture-integrated RMS or visible granularity MUST state its aperture and MUST NOT claim invariance.
- **Validity:** Values below `1` or above `16` MUST fail before rendering.

### Texture acceptance

- **Positive default cell:** At the 3.66 µm reference pitch, the delivered positive base cell MUST be finer than the negative cell and at least one source pixel wide. It MUST be about two source pixels per cell.
- **Positive microcontrast cell:** At the same reference pitch, the fixed 9.0 µm microcontrast cell MUST be at least one source pixel wide.
- **Crispness:** At fixed seed, target, and geometry, the positive delivered field MUST report a positive lag-1 adjacent-sample correlation and a shorter correlation tail than the negative delivered base. The record MUST report lags 1 through 4 and the declared spectral or adjacent-sample statistic.
- **Two failure modes:** A field that reads only as a low-frequency density cloud fails for the same reason as a field whose adjacent samples are uncorrelated pixel noise. The positive coarse base and its fixed microcontrast component MUST sit between those modes, not at either endpoint.
- **Scale separation:** At large `dye_size`, the positive coarse base MUST remain the dominant correlation length; the fixed microcontrast component MUST NOT replace it. At size `1` the 7.0 µm coarse cell and the 9.0 µm fixed component overlap by design because `dye_size` controls only the coarse scale.
- **Visual:** At 100% and the intended viewing size, positive grain MUST remain visible without broad smears, soapy clumps, or uncorrelated single-pixel noise. The review MUST use the same non-Dye settings as the negative comparison.

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
