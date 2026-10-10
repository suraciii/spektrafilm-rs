# Film grain V3 design

## Decision

V3 must retain the developed film-density boundary and the profile's three-sublayer response. It must realize conditional Poisson dye masses in global film coordinates. A normalized spatial operator must form the complete realization, including its subject-dependent expectation. Rectangular output footprints must integrate that field. The existing spectral scanner must receive the resulting CMY density.

The first candidate uses one Gaussian dye-support width shared by the layers and channels. Statistical event area controls count variance independently of this width. Independent CMY realizations remain the initial choice. The common width does not imply neutral grain or correlated channel samples.

The microscopic V1-derived support formula is a diagnostic alternative, not the production default. With resolved Portra 800 default controls it gives sigma about 0.38–1.15 micrometers. Resolving its narrowest support at half-sigma requires about 23 billion field cells for a 35 mm long-edge 3:2 frame. Tiling reduces memory but does not remove this sampling cost. This alternative must not determine the full-frame allocation policy.

The selected finite-resolution canvas must be certified on small regions before full-frame integration. Direct Gaussian/box integration defines its independent mathematical oracle. A spatial grid is a numerical representation, not a population of visible square grains. Full-frame performance and photographic acceptance are required before the candidate is offered as validated.

Explicit particle enumeration is not selected. It can require excessive event counts even without a dense canvas. A Gaussian residual approximation is not selected because it changes count tails and can require density clipping. Neither is a fallback for an expensive or visually unsuccessful candidate.

## Terms

A **density target** is the conditional developed CMY response after film curves and DIR. It is not an independent image to preserve downstream of formation.

A **field cell** is a numerical film region carrying aggregate conditional dye mass. It is not one visible grain or one silver-halide crystal.

**Dye support** is the normalized spatial operator applied to the dye distribution. Its width is expressed in film units, independently of statistical event area and field pitch.

An **output footprint** is the film region and normalized weighting read by an output pixel. The first candidate uses a rectangular box.

A **reference canvas** is the global discretization of the conditional field. Production tiles are storage partitions of this canvas, not separate random fields.

## Processing boundary

V3 must consume expected developed CMY density after the existing density curves and DIR processing. It must not resample exposure and repeat nonlinear chemistry inside the grain model.

For active V3, the pipeline must establish a full-film density-target grid independently of the requested crop and output upscale. This grid must use the loaded source dimensions, full-film pitch, existing exposure calculation, and existing non-grain film processing. Requested output resampling must happen in the field readout. V1 and V2 retain their existing ordering.

Auto exposure and spatial film processing must use the same full source for full-frame and cropped V3 renders. A crop must not substitute its boundary for the film boundary.

