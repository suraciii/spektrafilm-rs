# Film grain Dye design

## Decision

Dye realizes developed film record density as a coordinate-addressed value-noise field whose amplitude follows a granularity curve derived from development statistics.

The delivered base cell is polarity-dependent: negative profiles use 14.0 µm, and positive profiles use 7.0 µm. Positive profiles keep that base on every density layer and add a 3.85 µm microcontrast lattice at 35% field variance. The finer positive structure keeps visible grain from collapsing into a low-frequency density cloud without returning to subpixel noise.

Positive profiles also carry twice the delivered field and count amplitude at the same public strength. Their direct-scan density-to-encoded contrast maps the same density modulation to a smaller encoded contrast than a print scan.

The profile's density layers split the developed record into one to three layer targets; each layer carries its own field and conditional count, and the layer densities sum back to the developed target. Every draw preserves the target mean in expectation, so the field carries grain and never tone.

The field lives on the film lattice. The readout resamples it: finer grids interpolate a cell centre, coarser grids integrate the lattice cells the output cell covers. A crop, a preview scale, or a stage split therefore samples one film, not a second realization.

## Terms

**Film lattice**: the source raster's film-coordinate lattice. It is the only place the field is realized. The output grid is a sampling of it.

**Value-noise field**: one unit-variance smoothstep-interpolated value-noise lattice. It carries the aggregate texture at a film-coordinate correlation length.

**Delivered base cell**: the base value-noise cell. Negative profiles deliver 14.0 µm; positive profiles deliver 7.0 µm. It is engine data, not a public control.

**Microcontrast field**: an independent value-noise lattice mixed into the field by variance. Positive profiles deliver a relative cell of 0.55 and a variance fraction of 0.35; negative profiles deliver zero weight. The mixture keeps unit variance.

**Density layer**: one of the profile's one to three emulsion layers. Its raw layer curve supplies a relative share, normalized pointwise so the layer targets sum to the developed record target. A small non-monotonic toe in the measured composite curve is flattened to a monotone envelope before inversion.

**Granularity taper**: the relative amplitude `sqrt(sat · p · (1 − p))`, normalized by its own peak. It vanishes at clear base and at record saturation.

**Record coupling**: the shared and independent split between the three records, `sqrt(rho)` on the shared field and `sqrt(1 − rho)` on each record's own field. The shared field is common to the records of one density layer; each layer has its own record streams.

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

For record density `t`, normalized layer share `t_l` with `sum_l t_l = t`, event area `q`, output cell film area `A`, and fog baseline `f`:

```text
p_l    = clamp(t_l / dmax, 0, 1)
sat_l  = 1 - p_l * uniformity
taper_l= sqrt(sat_l * p_l * (1 - p_l)) / peak
u_base = sqrt(1 - w) * u_coarse + sqrt(w) * u_fine
u_c,l  = sqrt(rho) * u_shared + sqrt(1 - rho) * u_c_own,l
e_l    = 1 + white * strength_c * taper_l * u_c,l
lambda_l = (t_l + f/n) * e_l * A / q
count_l  = Poisson(lambda_l)
D      = sum_l (q * count_l / A - f/n)
```

`E[D] = t` holds exactly because `E[count_l] = lambda_l` and the layer shares sum to `t`; it holds per record, per cell, and for any strength. The fog is divided by the active layer count so the total event rate and its count variance stay those of the single-layer law at the same total density.

`cell_base` is 14.0 µm for negative profiles and 7.0 µm for positive profiles. The positive microcontrast uses `w = 0.35` at `0.55 * cell_base`; negative profiles use `w = 0`.

Both components have unit variance, so `u_base` has unit variance and the existing law is unchanged. The profile layer curves do not reorder those delivered bases.

The expectation diagnostic replaces `count_l` by `lambda_l` and the field draws by their conditional means; it MUST return the target density.

Fog keeps the event rate alive at clear base. It is a density baseline, not a level offset: the same `f` is subtracted after the count, so tone is unchanged.

## Strength controls

Let `s` be public grain strength and `a` be the polarity's delivered amplitude scale (`2.0` for positive profiles, `1.0` for negative). The delivered constants map as:

```text
s_eff     = a * s
white_eff = white * s_eff
base_eff  = base * s_eff
q_eff     = q * s_eff^2
```

`Var(D) = q_eff (t + f) e / A`, so the count term's standard deviation scales with `sqrt(q_eff)` while the envelope's standard deviation scales with `white_eff`; both scale linearly with `s`. The taper and the per-record ratio are unchanged.

`s = 0` is the deterministic limit: every draw is replaced by its conditional mean, so the law returns the target density with no random field or residual count noise. `s = 1` leaves the polarity's delivered constants unchanged. The positive amplitude scale is part of that delivered law, not a public control.

A mapped event area below the f32 normal minimum resolves to the deterministic limit on both executors. Above the exact count-rate threshold the count deviation is normal and evaluated directly, so a small event area cannot overflow the rate or lose the mean.

A delivered cell whose size or coordinates are not finite and positive MUST fail before allocation.

## Field construction

One smoothstep-interpolated value-noise lattice uses a delivered base cell size of 14.0 µm for negative profiles and 7.0 µm for positive profiles. The field is multiplied by one fixed scale that makes it unit variance.

Positive profiles mix a second independent lattice at 0.55 times the cell size and 35% of the variance; negative profiles use one lattice.

