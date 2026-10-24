# Benchmark measurement protocol

The [product specification](../spec.md) defines evaluation scope and terms. This document owns measurement domains, algorithms, populations, and interpretation limits. The [machine interface](../../instrument/spec.md) defines serialization.

## Protocol identity

Each suite MUST select a named, versioned measurement protocol. The protocol identity MUST cover metric implementations, color transforms, sampling scales, region masks, distribution estimators, and stochastic aggregation. Changing any of these MUST create a new protocol identity.

A measurement MUST identify its domain, units, case, region, sample count, and estimator. A percentile MUST state its population. Pixel percentiles and percentiles across cases or seeds MUST remain separate.

An available scalar MUST be finite. An unavailable or inapplicable measurement MUST contain a stable reason and no numeric value. Reasons MUST distinguish missing evidence, incompatible conditions, unsupported reference scope, invalid correspondence, computation failure, and an intentionally unselected metric.

## Image domains

### Native numerical domain

The native domain MUST retain the renderer's declared color space, transfer state, and floating samples. Numeric health checks MUST run before display conversion or clipping. Comparisons in this domain MUST require matching color interpretation or use a declared conversion.

The protocol MUST record the floating master format and precision. A channel-interleaved raw dump MUST include dimensions, channel order, endianness, numeric type, color space, and transfer state in its run record. File length MUST match those declarations.

Expanding f32 computations into f64 storage MUST NOT imply f64 computation.

Raster decoding MUST preserve the container's native channel precision before normalization. RGB16 PNG inputs and references MUST retain all 16 bits, including filtered and interlaced external files.

### Common display domain

The standard appearance protocol MUST convert both sides to encoded sRGB with D65 white through a fixed, versioned color transform. It MUST decode the source transfer function before color conversion. It MUST record chromatic adaptation and rendering intent where applicable.

An ICC-managed reference MUST use its identified profile. An untagged reference MUST require an explicit declaration.

The protocol MUST preserve an unclipped conversion for range diagnostics. It MUST use a separately identified [0,1]-bounded display copy for SDR appearance measurements. It MUST report clipping introduced by this conversion.

It MUST NOT infer a gamma of 2.2 from an sRGB label.

CIELAB measurements in the standard appearance protocol MUST use the D65 reference white and CIE 1931 2-degree observer. An ICC PCS intermediate with D50 white MUST be adapted consistently before this calculation. The implementation MUST NOT mix D50 and D65 Lab values.

A different display domain or viewing condition MUST select a different protocol. Automatic exposure or tone mapping MUST NOT be inserted into an SDR version comparison. An HDR protocol MUST declare its luminance scale, tone mapping, exposure sampling, and viewing conditions separately.

## Color difference

The standard protocol MUST compute CIEDE2000, identified as `delta_e00`, with `kL = kC = kH = 1`. It MUST retain a full-resolution difference map for applicable regions. It MUST report median, P95, P99, and maximum values.

A policy with an applicable pixel threshold MAY request the fraction of pixels above that threshold. That fraction MUST identify the threshold and valid pixel count.

The implementation MUST pass the [Sharma, Wu, and Dalal supplementary data](https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/). Verification MUST include zero chroma, hue wrapping, input symmetry, and neutral cases. The metric version MUST identify the implementation used.

The protocol MUST report signed `delta_l_star`, `delta_a_star`, `delta_b_star`, and `delta_c_star` in the same valid regions. It MUST retain signed maps and report signed mean and median. Chroma MUST mean `sqrt(a*^2 + b*^2)`.

Hue-angle diagnostics MUST exclude colors below a declared chroma floor.

CIEDE2000 MUST remain a color-difference measurement. Its lightness, chroma, and hue terms MUST NOT be described as independent additive penalties. A custom equal-lightness color diagnostic MUST have a separate metric identity and MUST NOT be labeled complete-image `delta_e00`.

Color-card references MUST compare robust interior patch values with declared patch masks. The protocol MUST exclude patch edges, glare, and invalid correspondence. Patch measurements MUST NOT be substituted for a full-scene claim.

