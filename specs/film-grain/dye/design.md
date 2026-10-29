# Film grain Dye design

## Decision

Dye realizes developed film record density as a band-limited correlated field whose amplitude follows a granularity curve derived from development statistics. Every draw keeps the target mean exactly, so the field carries grain and never tone.

The field lives on the film lattice. The readout resamples it: finer grids interpolate a cell centre, coarser grids integrate the lattice cells the output cell covers. A crop, a preview scale, or a stage split therefore samples one film, not a second realization.

## Terms

**Film lattice**: the source raster's film-coordinate lattice. It is the only place the field is realized. The output grid is a sampling of it.

**Correlated field**: unit-variance noise filtered by the declared octave Gaussians and band-limited at 12 µm. It carries the aggregate texture.

**Granularity taper**: the relative amplitude `sqrt(sat · p · (1 − p))`, normalized by its own peak and floored. It vanishes at clear base and at record saturation.

**Record coupling**: the shared and independent split between the three records, `sqrt(rho)` on the shared field and `sqrt(1 − rho)` on each record's own field.

**Event area**: the density-times-area carried by one developed event, `q`. It sets the count variance, not a microscopic crystal size.

## Processing boundary

Dye consumes the developed record-density target at the film-density boundary and returns realized density in the same domain. It does not reinterpret scanner or print controls, and it does not read the current image's statistics.

## Film geometry

The caller supplies the full-film rectangle, source-pixel geometry, resolved crop, and actual output dimensions. Dye MUST NOT introduce a second film-format parameter or a stock-name format rule.