The profile's density layers carry the multi-layer structure instead of an octave stack. Negative profiles use relative cell sizes 1.0, 0.5, and 0.3 of the 14.0 µm base.

The profile curves select the active layer count and each layer's density share. Positive profiles keep 1.0 on every layer. Their density curves weight a normal-density target toward the finest-index layer; a 0.3 cell at a 35 mm 1:1 crop would fall below one source pixel and read as uncorrelated noise.

On a 1:1 crop from a 35 mm frame at a 3.66 µm source pitch, the negative base places one lattice cell every 3.8 source pixels. Its adjacent-sample correlation is about 0.7 and falls below 0.1 by the fourth sample.

The positive base places one cell every 1.9 source pixels. Its adjacent-sample correlation is about 0.45–0.5 at lag 1 and near zero by lag 2. The positive texture is visibly finer while keeping a coherent cell above one source pixel at the delivered scale.

The microcontrast cell is 1.05 source pixels at the reference pitch. At 35% variance it sharpens the field without replacing the base as the dominant correlation length.

A cell size that would put every source pixel on one lattice phase is nudged by 1.37% before evaluation, so amplitude does not step when geometry lands on that aliasing case.

There is no Gaussian filter, high-pass, per-image standardization, or separate clean-detail path. The microcontrast component is part of the same conditional field and is not applied after readout.

The noise is coordinate-addressed: a 32-bit hash of seed, stream and global source pixel. The CPU reference and the device pass use the same hash, the same streams, the same cell size, and the same coordinates, so one seed yields one realization on both executors.

Stream numbering is fixed: the shared field uses stream `0`; record `c` of density layer `l` uses stream `1 + 4*l + c`. The corresponding count stream is `100 + 4*l + c`. A monochrome profile uses the first record's stream in every lane.

## Device execution

The device evaluates the same law in two passes. One dispatch writes the shared and per-record value-noise fields for every active density layer. The density law then splits the target, mixes the fields, draws each layer's count, and sums the layer densities.

The field pass is pointwise, so a band needs no filter halo; its transport window still includes one source cell of bilinear readout support.

The profile layer curve table is uploaded with the field request, so the resident carrier can execute the same layered law. Each active layer reads its relative cell size from the same engine constants on both executors.

The count is drawn by the same method on both executors: inversion below thirty events, PTRS above, up to the exact-rate threshold of `10^9` events per cell. Above that threshold both executors draw the same coordinate-addressed normal deviation.

A normal approximation below the threshold would match the mean and the variance but not the distribution, and the difference shows once the rate is small.

The unit-variance scale is a fixed constant of the smoothstep value-noise process, shared by both executors. The shader MUST NOT standardize a band or read image/field statistics.

Bands key their noise on global source coordinates. Because the field is pointwise, bands join without a field halo or seam; the source window's one-cell readout support is not a field halo. The pass MUST NOT require a device-side reduction or a synchronization point.

## Deterministic comparison

The implementation MUST provide a diagnostic expectation mode that replaces each draw with its conditional mean while retaining target preparation, readout, downstream settings, and precision. It is the expectation at the density boundary, not the seed-ensemble mean after nonlinear spectral or scanner operations.

Expectation mode MUST NOT mutate the user recipe or become an alternate production grain law. It exists to separate deterministic transfer from stochastic variation during acceptance.

## Numerical certification

Correlation length is a polarity-selected model constant anchored to film geometry. It MUST NOT depend on source dimensions, crop size, output size, or seed.

Certification MUST cover constant targets, graded targets, saturation, fractional readout translations, noninteger lattice-to-output ratios, film boundaries, and band splits. It MUST compare production readout with an independent footprint-integration oracle that does not reuse production helpers.

Certification MUST measure expected-density error, readout RMS, spatial covariance or power spectrum, and band-split agreement. It MUST state numerical budgets before evaluating a candidate.

At fixed target, geometry, seed, precision, and output footprints, full-frame, crop, tiled, and stage-split reads of one film MUST agree within the declared precision budget.

## Optical measurement and calibration

The field realization MUST NOT read statistics from the current image or normalize a realization to its measured RMS. Measurement is an external operation over fixed stimuli, fixed geometry, fixed receiver and aperture, fixed density interpretation, and a declared seed population.

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
2. **Layer realization:** profile layer curves, layer-target split, lattice geometry, two-component value noise, and record coupling.
3. **Count realization:** event area, layer fog share, and the conditional count.
4. **Readout:** film-coordinate resampling to the requested grid.
5. **Optical measurement:** independent transmission, aperture, density, and calibration records.
6. **Executor adapters:** CPU reference and GPU production implementations over the same law.

The CPU reference and GPU production executor MUST implement the same seed, stream, coordinate, and geometry semantics. They MAY differ in arithmetic precision and count-draw construction, and such differences MUST stay inside the count term.

No module MAY duplicate another module's film chemistry, optical measurement, parameter fit, or route selection. User surfaces MAY expose algorithm inputs, but they MUST NOT encode calibration evidence or infer stock calibration from an appearance label.

## Implementation gates

Implementation MUST proceed in gates:

1. Prove the density law and its mean on deterministic targets, including clear base, mid densities, and saturation.
2. Prove the field construction: layer-target sum, two-component field variance, polarity-selected base and microcontrast cells, and coordinate-addressed layer streams on both executors. The delivered positive field MUST report its adjacent-sample correlation lags and remain above the subpixel-noise case at the 35 mm 1:1 reference geometry with a 3.66 µm source pitch.
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