### Regional color diagnosis

Regional color diagnostics MUST use independent medians of the bounded-display Lab channels within the frozen interior masks. They MUST retain each side's Lab center, chroma, sample count, and geometry. Reference processing eligibility MUST apply before emitting any reference distance.

Unknown grading or histogram matching MUST NOT become an eligible color target through a diagnostic layout or native-stage capture.

`reference.patch_delta_e00_equal_l` MUST replace only the model center's L* with the reference center's L* before computing CIEDE2000. It MUST preserve both a*/b* pairs. This diagnostic MUST NOT change an image, estimate an exposure correction, or claim an additive component of ordinary CIEDE2000.

Hue diagnostics MUST use the authored `color.hue_chroma_floor`. Hue MUST be `atan2(b*, a*)` in degrees. `reference.patch_hue_shift` MUST mean model minus reference on the shortest circular interval `(-180, 180]`.

`reference.patch_hue_error` MUST report its absolute value. Both centers MUST meet the chroma floor; a center below it MUST produce `below_chroma_floor`, not zero hue error. Omission of the floor MUST produce `hue_diagnostics_not_selected`.

For each authored region pair, `reference.pair_separation_delta_e00` MUST retain CIEDE2000 between the two model centers. Its `reference_relation` observation MUST retain the corresponding reference separation. `reference.pair_separation_change` MUST report signed model-minus-reference separation.

`reference.pair_separation_departure` MUST report the absolute departure from the reference separation. A small model separation does not establish that either patch has the correct color. Pair diagnostics MUST identify both region endpoints and MUST NOT claim pixel correspondence between the images.

Version measurements MUST use the existing `baseline_distance`, `candidate_distance`, and `distance_delta` suffixes. Single-run measurements MUST use `simulation_distance`. A version `distance_delta` MUST remain the arithmetic candidate-minus-baseline observation, including for signed hue shifts; it is not a separately wrapped candidate-to-baseline hue angle.

Whole-chart evidence MUST retain all eligible regions and identify every summary population by its member identities and count. Color, neutral, and endpoint populations MUST remain separate. Tail summaries and worst regressions MUST remain visible beside focus-region improvements.

This evidence MUST NOT create an automatic all-patch nonregression requirement or a universal acceptance score.

### Retained region coordinates

Comparison MUST retain two separate color-center estimators for declared regions that select `color`. The implicit full-image population used by scalar metrics MUST NOT create region readouts, including on measurement failure. `bounded_lab_channel_median` MUST reuse the componentwise bounded-display Lab medians.

`unclipped_linear_srgb_mean` MUST average the unclipped linear sRGB samples before one XYZ-to-Lab conversion. Both MUST identify D65, CIE 1931 2-degree observer, and readout version `color-region-v1`. Neither estimator MAY replace pixel-map aggregates or ordinary patch metrics.

Each side MUST retain Lab, C*, gated hue, geometry, coordinate space, and pixel count. The mean estimator MUST also retain its linear sRGB mean. Independent reference regions MUST use each image's own geometry.

Aligned references MUST use the intersection of the declared region and the retained common-area mask, and MUST cite that mask.

Each displacement MUST identify its source and destination roles. Signed Lab and chroma changes MUST mean destination minus source. The record MUST retain the three-component Lab vector, signed L*/C*/h changes, Euclidean `delta_e76`, CIEDE2000, and the separately named equal-L CIEDE2000 diagnostic.

Full Lab vectors, lightness, DE76, and DE00 MUST obey tone eligibility. Chroma and equal-L diagnostics MUST obey chroma eligibility. Hue MUST also obey the authored floor.

Visual-only references MUST NOT acquire numerical readouts. Unselected families and empty populations MUST retain reasons, not zero-valued coordinates. Nonfinite derived values MUST fail measurement.

### Native-stage region statistics

Comparison MUST derive native-stage channel means, minima, and maxima only from captured buffers. Each available record MUST identify the run role, stage, output-region geometry, producer coordinate space, native domain, units, channel order, pixel count, host precision, f64 storage, and buffer artifact. CMY log10 record exposure and optical density MUST NOT be converted to Lab or relabeled RGB.

