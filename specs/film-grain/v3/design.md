# Film grain V3 design

## Decision

V3 must retain the developed film-density boundary and the profile's three-sublayer response. It must realize conditional Poisson dye masses in global film coordinates. A normalized spatial operator must form the complete realization, including its subject-dependent expectation. Rectangular output footprints must integrate that field. The next existing spectral stage must receive the resulting CMY density.

V3 uses one Gaussian dye-support width shared by the layers and channels. Statistical event area controls count variance independently of this width. The CMY realizations are independent. The common width does not imply neutral grain or correlated channel samples. A raw, unprocessed negative scan shows weak inter-record grain correlation, consistent with this choice; see [evidence and limits](#evidence-and-limits).

The microscopic V1-derived support formula is a diagnostic alternative, not the production default. With resolved Portra 800 default controls it gives sigma about 0.38–1.15 micrometers. Resolving its narrowest support at half-sigma requires about 23 billion field cells for a 36 mm long-edge 3:2 frame. Tiling reduces memory but does not remove this sampling cost. This alternative must not determine the full-frame allocation policy.

The finite-resolution canvas must pass certification on small regions before full-frame integration. Direct Gaussian/box integration defines its independent mathematical oracle. A spatial grid is a numerical representation, not a population of visible square grains. Full-frame performance and photographic acceptance are required before the release is offered as validated.

Explicit particle enumeration is not selected. It can require excessive event counts even without a dense canvas. A Gaussian residual approximation is not selected because it changes count tails and can require density clipping. Neither is a fallback for an expensive or visually unsuccessful model.

## Terms

A **density target** is the conditional developed CMY response after film curves and DIR. It is not an independent image to preserve downstream of formation.

A **field cell** is a numerical film region carrying aggregate conditional dye mass. It is not one visible grain or one silver-halide crystal.

**Dye support** is the normalized spatial operator applied to the dye distribution. Its width is expressed in film units, independently of statistical event area and field pitch.

An **output footprint** is the film region and normalized weighting read by an output pixel. V3 uses a rectangular box.

A **reference canvas** is the global discretization of the conditional field. Production tiles are storage partitions of this canvas, not separate random fields.

## Processing boundary

V3 must consume expected developed CMY density after the existing density curves and DIR processing. It must not resample exposure and repeat nonlinear chemistry inside the grain model.

For active V3, the pipeline must establish a full-film density-target grid independently of the requested crop and output upscale. This grid must use the loaded source dimensions, full-film pitch, existing exposure calculation, and existing non-grain film processing. Requested output resampling must happen in the field readout. V1 and V2 retain their existing ordering.

Auto exposure and spatial film processing must use the same full source for full-frame and cropped V3 renders. A crop must not substitute its boundary for the film boundary.

This ordering differs from the existing grain-off path when cropping or resizing is requested. The existing exact bypass must remain unchanged. Acceptance must therefore include the [deterministic expectation](#deterministic-comparison), rather than attributing all V3-versus-off differences to random grain.

V3 output must enter the next existing stage at the film-density boundary. On the print route, this stage is the enlarger print exposure. On the film-scan route, it is the film scanner. Enlarger illumination, filtration, diffusion filter, print paper, print development, scanner illumination, dye and base spectra, color conversion, negative interpretation, gamut compression, lens blur, unsharp mask, and encoding keep their existing owners. V3 must not compose grain after encoding.

The [product specification](spec.md#supported-condition) owns the supported conditions and bypass rules.

## Film geometry

`camera.film_format_mm` owns the full-source film long edge. The runtime parameter contract must validate it as finite and strictly positive. V3 must consume its resolved value without another format range or whitelist. The long edge is the exposed image dimension, not a roll-width designation such as 35 mm or the stock name 120. The source aspect ratio determines the other film dimension. Supporting a numerical long edge does not expand film chemistry or usage support.

Use the existing convention that `camera.film_format_mm` gives the full-film long edge. For source dimensions `W,H`, define pitch `q = 1000 * film_format_mm / max(W,H)` in micrometers. Film bounds are `[0,W*q] × [0,H*q]`. Source pixels occupy rectangles of width and height `q` with centers at `((x+0.5)*q,(y+0.5)*q)`.

Changing `camera.film_format_mm` changes the physical film extent, source-pixel pitch, and output-footprint size in film units. The requested crop and resize continue to determine raster dimensions. Format changes must keep dye support, event area, field pitch, stock resolution, and random-stream addressing unchanged. At fixed output dimensions, a larger format reads larger film footprints. The model must integrate those footprints without a format-dependent strength correction.

At fixed aspect ratio and field pitch, the full-film cell count grows approximately as the square of the long edge. Band partitioning bounds working storage but does not remove this workload. Resource checks must use the resolved geometry and the existing allocation and coordinate limits. A resource error must identify the exceeded limit and an applicable remedy before output publication. The implementation must not silently coarsen the field or change the model to fit a larger format.

Reuse existing crop rounding and slice normalization. Carry its resolved source rectangle `(x0,y0,w,h)` into V3. The requested region is `[x0*q,(x0+w)*q] × [y0*q,(y0+h)*q]`.

Use existing output dimension rounding. For actual output dimensions `Wo,Ho`, partition the requested region into rectangular output footprints. Derive each axis pitch from the region extent divided by its actual output dimension. Do not use nominal upscale after rounding.

When downstream spatial processing is active, extend the requested output lattice by the minimum whole output pixels that cover the full film. For source-axis length `N`, source crop `[a,a+b)`, and rounded output length `M`, retain pitch `p=b*q/M`. Prepend `ceil(a*M/b)` samples and append `ceil((N-a-b)*M/b)` samples, using integer ceiling division. The enclosing grid may extend less than one output footprint beyond each film edge. Extend source targets constantly there, while retaining independently addressed field cells. Apply downstream boundary rules at this enclosing grid's edges. Extract the requested integer output rectangle only after those stages. Aligned grids have the same boundary as an ordinary full-frame render; unaligned grids retain their requested pitch and phase.

After V3 readout, scalar physical optical controls use the actual output pitch on the original source's long axis, with the horizontal axis selected for a square source. Scanner blur and unsharp-mask radii remain pixel controls. If rounded output pitches differ between axes, this scalar optical approximation is not isotropic in film space. Equal-film-scale comparisons require equal pitches or disabled downstream spatial effects.

Treat the developed source target as piecewise constant on its source rectangles. Allocate sublayer targets on that grid first. Transfer each absolute sublayer target to a field cell by exact area-weighted overlap with source rectangles. This conservative remapping must preserve constants and integrated target mass. It must not add source-center bilinear reconstruction followed by a second box average. The source rectangle is a declared reconstruction assumption, not a new dye-support filter.

Extend source edge targets constantly outside the full-film bounds. Reconstruction must not clamp at crop or tile edges. Additional field samples do not recover missing source information.

Field cells are square, with internal pitch `h` and signed global indices `(i,j)`. Cell `(i,j)` occupies `[i*h,(i+1)*h] × [j*h,(j+1)*h]`. The origin is the full-film top-left boundary. Crop bounds and output dimensions must not redefine this grid.

## Parameters

Resolve `rms_granularity`, `particle_scale_sublayers`, `density_min`, and `uniformity` through existing runtime and stock resolution. These are model parameters, not measured crystal geometry. Record the resolved values in acceptance.

The stock granularity preset sets the channel event-area scale `k_c`. For preset value `G_c`, `k_c = (G_c / 6)^2`. The anchor `6` is the preset of the reference stock `kodak_portra_800`. A non-finite or non-positive `G_c` gives `k_c = 1`. Readout variance is proportional to event area, so at fixed density and support the readout RMS is proportional to `G_c`. The preset is a ratio only. Its absolute value is not Kodak density RMS, and it does not enter the sampler by another path.

`k_c` is the only source of channel-to-channel strength differences. V3 must not read `particle_scale`. That V1 control defaults to `[1.6, 1.6, 3.2]` and already makes the blue record coarser. A blue-heavy granularity preset would count the same difference twice; see [evidence and limits](#evidence-and-limits). Monochrome resolution equalizes `rms_granularity` across channels and therefore `k_c`.

`v3_event_area_um2` gives the base event area in square micrometers, with default `1.28`. It must be finite and strictly positive. With the reference stock, `k_c = 1`, and the default is the validated absolute event area of every channel. V3 must not read `particle_area_um2`, which keeps its V1 meaning and default.

`v3_dye_support_um` gives Gaussian sigma in micrometers, with default `3.5`. It must be finite and strictly positive. It must not be derived from event area, output dimensions, or V1 blur controls.

Both defaults were selected together on the reference condition. Through Kodak's 48-micrometer aperture definition, they give the reference stock's green record a diffuse RMS granularity of about 9 at middle density. At the 6.35-micrometer sampling of a real 35 mm UltraMax 400 scan, their mean grain-clump size is 6.9 micrometers; the scan measures 6.7–7.0. The defaults are validated for the reference condition only.

Effective event area governs statistical strength. Support governs spatial covariance and mean detail transfer. Field pitch governs numerical accuracy. The implementation must keep these meanings distinct. At fixed targets and support, multiplying event area by a factor multiplies the continuum readout variance by that factor; it must not enlarge support. Changing support changes both covariance shape and mean transfer. A larger support must not be advertised as a free improvement in grain size.

Active areas and scales must be finite and strictly positive. Floors must be finite and nonnegative. Uniformity must be finite in `[0,1]`. Validate support, counts, geometry, and addressable allocation before rendering. Resource failure must be explicit. Do not silently coarsen sampling, remove layers, or approximate the count law to fit resources.

V3 always uses three profile sublayers. It ignores V1's `particle_area_um2`, `particle_scale`, `sublayers_active`, `n_sub_layers`, `blur`, `blur_dye_clouds_um`, `micro_structure`, `micro_sublayers`, `mult_usm_sigma`, and `mult_usm_amount`. It ignores V2 controls and `settings.use_fast_stats`. No chroma-correlation, chroma-suppression, or numerical-resolution look control is selected.

## Sublayer target allocation

Use the profile resolved for the actual render, after model-backed curve reconstruction and selected film chemistry. Do not use stale sampled JSON arrays when the loader rebuilds them. The resolved profile must have finite nonnegative sublayer curves on the composite exposure axis. Each normalized composite density axis must be nondecreasing with positive range. Each channel must have positive total layer capacity.

For channel `c` and layer `l`, let `m_lc` be the maximum resolved, unnormalized layer density, `f_lc = m_lc / sum_l m_lc`, `b_lc = f_lc * density_min_c`, and `M_lc = m_lc + b_lc`. Skip zero-capacity layers. Effective event area is `a_lc = v3_event_area_um2 * k_c * particle_scale_sublayers_l`, with `k_c` from [parameters](#parameters).

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

Production packs the two signed 32-bit cell coordinates into one 64-bit address and applies a bijective integer mixer. Both coordinates, including formation halos, must fit this domain. This address mapping replaces the prototype mapping, so the same prototype seed produces a different realization. Counts remain conditionally independent with the same model parameters and probability law.

## Dye formation

The continuum reference is piecewise constant cell mass density formed by a separable Gaussian of sigma `v3_dye_support_um`. Truncate each axis to exactly `[-4*sigma,4*sigma]` and normalize it to unit integral. Production and independent oracles must use this same declared truncation. It must not vary with field pitch.

All layers and channels use this support. Sum the three stochastic absolute layer densities per channel before filtering; linearity makes this equivalent to filtering each layer separately. This reduces production spatial filtering to three channels. Counts still require their individual layer parameters and streams.

Production may evaluate the formed field as cell averages with separable finite filters. A one-dimensional coefficient between source cell `S_i` and destination cell `C_j` must equal `integral_Cj integral_Si g_sigma(x-z) dz dx / h`, with the normalized truncated Gaussian `g_sigma`. These coefficients map cell density to formed cell-average density. They must be nonnegative and sum to one over source cells, including required halo cells. CDF-based integration may compute them; point-sampled Gaussian taps must not substitute silently.

Filter the complete stochastic density, or filter its exact expectation and centered counts with the identical operator. Do not preserve a clean mean around that operator. Sum layers, form each channel, and subtract `density_min_c` once. The offset is not the spectral film base. Do not clamp the realization after subtraction.

Uniform expected readout is `D_c`. Nonuniform detail follows source reconstruction, layer allocation, conservative remapping, dye support, and footprint integration. The same support acts on mean and realization. Independent grain size and sharpness improvements are not promised by this model.

Tiles must include field-support and output-footprint halos. Halo values use global coordinates and full-film targets. Tile edges must not reflect, clamp, or truncate the field. Process layers into one channel accumulator and process channels sequentially where practical. A materialized full-film field is not required.

Production partitions the output into horizontal bands of at most 128 output rows. Each band holds at most 2^24 field cells, including halos, and at most 2^24 source pixels. A coarse output pitch reads many cells per output row, so a band that would exceed the budget halves its row count until it fits. Band height only partitions storage; it must not change any cell, count, or readout value. Only a single output row above the budget fails, with an actionable error.

Validate the coordinate domain, output storage arithmetic, stencil counts, and a conservative one-row resource bound before allocating stencils. Validate finite positive effective event areas and representable conditional mass and rates before generating counts. The maximum conditional rate is `2^52`; a proposed count outside the exact f64 integer range below `2^53` must fail explicitly instead of saturating. These resource limits must not change the count law or silently coarsen the field.

## Readout and detail

Treat formed cell averages as piecewise constant for the production readout. Integrate their exact fractional overlap with each output rectangle and divide by footprint area. Cover subcell footprints and noninteger ratios. This reconstruction is a numerical approximation to the continuum reference; certification must bound it independently.

The next spectral stage receives this averaged CMY density. This is a density-domain approximation, not optical aperture integration. At one wavelength, equal-area densities 0 and 2 transmit an average of `0.505`; their average density transmits `0.1`. Microscopic density variation can also bias uniform-region transmission even when a large output box suppresses visible noise.

Each supported route must pass a spectral aperture oracle on small representative realizations before photographic acceptance on that route. The oracle reads the same realization at about half the field pitch or finer. At each fine sample, it combines film base and CMY spectra into wavelength-dependent density and integrates its transmission with the actual downstream spectral weights: enlarger illuminant, filtration, and paper sensitivity on the print route, or scanner illuminant and response on the film-scan route. It averages this linear exposure over each output box. Both the oracle exposure and the production exposure then pass through the actual downstream chain. Do not exponentiate CMY coefficients independently.

Compare the oracle and the density-average readout on low, middle, and high patches, an edge, narrow lines, and representative grain with fixed settings. Report the exposure error and the final positive RGB error, including tone and chroma bias over the patch and the change in grain RMS. A raw wavelength error must not be reported as final RGB error. Confirm that the fine and production reads see the same field: the box average of the fine CMY read must equal the production CMY read.

The density-average readout is acceptable on a route when, at the comparison pitch, the patch mean bias is at most 0.5 of an 8-bit code per channel, the grain RMS changes by at most 2%, and the averaged edge and line profiles differ by at most 2 codes. If the gate fails, reconsider the film-density output boundary with a concrete spectral-readout change. Do not conceal the failure by retuning grain strength. This gate does not certify physical Portra equivalence.

Dye support is formation; the box is sampling; enlarger diffusion and scanner blur are downstream optics. In scale comparisons, adjust existing downstream pixel controls to keep declared film-space widths, or disable them and record that choice. A scalar pixel sigma cannot exactly describe anisotropic film footprints; restrict such equivalence checks to equal pitches or zero downstream blur.

## Deterministic comparison

Provide a diagnostic V3 expectation render by replacing each cell's random count with its conditional mean `lambda`. Keep full-source development, layer allocation, remapping, formation, readout, scanner settings, and precision identical to the stochastic V3 render. This is the expectation at the CMY boundary; nonlinear scanning means it is not the seed-ensemble mean of final RGB.

Compare existing grain off, V1, V3 expectation, and stochastic V3. Isolate V1 blur, microstructure, and density sharpening. Separate changed development/resampling from deterministic formation transfer, and separate both from stochastic grain. Do not infer improvement solely from V3 versus the old off path.

At native output dimensions, measure reconstruction-only response with support disabled in a mathematical diagnostic, without presenting this as a supported production mode. This identifies remapping loss before the dye response. Narrow lines must not gain an independent clean-detail bypass to improve this comparison.

## Numerical certification

The production field pitch is `h = sigma/2`, anchored to the full film. Do not additionally refine it from source dimensions or output sampling. Source-to-cell transfer integrates source rectangles; source resolution is not a hidden field-grid control. A finer globally fixed pitch is admissible when the full-frame workload allows it.

With the default sigma of 3.5 micrometers, the pitch is 1.75 micrometers. A 36 mm long-edge 3:2 frame has about 282 million cells and 2.5 billion layer counts, before halos. A single f32 plane is about 1.1 GB. These estimates are workload bounds, not a timing measurement or a promise of interactive operation.

Run `h,h/2,h/4` checks on small representative film regions, not three full-frame microscopic canvases. Hold target surface, model parameters, and output footprints fixed. Cover low/middle/high targets, slanted edges, narrow lines, fractional translations, subcell boxes, noninteger ratios, and film boundaries.

Format coverage and full-frame evidence must follow the [film-format acceptance procedure](spec.md#film-format-acceptance). The numerical tolerances in this section apply at the physical footprints used by each probe.

Check implementation self-consistency with count moments and its discrete operator. For `H_jli`, the readout density weight of one cell's mass:

```text
E[readout_j] = sum_li mass_li*lambda_li*H_jli - density_min
Cov(readout_j,readout_k) = sum_li mass_li^2*lambda_li*H_jli*H_kli
```

Also use an independent Gaussian/rectangle integration oracle for continuum impulse, edge, line response, and covariance at nonzero separations. The oracle must not reuse production filter or overlap helpers. A stable wrong-width kernel must fail this comparison. For uniform regions, the continuum covariance is the sum of `v_lc * integral H_j(z)*H_k(z) dz`, where `v_lc=M_lc*M_lc*a_lc*p_lc*s_lc/f_lc` and `H_j(z)=integral w_j(x)*g_sigma(x-z) dx`.

For successive refinement and independent-oracle comparisons, the certified pitch must hold the expected CMY error to at most `1e-3` density units, and the RMS and spatial covariance error to at most `1%`. Express covariance error at every lag relative to the oracle's zero-lag variance, because tail covariances approach zero. Use an absolute `1e-6` bound for RMS below `1e-6`. Check edge and line response with the expected-density bound. These are engineering tolerances, not Portra measurements. Failure requires finer sampling or an explicit change to the numerical algorithm, not a wider visual claim.

The production pitch `sigma/2` carries a declared approximation instead. Its expected-density error must stay within `5e-3`, and its covariance error within `10%` of the zero-lag variance. This error comes from output-footprint granularity rather than from count statistics.

With sigma 3.5 micrometers and a 6-micrometer output footprint, the production pitch has a zero-lag variance error of 4.9%, the `sigma/4` pitch 1.2%, and the `sigma/8` pitch 0.38%. The certified pitch is therefore `sigma/8`. The production readout variance is about 5% below the continuum reference; the validated event area already includes this deficit. Certification and the acceptance record must state the pitch actually rendered.

Validate exact Poisson sampling with probability/CDF checks at representative zero, small, transition, and large intensities, in addition to mean and variance. Use declared statistical intervals and sufficient samples. At least 32 seeds are required for field checks; increase samples when uncertainty cannot distinguish an error. Same-moment non-Poisson counts must fail. A single-seed difference under grid refinement does not establish drift.

At fixed pitch, seed, target surface, and precision, overlapping CMY full-frame/crop and single-tile/multi-tile reads must agree within `1e-5` for f32 and `1e-10` for f64. Require identical output footprints for this equality. Crop dimension rounding can change footprints; such comparisons must instead declare the distinct sampling. Repeated identical execution must be deterministic.

## Runtime integration

`params/grain.rs` owns selection, support sigma, and the V3 event area. Metadata, CLI edits, presets, GUI controls, and state serialization must use that contract. Ignored V1/V2 controls, including `particle_area_um2`, must not appear as active V3 controls.

Geometry must carry full-film bounds, crop origin, target grid, and actual output footprints. The pipeline and film stage own full-source development. The grain module owns allocation, count realization, formation, and area readout. Printing, scanner interpretation, and encoding keep their owners.

Apply inactive, preview, spatial-deactivation, stochastic-deactivation, and LUT bypass before active-V3 parameter resolution and allocation. These specific modes must not allocate the field. Diagnostic taps, timing, and output dumps must continue to exercise active V3; a generic debug flag must not disable it. The expectation diagnostic must not mutate the user's recipe.

Active V3 must disable resident WGPU execution and use CPU. Report the effective backend in capability and dry-run output. Unsupported conditions and resources must fail before publication. Scan and Export must share the resolved model.

On the print route, the enlarger exposes the print paper from the V3 density, followed by the existing optional enlarger diffusion filter, print development, print scan, and encoding. On the film-scan route with `positive_scan`, spectral capture and lens blur precede calibrated negative endpoint interpretation, followed by gamut compression, unsharp mask, and encoding. With `direct_scan`, keep the existing direct order: capture, gamut compression, lens blur, unsharp mask, and encoding, without inversion. Convert-film remains outside active V3 support.

CMY crop certification does not by itself establish final RGB crop equality. With an active downstream spatial operation, such as the enlarger diffusion filter, scanner blur, or unsharp mask, evaluate it on the corresponding full-film output grid and crop afterward. An implementation may use bounded halos only if it proves equivalent boundary behavior. Otherwise use full-frame downstream processing. If crop output rounding changes footprint alignment, report this sampling difference rather than claim pixel equality.

## Implementation gates

First prove layer allocation, reconstruction, count law, independent formation/readout response, and optical approximation on small regions. Then compare portraits and other photographic regions using the deterministic expectation and real-scan reference. Support and event area are selected together from that evidence; see [parameters](#parameters).

Before declaring the reference condition supported, render the actual full frame through CLI and GUI Scan/Export. Record machine, source/output dimensions, pitch, peak resident memory, stage times, and cancellation behavior. Tile or cache optimizations must keep certified output. If the full-frame workload is unacceptable, revise execution and re-certify before release; do not silently fall back to another model or reduce the promised frame scope.

Permanent tests must cover allocation including zero weights, bypass precedence, support independence from event area, global crop/tile identity, fractional boxes, supported routes, the unsupported cine usage, and state round trips. The covariance certification test must use the default support and the zero-lag-variance error measure. Numerical and photographic evidence must identify the real entry point and precision. Synthetic proof alone must not be reported as visual acceptance.

The format change must retain the existing 36 mm realization for the same source, seed, recipe, precision, and implementation. Permanent regression tests must cover the default 35 mm recipe, an arbitrary positive non-36 mm long edge, format persistence, invalid format values, unchanged output dimensions, and crop/tile identity at both smaller and larger formats. Resource-limit checks must exercise an exceeded limit without allocating the oversized canvas.


## Evidence and limits

Normally processed color-negative film retains image dyes after silver removal. Kodak distinguishes subjective graininess from measured granularity. Conventional diffuse RMS uses a 48-micrometer diameter aperture at net density 1.00. Output-pixel RMS is not that measurement.

Portra 800's Print Grain Index cannot be compared with density RMS. Repository granularity presets are not verified Kodak density-RMS values.

Independent CMY fields are consistent with the available negative evidence. A raw UltraMax 400 negative scan from a Coolscan V ED, without disclosed noise reduction, shows inter-record grain coherence of about 0.0–0.45 across 5–160-micrometer wavelengths. The V3 film-scan negative of a flat patch at the same pitch measures −0.1–0.25. Finished lab scans of CineStill 800T and pushed Portra 800 show coherence of 0.56–0.99 and chroma-to-luma ratios of 0.3–0.77 at fine scales. In the night-sky window, red-green coherence falls from 0.94 at fine scales to 0.70 at coarse scales. This pattern matches scanner chroma low-pass filtering more closely than intrinsic record correlation. A prototype that correlated the record counts produced a flat coherence spectrum and did not match it. No public cross-spectrum of modern color-negative dye records was found.

Matched-tone comparisons against high-speed lab scans at the same film pitch agree on grain-clump size (4.0–4.6 versus 4.5–5.6 micrometers by half-autocorrelation radius) and on tonal polarity: bright regions show sparse light specks and dark regions show sparse dark specks. Luminance grain RMS agrees within about 20% on the pushed Portra 800 sky and the 800T night sky. In bright regions, the real scans are more strongly skewed toward sparse light specks (skewness −0.48 and −0.98) than V3 (−0.18 and −0.62). The references pass through unknown lab tone curves, sharpening, and JPEG encoding, so this difference is not attributed to the film model.

A coarser blue record is a real negative property. Kodak's VISION3 500T 5219 data publish diffuse RMS granularity per record with a 48-micrometer aperture. At low and middle exposures, blue reads about 0.02–0.03 sigma D, and green and red about 0.005–0.008. The repository 500T preset `[9, 13, 30]` has the same direction. Finished lab scans of CineStill 800T, which uses this emulsion, show a much smaller blue excess: the blue-to-green ratio of high-passed grain RMS is 1.29 and 1.13 in two tone-matched windows. Before the channel-scale rule above, V3 on the print route gave 3.13 and 2.71. Removing the doubled blue `particle_scale` gives 2.55 and 2.43 and halves the blue mean shift caused by grain through the nonlinear print response. An 8-micrometer scanner chroma low-pass on that output, as a stand-in for lab noise reduction, gives 1.43 and 1.39. The remaining difference is therefore mostly scanner processing and the route: those negatives were scanned, not printed on cine print film. V3 must not lower the blue preset to imitate it. Cine usage stays unsupported until print-film references are available.

The field boundary and exact Poisson mass law are mathematical choices. The 3.5-micrometer support is resolvable at the comparison scale and was preferred in user review over 8 micrometers. Unknown reference provenance and the supported output range remain explicit acceptance limits.

## References

- [V3 product contract](spec.md)
- [Film grain architecture](../design.md)
- [V1 contract](../v1/spec.md)
- [V2 contract](../v2/spec.md)
- [Dehancer investigation](../../../docs/dehancer-grain-reverse-engineering.md)
- Kodak, [The Essential Reference Guide for Filmmakers](https://www.kodak.com/content/products-brochures/Film/kodak-essential-reference-guide-for-filmmakers.pdf), printed pages 31 and 56–57.
- Kodak Alaris, [Portra 800 E-4040](https://kodakprofessional.com/sites/default/files/2025-07/e4040.pdf), printed page 3.
- Kodak, [VISION3 500T 5219/7219 technical data H-1-5219t, July 2015](https://www.kodak.com/content/products-brochures/Film/VISION3-500T-Color-Negative-Film-7219-TECHNICAL-DATA.pdf), page 4.
- Newson et al., [Realistic Film Grain Rendering](https://www.ipol.im/pub/art/2017/192/), sections 2–4.
- Last and Penrose, [Lectures on the Poisson Process](https://stoch.math.kit.edu/img/Last/lastpenrose2017.pdf), sections 5.3 and 15.4.