This ordering differs from the existing grain-off path when cropping or resizing is requested. The existing exact bypass must remain unchanged. Acceptance must therefore include the [deterministic expectation](#deterministic-comparison), rather than attributing all V3-versus-off differences to random grain.

V3 output must enter the existing scanner at the film-density boundary. Scanner illumination, dye and base spectra, color conversion, negative interpretation, gamut compression, lens blur, unsharp mask, and encoding remain scanner operations. V3 must not compose grain after encoding.

The [product specification](spec.md#initial-supported-condition) owns the supported conditions and bypass rules.

## Film geometry

Use the existing convention that `camera.film_format_mm` gives the full-film long edge. For source dimensions `W,H`, define pitch `q = 1000 * film_format_mm / max(W,H)` in micrometers. Film bounds are `[0,W*q] × [0,H*q]`. Source pixels occupy rectangles of width and height `q` with centers at `((x+0.5)*q,(y+0.5)*q)`.

Reuse existing crop rounding and slice normalization. Carry its resolved source rectangle `(x0,y0,w,h)` into V3. The requested region is `[x0*q,(x0+w)*q] × [y0*q,(y0+h)*q]`.

Use existing output dimension rounding. For actual output dimensions `Wo,Ho`, partition the requested region into rectangular output footprints. Derive each axis pitch from the region extent divided by its actual output dimension. Do not use nominal upscale after rounding.

Treat the developed source target as piecewise constant on its source rectangles. Allocate sublayer targets on that grid first. Transfer each absolute sublayer target to a field cell by exact area-weighted overlap with source rectangles. This conservative remapping must preserve constants and integrated target mass. It must not add source-center bilinear reconstruction followed by a second box average. The source rectangle is a declared reconstruction assumption, not a new dye-support filter.

Extend source edge targets constantly outside the full-film bounds. Reconstruction must not clamp at crop or tile edges. Additional field samples do not recover missing source information.

Field cells are square, with internal pitch `h` and signed global indices `(i,j)`. Cell `(i,j)` occupies `[i*h,(i+1)*h] × [j*h,(j+1)*h]`. The origin is the full-film top-left boundary. Crop bounds and output dimensions must not redefine this grid.

## Parameters

Resolve `particle_area_um2`, channel `particle_scale`, `particle_scale_sublayers`, `density_min`, and `uniformity` through existing runtime and stock resolution. These are model parameters, not measured Portra crystal geometry. Record the resolved values in acceptance. `rms_granularity` must not feed the sampler.

`v3_dye_support_um` gives Gaussian sigma in micrometers, with candidate default `8.0`. It must be finite and strictly positive. This empirical starting value must pass photographic and numerical acceptance before it is described as a validated default. It must not be derived from event area, output dimensions, or V1 blur controls.

Effective event area governs statistical strength. Support governs spatial covariance and mean detail transfer. Field pitch governs numerical accuracy. The implementation must keep these meanings distinct. At fixed targets and support, multiplying event area by a factor multiplies the continuum readout variance by that factor; it must not enlarge support. Changing support changes both covariance shape and mean transfer. A larger support must not be advertised as a free improvement in grain size.

Active areas and scales must be finite and strictly positive. Floors must be finite and nonnegative. Uniformity must be finite in `[0,1]`. Validate support, counts, geometry, and addressable allocation before rendering. Resource failure must be explicit. Do not silently coarsen sampling, remove layers, or approximate the count law to fit resources.

V3 always uses three profile sublayers. It ignores V1's `sublayers_active`, `n_sub_layers`, `blur`, `blur_dye_clouds_um`, `micro_structure`, `micro_sublayers`, `mult_usm_sigma`, and `mult_usm_amount`. It ignores V2 controls and `settings.use_fast_stats`. No separate chroma-correlation or numerical-resolution look control is selected.

## Sublayer target allocation

Use the profile resolved for the actual render, after model-backed curve reconstruction and selected film chemistry. Do not use stale sampled JSON arrays when the loader rebuilds them. The resolved profile must have finite nonnegative sublayer curves on the composite exposure axis. Each normalized composite density axis must be nondecreasing with positive range. Each channel must have positive total layer capacity.

For channel `c` and layer `l`, let `m_lc` be the maximum resolved, unnormalized layer density, `f_lc = m_lc / sum_l m_lc`, `b_lc = f_lc * density_min_c`, and `M_lc = m_lc + b_lc`. Skip zero-capacity layers. Effective event area is `a_lc = particle_area_um2 * particle_scale_c * particle_scale_sublayers_l`.

At source target `D_c`, obtain provisional nonnegative layer values `v_lc` with existing composite-to-layer interpolation brackets. Endpoint and duplicate-knot lookup must retain the existing right-bracket convention. These values provide relative allocation, not another tone curve.

Allocate `T_c = D_c + density_min_c` across layer capacities `M_lc`. Require `0 <= T_c <= sum_l M_lc`, allowing only documented arithmetic tolerance at endpoints. Fail on a materially unsupported target; do not silently clip it. Check the selected chemistry and DIR range before claiming recipe support.

Use iterative weighted capacity allocation:

1. Start with all positive-capacity layers unsaturated and remaining target `T_c`. Set weights to `v_lc + b_lc`.
2. If unsaturated weights sum to zero, replace them with the unsaturated capacity fractions. This replacement applies to the current iteration, not to a single multiplier equation with the original weights.
3. Distribute remaining target proportionally to the current weights. Fix each layer whose offered value reaches capacity at that capacity. Remove these layers and subtract their allocation from the remaining target.
4. Repeat on remaining layers, or commit the proportional allocation when no layer saturates. Handle zero target and total-capacity target directly.

At most three layers can saturate. Preserve `sum_l t_lc = D_c + density_min_c` within arithmetic tolerance. Check curve knots, intermediate targets, zero-weight cases, endpoints, selected chemistry, and DIR. Transfer these absolute targets conservatively to field cells as defined under film geometry.

## Conditional dye mass

For cell area `A=h*h` and remapped absolute layer target `t_lc`, define `p_lc=t_lc/M_lc` on closed endpoints `[0,1]` and `s_lc=1-uniformity_c*(1-1e-6)*p_lc`. The regularizer keeps `s` positive; it is not V1's endpoint probability clamp.

```text
lambda_lc = A*f_lc*p_lc / (a_lc*s_lc)
K_lc ~ Poisson(lambda_lc)
mass_lc = M_lc*a_lc*s_lc / f_lc
cell_absolute_density_lc = mass_lc*K_lc / A
```

At `p=0`, use zero count. At `p=1`, sample the stated law. Use an exact Poisson algorithm. Rounded normal count approximations must not replace it. Poisson thinning gives this law from V1's interior Poisson-then-binomial distribution without requiring its random-call order.

The local expectation is `t_lc`. Unformed cell variance is `M_lc*M_lc*a_lc*p_lc*s_lc/(A*f_lc)`. Event mass is independent of cell area; intensity scales with area. Raw cell-density variance grows under refinement while formed readout statistics must converge.

Address streams by the full 64-bit seed, fixed V3 channel/layer tags, and global signed cell coordinates. Output indices, crop dimensions, tile size, traversal order, and thread scheduling must not enter the address. Keep rejection state local to a cell. Reuse the tagged RNG boundary where possible; do not allocate a full MT19937 state per cell to reproduce V1 order.

## Dye formation

The continuum reference is piecewise constant cell mass density formed by a separable Gaussian of sigma `v3_dye_support_um`. Truncate each axis to exactly `[-4*sigma,4*sigma]` and normalize it to unit integral. Production and independent oracles must use this same declared truncation. It must not vary with field pitch.

All layers and channels use this support. Sum the three stochastic absolute layer densities per channel before filtering; linearity makes this equivalent to filtering each layer separately. This reduces production spatial filtering to three channels. Counts still require their individual layer parameters and streams.

Production may evaluate the formed field as cell averages with separable finite filters. A one-dimensional coefficient between source cell `S_i` and destination cell `C_j` must equal `integral_Cj integral_Si g_sigma(x-z) dz dx / h`, with the normalized truncated Gaussian `g_sigma`. These coefficients map cell density to formed cell-average density. They must be nonnegative and sum to one over source cells, including required halo cells. CDF-based integration may compute them; point-sampled Gaussian taps must not substitute silently.

Filter the complete stochastic density, or filter its exact expectation and centered counts with the identical operator. Do not preserve a clean mean around that operator. Sum layers, form each channel, and subtract `density_min_c` once. The offset is not the spectral film base. Do not clamp the realization after subtraction.

Uniform expected readout is `D_c`. Nonuniform detail follows source reconstruction, layer allocation, conservative remapping, dye support, and footprint integration. The same support acts on mean and realization. Independent grain size and sharpness improvements are not promised by this model.

Tiles must include field-support and output-footprint halos. Halo values use global coordinates and full-film targets. Tile edges must not reflect, clamp, or truncate the field. Process layers into one channel accumulator and process channels sequentially where practical. A materialized full-film field is not required.

## Readout and detail

Treat formed cell averages as piecewise constant for the production readout. Integrate their exact fractional overlap with each output rectangle and divide by footprint area. Cover subcell footprints and noninteger ratios. This reconstruction is a numerical approximation to the continuum reference; certification must bound it independently.

The scanner receives this averaged CMY density. This is a density-domain approximation, not optical aperture integration. At one wavelength, equal-area densities 0 and 2 transmit an average of `0.505`; scanning their average density transmits `0.1`. Microscopic density variation can also bias uniform-region transmission even when a large output box suppresses visible noise.

Before full-frame GUI integration, exercise a spectral aperture oracle on small representative realizations. At each spatial sample, combine film base and CMY spectra into wavelength-dependent density, convert to transmission, integrate over the same box, and integrate with the actual scanner illuminant and response. Then apply the actual scan-output interpretation. Do not exponentiate CMY coefficients independently.

Compare the oracle and density-average candidate on low/middle/high patches, edges, narrow lines, and representative grain with fixed settings. Report linear capture and final positive RGB errors, including seed-ensemble tone and chroma shifts. A raw wavelength error must not be reported as final RGB error. Verify integration convergence independently.

Density averaging may proceed to photographic acceptance only when the oracle comparison establishes an acceptable error scope for the claimed condition. If it fails, the film-density output boundary must be reconsidered with a concrete spectral-readout change before full-frame integration; do not conceal the failure by retuning grain strength. This gate does not certify physical Portra equivalence.

Dye support is formation; the box is sampling; scanner blur is downstream optics. In scale comparisons, adjust existing scanner pixel controls to retain declared film-space widths, or disable them and record that choice. The scanner's scalar pixel sigma cannot exactly describe anisotropic film footprints; restrict such equivalence checks to equal pitches or zero scanner blur.

## Deterministic comparison

Provide a diagnostic V3 expectation render by replacing each cell's random count with its conditional mean `lambda`. Keep full-source development, layer allocation, remapping, formation, readout, scanner settings, and precision identical to the stochastic V3 render. This is the expectation at the CMY boundary; nonlinear scanning means it is not the seed-ensemble mean of final RGB.

Compare existing grain off, V1, V3 expectation, and stochastic V3. Isolate V1 blur, microstructure, and density sharpening. Separate changed development/resampling from deterministic formation transfer, and separate both from stochastic grain. Do not infer improvement solely from V3 versus the old off path.

At native output dimensions, measure reconstruction-only response with support disabled in a mathematical diagnostic, without presenting this as a supported production mode. This identifies remapping loss before the dye response. Narrow lines must not gain an independent clean-detail bypass to improve this comparison.

## Numerical certification

Start the candidate with internal pitch `h = sigma/2`, anchored to the full film. Do not additionally refine it from source dimensions or output sampling. Source-to-cell transfer integrates source rectangles; source resolution is not a hidden field-grid control. Certification may require a smaller globally fixed pitch for the candidate.

With default sigma 8 micrometers, the initial pitch is 4 micrometers. A 35 mm long-edge 3:2 frame has about 51 million cells and 459 million layer counts, before halos. A single f32 plane is about 204 MB. These estimates are workload bounds, not a timing measurement or a promise of interactive operation.

Run `h,h/2,h/4` checks on small representative film regions, not three full-frame microscopic canvases. Hold target surface, model parameters, and output footprints fixed. Cover low/middle/high targets, slanted edges, narrow lines, fractional translations, subcell boxes, noninteger ratios, and film boundaries.

Check implementation self-consistency with count moments and its discrete operator. For `H_jli`, the readout density weight of one cell's mass:

```text
E[readout_j] = sum_li mass_li*lambda_li*H_jli - density_min
Cov(readout_j,readout_k) = sum_li mass_li^2*lambda_li*H_jli*H_kli
```

Also use an independent Gaussian/rectangle integration oracle for continuum impulse, edge, line response, and covariance at nonzero separations. The oracle must not reuse production filter or overlap helpers. A stable wrong-width kernel must fail this comparison. For uniform regions, the continuum covariance is the sum of `v_lc * integral H_j(z)*H_k(z) dz`, where `v_lc=M_lc*M_lc*a_lc*p_lc*s_lc/f_lc` and `H_j(z)=integral w_j(x)*g_sigma(x-z) dx`.

For successive refinement and independent-oracle comparisons, expected CMY error must be at most `1e-3` density units. RMS and spatial covariance error must be at most `1%` relative to nonzero oracle values; use an absolute `1e-6` bound for RMS below `1e-6` and `1e-8` density-squared for covariance magnitude below `1e-8`. Check edge and line response with the expected-density bound. These are engineering tolerances, not Portra measurements. Failure requires finer sampling or an explicit change to the numerical algorithm, not a wider visual claim.

Measured on the certification patterns (slanted edge, one- and two-pixel lines, flat dark and bright areas, one output pixel per source pixel): at `sigma/2` the worst expected-density error is `3.8e-3` and the worst covariance error is `6.7%`; at `sigma/4` they are `2.3e-6` and `0.57%`; at `sigma/8` they are `2.3e-6` (oracle-limited) and `0.15%`.

Both gates are therefore met at `sigma/4` and finer. The production candidate `sigma/2` carries a declared approximation instead: its expected-density error stays within `5e-3` and its covariance error within `10%` of the continuum reference, dominated by output-footprint granularity rather than by count statistics. Certification, and the acceptance record, must state the pitch actually rendered; a finer globally fixed pitch is admissible whenever the full-frame workload allows it.

Validate exact Poisson sampling with probability/CDF checks at representative zero, small, transition, and large intensities, in addition to mean and variance. Use declared statistical intervals and sufficient samples. At least 32 seeds are required for field checks; increase samples when uncertainty cannot distinguish an error. Same-moment non-Poisson counts must fail. A single-seed difference under grid refinement does not establish drift.

At fixed pitch, seed, target surface, and precision, overlapping CMY full-frame/crop and single-tile/multi-tile reads must agree within `1e-5` for f32 and `1e-10` for f64. Require identical output footprints for this equality. Crop dimension rounding can change footprints; such comparisons must instead declare the distinct sampling. Repeated identical execution must be deterministic.

## Runtime integration

`params/grain.rs` owns selection and support sigma. Metadata, CLI edits, presets, GUI controls, and state serialization must use that contract. Replace the unimplemented scale proposal with the micrometer control everywhere. Ignored V1/V2 controls must not appear as active V3 controls.

Geometry must carry full-film bounds, crop origin, target grid, and actual output footprints. The pipeline and film stage own full-source development. The grain module owns allocation, count realization, formation, and area readout. Scanner interpretation and encoding retain their owners.

Apply inactive, preview, spatial-deactivation, stochastic-deactivation, and LUT bypass before active-V3 parameter resolution and allocation. These specific modes must not allocate the field. Diagnostic taps, timing, and output dumps must continue to exercise active V3; a generic debug flag must not disable it. The expectation diagnostic must not mutate the user's recipe.

Active V3 must disable resident WGPU execution and use CPU. Report the effective backend in capability and dry-run output. Unsupported conditions and resources must fail before publication. Scan and Export must share the resolved model.

For `positive_scan`, spectral capture and lens blur precede calibrated negative endpoint interpretation, followed by gamut compression, unsharp mask, and encoding. For `direct_scan`, preserve the existing direct order: capture, gamut compression, lens blur, unsharp mask, and encoding, without inversion. Convert-film remains outside active V3 support.

CMY crop certification does not by itself establish final RGB crop equality. With active scanner blur or unsharp mask, evaluate their spatial operations on the corresponding full-film output grid and crop afterward. An implementation may use bounded halos only if it proves equivalent scanner boundary behavior. Otherwise use full-frame scanner processing. If crop output rounding changes footprint alignment, report this sampling difference rather than claim pixel equality.

## Implementation gates

First prove layer allocation, reconstruction, count law, independent formation/readout response, and optical approximation on small regions. Then compare portraits and other photographic regions using the deterministic expectation and real-scan reference. Select support and event area together from that evidence; the candidate defaults do not establish equivalence.

Before declaring the first condition supported, render the actual full frame through CLI and GUI Scan/Export. Record machine, source/output dimensions, pitch, peak resident memory, stage times, and cancellation behavior. Tile or cache optimizations must retain certified output. If the full-frame workload is unacceptable, revise execution and re-certify before release; do not silently fall back to another model or reduce the promised frame scope.

Permanent tests must cover allocation including zero weights, bypass precedence, support independence from event area, global crop/tile identity, fractional boxes, supported routes, and state round trips. Numerical and photographic evidence must identify the real entry point and precision. Synthetic proof alone must not be reported as visual acceptance.

## Evidence and limits

Normally processed color-negative film retains image dyes after silver removal. Kodak distinguishes subjective graininess from measured granularity. Conventional diffuse RMS uses a 48-micrometer diameter aperture at net density 1.00. Output-pixel RMS is not that measurement.

Portra 800's Print Grain Index cannot be compared with density RMS. Repository granularity presets are not verified Kodak density-RMS values. Independent CMY fields and Gaussian support are initial hypotheses, not identified causes of dirty color or a calibrated emulsion model.

The field boundary and exact Poisson mass law are mathematical choices. The 8-micrometer support is an empirical candidate intended to be resolvable at the comparison scale. It may still blur detail excessively or produce the wrong texture. Photographic acceptance must reject that outcome rather than adding a clean-detail branch. Unknown reference provenance, optical-readout adequacy, and supported output range remain explicit acceptance limits.

## References

- [V3 product contract](spec.md)
- [Film grain architecture](../design.md)
- [V1 contract](../v1/spec.md)
- [V2 contract](../v2/spec.md)
- [Dehancer investigation](../../../docs/dehancer-grain-reverse-engineering.md)
- Kodak, [The Essential Reference Guide for Filmmakers](https://www.kodak.com/content/products-brochures/Film/kodak-essential-reference-guide-for-filmmakers.pdf), printed pages 31 and 56–57.
- Kodak Alaris, [Portra 800 E-4040](https://kodakprofessional.com/sites/default/files/2025-07/e4040.pdf), printed page 3.
- Newson et al., [Realistic Film Grain Rendering](https://www.ipol.im/pub/art/2017/192/), sections 2–4.
- Last and Penrose, [Lectures on the Poisson Process](https://stoch.math.kit.edu/img/Last/lastpenrose2017.pdf), sections 5.3 and 15.4.