An output region MAY measure an output-stage buffer of matching published dimensions. A working grid MUST also have retained source-alignment evidence and matching working/output dimensions. A source grid MUST additionally have explicit crop-free, unit-scale, preview-free geometry on both routes.

Dimension equality alone MUST NOT map film-frame or grain-aperture grids. Unmapped grids MUST report `stage_region_geometry_unmapped`. Uncaptured stages MUST remain unavailable; route-skipped stages MUST retain the producer's not-applicable reason.

Missing, malformed, or nonfinite captured buffers MUST fail validation or measurement.

## Tone and contrast

Controlled tone probes MUST identify their input exposure values in stops or log exposure. The response MUST preserve the input-to-output curve in linear luminance and Lab lightness. It MUST retain baseline and candidate response curves and signed differences.

Local response slopes MUST identify the exposure interval and units. Toe and shoulder measurements MUST state their estimator and interval. The protocol MUST NOT report arbitrary curve knees as physical sensitometry.

A density measurement MUST use the reference's declared instrument and density definition instead of a rendered RGB proxy.

Natural-scene tone diagnostics MUST report fixed-region lightness quantiles and signed differences. Shadow, midtone, and highlight masks MUST be derived from a frozen suite mask or the baseline under a declared rule. Candidate-dependent region selection MUST NOT change the comparison population silently.

Local contrast MUST be measured at declared spatial scales. The protocol MUST state whether it uses linear luminance or display lightness. High-frequency contrast MUST remain distinct from tone-curve contrast because grain, scene detail, blur, and sharpening also change local variance.

## Perceptual spatial difference