The GUI editor for `camera.film_format_mm` MUST use a 1-1000 mm usability range and MUST clamp typed values to that range. The validator MUST accept every strictly positive document value, including values outside the editor range. The [GUI workflow specification](../../gui-workflow/spec.md#numeric-editor-ranges) owns the general editor-range rule; this sentence owns the Dye geometry consequence.

Source pixels occupy declared film rectangles. The full-film origin is the global coordinate origin.

Crop bounds, tile bounds, output dimensions, dispatch shape, traversal order, and thread scheduling MUST NOT redefine the origin or the lattice.

Each output cell owns the film rectangle obtained by partitioning the requested region by the actual output dimensions. Readout MUST integrate fractional lattice overlap and MUST NOT assume an integer lattice-to-output ratio.

If downstream spatial processing needs context outside the requested crop, the caller MUST evaluate Dye on a full-film or proven-equivalent enclosing lattice, then extract the requested rectangle. A crop or tile edge MUST NOT reflect, clamp, truncate, or reseed the field.

Changing film geometry changes source and output footprint sizes. It MUST NOT silently change correlation length, event area, lattice pitch, target density, or stream addressing. Resource checks MUST reject an unrepresentable request before allocation.

## Density law

For record density `t`, event area `q`, output cell film area `A`, and fog baseline `f`:

```text
p      = clamp(t / dmax, 0, 1)
sat    = 1 - p * uniformity
taper  = max(sqrt(sat * p * (1 - p)) / peak, floor)
u_c    = sqrt(rho) * u_shared + sqrt(1 - rho) * u_c_own
e      = 1 + white * strength_c * taper * u_c
lambda = (t + f) * e * A / q
count  = Poisson(lambda)
D      = q * count / A - f
```

`E[D] = t` holds exactly because `E[count] = lambda`, and it holds per record, per cell, and for any strength.

The expectation diagnostic replaces `count` by `lambda` and the per-octave draws by their conditional means; it MUST return the target density.

Fog keeps the event rate alive at clear base. It is a density baseline, not a level offset: the same `f` is subtracted after the count, so tone is unchanged.

## Look controls

Let `s` be grain strength and `k` be grain scale. The delivered constants map as:

```text
white_eff    = white * s
base_eff     = base * s
q_eff        = q * s^2
octave_i_eff = octave_i * k
highpass_eff = highpass * k
```

`Var(D) = q_eff (t + f) e / A`, so the count term's standard deviation scales with `sqrt(q_eff)` while the envelope's standard deviation scales with `white_eff`; both scale linearly with `s`. The taper and the per-record ratio are unchanged.

`s = 0` is the deterministic limit: every draw is replaced by its conditional mean, so the law returns the target density with no random field or residual count noise. `s = 1` and `k = 1` leave the delivered constants unchanged.

`k` reaches both executors through the same octave and band-limit taps. A requested scale MUST scale the device band halo and workspace bound with it.

A mapped event area below the f32 normal minimum resolves to the deterministic limit on both executors. Above the exact count-rate threshold the count deviation is normal and evaluated directly, so a small event area cannot overflow the rate or lose the mean.

A scale and pitch whose Gaussian needs more taps than the device supports MUST fail before allocation. The kernel MUST NOT be truncated.

## Field construction

Each octave is white noise filtered by a Gaussian of `diameter / 2` in micrometres, standardized to unit variance, then summed with normalized weights. The sum is band-limited by subtracting its own 12 µm Gaussian and standardized once more.

The noise is coordinate-addressed: a 32-bit hash of seed, stream, and global film coordinates, Box-Muller to a normal. The CPU reference and the device pass use the same hash, the same streams, and the same coordinates, so one seed yields one realization on both executors.

Stream numbering is fixed: the shared field uses streams `0 * 8 + octave`, and record `c` uses `(c + 1) * 8 + octave`.

## Device execution

The device evaluates the same law in passes: one Gaussian per octave axis, a weighted combine, a band-limit subtract, then the density law per record.

The count is drawn by the same method on both executors: inversion below thirty events, PTRS above, up to the exact-rate threshold of `10^9` events per cell. Above that threshold both executors draw the same coordinate-addressed normal deviation.

A normal approximation below the threshold would match the mean and the variance but not the distribution, and the difference shows once the rate is small.

The normalize scale is measured on the host by filtering white noise with the very taps the shader uses. It measures each octave's gain and the band-limited scale together, because a Gaussian discretized below one pixel passes only part of the variance.

Bands carry a filter halo and key their noise on global coordinates, so halo overlap reproduces the field and bands join without a seam. The pass MUST NOT require a device-side reduction or a synchronization point.

## Deterministic comparison

The implementation MUST provide a diagnostic expectation mode that replaces each draw with its conditional mean while retaining target preparation, readout, downstream settings, and precision. It is the expectation at the density boundary, not the seed-ensemble mean after nonlinear spectral or scanner operations.

Expectation mode MUST NOT mutate the user recipe or become an alternate production grain law. It exists to separate deterministic transfer from stochastic variation during acceptance.

## Numerical certification

Correlation length is a model constant anchored to film geometry. It MUST NOT depend on source dimensions, crop size, output size, or seed.

Certification MUST cover constant targets, graded targets, saturation, fractional readout translations, noninteger lattice-to-output ratios, film boundaries, and band splits. It MUST compare production readout with an independent footprint-integration oracle that does not reuse production helpers.

Certification MUST measure expected-density error, readout RMS, spatial covariance or power spectrum, and band-split agreement. It MUST state numerical budgets before evaluating a candidate.

At fixed target, geometry, seed, precision, and output footprints, full-frame, crop, tiled, and stage-split reads of one film MUST agree within the declared precision budget.

## Optical measurement and calibration

The formation algorithm MUST NOT read statistics from the current image or normalize a realization to its measured RMS. Measurement is an external operation over fixed stimuli, fixed geometry, fixed receiver and aperture, fixed density interpretation, and a declared seed population.

The optical measurement record MUST identify:

- stock or profile identity and source provenance;
- film record-density and density-to-transmission conventions;
- receiver spectrum, aperture or point-spread function, wavelength range, and sampling;
- stimulus geometry, output footprint, and viewing condition;
- seed population and realization count;
- algorithm revision, numerical precision, and executor;
- measured mean, variance/RMS, covariance or spectrum, tone bias, and color bias;
- status: `measured`, `estimated`, `proxy`, `uncalibrated`, or `rejected`.

The calibration solver MUST fit parameters against a fixed development and optical condition. It MUST use separate fit and holdout stimuli or seeds. It MUST evaluate mean density, optical RMS, spatial correlation, and relevant tone/color bias together.

RMS is a measurement under a declared protocol. It is not automatically a runtime control, a universal stock property, or proof of microscopic grain size.

### Measurement statistics and revision

The standalone oracle owns the population estimators. For `N >= 2` receiver-density readings `D_i`, it MUST compute:

```text
mean = sum_i D_i / N
C_pop = sum_i (D_i - mean) (D_i - mean)^T / N
rms_density[j] = sqrt(C_pop[j,j])
uncertainty_density[j] = sqrt(C_pop[j,j] / (N - 1))
```

`covariance` MUST retain the descriptive divisor `N`. `uncertainty_density` MUST use the sample-variance estimate of the mean's standard error under independent seed realizations. It is not an RMS error bar.

The oracle MUST stamp new output records' existing `protocol.algorithm_revision` with the prefix `dye-measurement-v2:` followed by the caller's algorithm revision. An input revision already bearing this prefix MUST retain it without duplication.

Older unprefixed records MUST deserialize without relabeling or recomputing statistics. Their original generating revision determines their interpretation.

## Runtime integration and module boundaries

The implementation MUST keep these boundaries explicit:

1. **Target preparation:** full-film geometry, source reconstruction, film-stage density target, and profile resolution.
2. **Field realization:** lattice geometry, octave filtering, band limit, and record coupling.
3. **Count realization:** event area, fog baseline, and the conditional count.
4. **Readout:** film-coordinate resampling to the requested grid.
5. **Optical measurement:** independent transmission, aperture, density, and calibration records.
6. **Executor adapters:** CPU reference and GPU production implementations over the same law.

The CPU reference and GPU production executor MUST implement the same seed, stream, coordinate, and geometry semantics. They MAY differ in arithmetic precision and count-draw construction, and such differences MUST stay inside the count term.

No module MAY duplicate another module's film chemistry, optical measurement, parameter fit, or route selection. User surfaces MAY expose algorithm inputs, but they MUST NOT encode calibration evidence or infer stock calibration from an appearance label.

## Implementation gates

Implementation MUST proceed in gates:

1. Prove the density law and its mean on deterministic targets, including clear base, mid densities, and saturation.
2. Prove the field construction: per-octave variance, band-limit behaviour, and coordinate-addressed streams on both executors.
3. Prove readout: fractional overlap, constant preservation, and impulse and edge response against an independent oracle.
4. Prove crop, tile, band, and stage-split invariance and resource rejection without allocating an oversized request.
5. Prove the optical measurement oracle on synthetic constant and varying fields, including transmission positivity, kernel normalization, density ordering, wavelength integration, and receiver/aperture changes.
6. Produce per-stock or per-profile calibration records with fit and holdout separation.
7. Validate CPU f32/f64, usable hardware GPU, actual CLI and GUI Scan/Export paths, and photographic references only after the preceding gates pass.

Each gate MUST publish its inputs, algorithm revision, precision, seed population, output hashes or measurements, and pass/fail result. A local probe, one stock, one seed, one crop, or a software GPU adapter MUST NOT be presented as universal algorithm or photographic acceptance.

## Evidence and limits

Dye does not require microscopic crystal reconstruction, particle enumeration, multiple texture scales, encoded-RGB noise, scanner noise reduction, or a universal physical grain-diameter control. These are separate model decisions, not implicit fallbacks.

Real scans guide photographic validation. They do not define an algorithm until receiver, aperture, density, wavelength, source provenance, and processing are declared. Commercial simulation resemblance does not establish correctness or attribution.

The [product contract](spec.md#controls-and-resolved-inputs) owns parameter compatibility. It does not establish compatibility with earlier unsupported realizations or weaken the algorithm's validation gates.