The standard protocol MUST support LDR-FLIP on the common bounded display copies. It MUST use a pinned [official FLIP implementation](https://github.com/NVlabs/flip). It MUST record pixels per degree and output size.

It MUST retain the difference map and its mean, P95, P99, and maximum over each applicable region.

The suite MUST declare the virtual viewing condition. A different pixels-per-degree value MUST NOT share calibrated budgets without separate validation. FLIP results MUST describe perceived difference under that condition, not aesthetic quality or film fidelity.

A FLIP map MAY rank automatic detail regions. It MUST NOT replace semantic regions or dedicated grain and halation measurements. Cross-platform results MUST NOT promise exact bit identity unless verified for the pinned implementation and environment.

SSIM MAY be provided as an explicitly selected diagnostic. Its domain, window, constants, data range, and scale MUST be fixed. Its luminance, contrast, and structure terms MUST NOT be interpreted as independent film parameters.

It MUST NOT substitute for color measurement. Learned perceptual or population metrics MUST NOT be silently added to the standard protocol.

## Grain

Grain probes MUST use uniform gray and color fields at declared levels. The protocol MUST identify the grain engine, film format, image geometry, and active downstream stages. Spatial effects that remain enabled MUST be treated as part of the measured chain.

Fixed-seed comparisons MUST retain pixel differences. Statistical grain evaluation MUST measure each seed separately and then aggregate the specified seed population. A stochastic realization difference alone MUST NOT produce a grain-quality failure.

Residual extraction MUST state its domain, detrending method, window size, and border exclusion. A flat-field mean subtraction and a high-pass residual from a textured scene MUST have different identities. Grain-off output MAY be used as a paired reference if the case explicitly supplies that render configuration.

Disabling grain MUST NOT be inferred by the evaluator.

The protocol MUST report per-channel and luminance residual RMS, chromatic residual strength, and channel covariance or correlation. A correlation with zero variance MUST be unavailable rather than zero. Encoded RGB diagnostics MUST be labeled as such and MUST NOT be called linear-light noise.

The power spectral density MUST identify its window, normalization, frequency units, and valid sample dimensions. It MUST retain a two-dimensional spectrum to expose directionality or periodic structure. A radial spectrum MUST retain frequency bins and spectral power.

A scalar grain-scale estimator MUST state its derivation and MUST NOT replace the spectrum.

Frequency and spatial-size units MUST use pixels unless an evidence-backed film-coordinate mapping is available. A physical mapping MUST account for crop, resize, orientation, and film format. Unknown scanner resolution MUST NOT be replaced with file DPI metadata.

The protocol MUST retain seed count, each seed's measurements, cross-seed mean, and sample standard deviation. Fewer than two seeds MUST make cross-seed sample standard deviation unavailable. A confidence interval MUST state its estimator and assumptions.

Seed variation MUST NOT establish cross-scene generalization.

Real-scan grain regions MUST have declared flatness, tone range, and processing limitations. Registration copies MUST NOT be used to measure native grain spectra. JPEG compression, resizing, scanner sharpening, and noise reduction MUST remain identified as confounders.

Output RGB RMS MUST NOT be labeled diffuse RMS granularity. Comparison with manufacturer granularity MUST reproduce the declared density domain, measuring aperture, and exposure conditions. [Kodak's 48-micrometer measurement example](https://www.kodak.com/content/pdfs/motion/KODAK-VISION3-250D-5207-7207-technical-information.pdf) is an instrument-specific reference, not a universal RGB budget.

## Halation, diffusion, and edge response

Bright-source probes MUST identify source shape, exposure, background level, film-coordinate scale when known, and downstream tone processing. The protocol MUST retain per-channel radial profiles for points and normal profiles for edges.

The source core and background masks MUST be frozen from the probe or baseline. Source-core clipping MUST remain measured. Candidate-dependent threshold segmentation MUST NOT silently move the measurement origin or integration area.

Halation diagnostics MUST report channel excess, peak amplitude and position, declared profile-width estimates, and integrated spill outside the source core. Spill MUST state its domain, channel weighting, and integration limits. A red halo, neutral bloom, and sharpening overshoot MUST remain distinguishable in the retained profiles.

Edge or sinusoidal probes MUST use fixed exposure and input contrast. Edge response MUST retain overshoot, undershoot, and edge-spread evidence. SFR or MTF estimates MUST state sampling, units, and the estimator.

An end-to-end nonlinear film, print, and scan response MUST NOT be presented as the emulsion's independent MTF.

Natural bright-source regions MAY supply diagnostic profiles if their geometry is documented. Unknown source exposure, optics, and scanner processing MUST prevent an unqualified physical fit.

## Numerical health

Each rendered master MUST report dimensions, sample count, finite-sample status, per-channel extrema, and fractions below zero and above one in its declared native domain. An invalid floating sample MUST retain its location count and produce an execution-quality finding.

Display conversion MUST report its own below-zero and above-one fractions before clipping. Saving and display clipping MUST remain distinct from film response. Intentional negative or super-white output MUST NOT fail merely because it is outside [0,1].

The policy MUST select the relevant constraint.

Aligned numeric comparisons MUST report maximum absolute difference and RMSE with their domain. These values MUST remain numerical diagnostics rather than perceptual or aesthetic scores. File hashes MUST identify artifacts and MUST NOT substitute for decoded-image comparison when metadata or container encoding can vary.

## Aggregation and calibration

Case and region statistics MUST identify their valid masks and counts. A suite summary MUST aggregate equally weighted cases unless the suite declares another fixed weighting. It MUST NOT silently weight larger images more heavily.

It MUST retain worst cases and protected region failures.

A reference objective MUST compare baseline-to-reference and candidate-to-reference with the same eligible population and protocol. Improvement MUST NOT be reported if either distance is unavailable or uses a different region, reference transform, or metric version.

Budgets MUST be tied to the protocol identity, observation scale, case population, and region rule. Calibration evidence MUST distinguish same-version noise, seed variation, accepted changes, and known regressions. Human labels MUST distinguish visibility, acceptance, and reference suitability.

A bootstrap or other uncertainty estimate MUST identify the resampling unit. Pixels within one image MUST NOT be treated as independent scene samples. The benchmark MUST NOT invent a significance level or claim confidence from a small unqualified corpus.
