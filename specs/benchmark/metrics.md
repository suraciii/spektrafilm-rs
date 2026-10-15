# Benchmark measurement protocol

The [product specification](spec.md) defines evaluation scope and terms. This document owns measurement domains, algorithms, populations, and interpretation limits. The [machine interface](interface.md) defines serialization.

## Protocol identity

Each suite must select a named, versioned measurement protocol. The protocol identity must cover metric implementations, color transforms, sampling scales, region masks, distribution estimators, and stochastic aggregation. Changing any of these must create a new protocol identity.

A measurement must identify its domain, units, case, region, sample count, and estimator. A percentile must state its population. Pixel percentiles and percentiles across cases or seeds must remain separate.

An available scalar must be finite. An unavailable or inapplicable measurement must contain a stable reason and no numeric value. Reasons must distinguish missing evidence, incompatible conditions, unsupported reference scope, invalid correspondence, computation failure, and an intentionally unselected metric.

## Image domains

### Native numerical domain

The native domain must retain the renderer's declared color space, transfer state, and floating samples. Numeric health checks must run before display conversion or clipping. Comparisons in this domain must require matching color interpretation or use a declared conversion.

The protocol must record the floating master format and precision. A channel-interleaved raw dump must include dimensions, channel order, endianness, numeric type, color space, and transfer state in its run record. File length must match those declarations. Expanding f32 computations into f64 storage must not imply f64 computation.

Raster decoding must preserve the container's native channel precision before normalization. RGB16 PNG inputs and references must retain all 16 bits, including filtered and interlaced external files.

### Common display domain

The standard appearance protocol must convert both sides to encoded sRGB with D65 white through a fixed, versioned color transform. It must decode the source transfer function before color conversion. It must record chromatic adaptation and rendering intent where applicable. An ICC-managed reference must use its identified profile. An untagged reference must require an explicit declaration.

The protocol must preserve an unclipped conversion for range diagnostics. It must use a separately identified [0,1]-bounded display copy for SDR appearance measurements. It must report clipping introduced by this conversion. It must not infer a gamma of 2.2 from an sRGB label.

CIELAB measurements in the standard appearance protocol must use the D65 reference white and CIE 1931 2-degree observer. An ICC PCS intermediate with D50 white must be adapted consistently before this calculation. The implementation must not mix D50 and D65 Lab values.

A different display domain or viewing condition must select a different protocol. Automatic exposure or tone mapping must not be inserted into an SDR version comparison. An HDR protocol must declare its luminance scale, tone mapping, exposure sampling, and viewing conditions separately.

## Color difference

The standard protocol must compute CIEDE2000, identified as `delta_e00`, with `kL = kC = kH = 1`. It must retain a full-resolution difference map for applicable regions. It must report median, P95, P99, and maximum values. A policy with an applicable pixel threshold may request the fraction of pixels above that threshold. That fraction must identify the threshold and valid pixel count.

The implementation must pass the [Sharma, Wu, and Dalal supplementary data](https://hajim.rochester.edu/ece/sites/gsharma/ciede2000/). Verification must include zero chroma, hue wrapping, input symmetry, and neutral cases. The metric version must identify the implementation used.

The protocol must report signed `delta_l_star`, `delta_a_star`, `delta_b_star`, and `delta_c_star` in the same valid regions. It must retain signed maps and report signed mean and median. Chroma must mean `sqrt(a*^2 + b*^2)`. Hue-angle diagnostics must exclude colors below a declared chroma floor.

CIEDE2000 must remain a color-difference measurement. Its lightness, chroma, and hue terms must not be described as independent additive penalties. A custom equal-lightness color diagnostic must have a separate metric identity and must not be labeled complete-image `delta_e00`.

Color-card references must compare robust interior patch values with declared patch masks. The protocol must exclude patch edges, glare, and invalid correspondence. Patch measurements must not be substituted for a full-scene claim.

### Regional color diagnosis

Regional color diagnostics must use independent medians of the bounded-display Lab channels within the frozen interior masks. They must retain each side's Lab center, chroma, sample count, and geometry. Reference processing eligibility must apply before emitting any reference distance. Unknown grading or histogram matching must not become an eligible color target through a diagnostic layout or native-stage capture.

`reference.patch_delta_e00_equal_l` must replace only the model center's L* with the reference center's L* before computing CIEDE2000. It must preserve both a*/b* pairs. This diagnostic must not change an image, estimate an exposure correction, or claim an additive component of ordinary CIEDE2000.

Hue diagnostics must use the authored `color.hue_chroma_floor`. Hue must be `atan2(b*, a*)` in degrees. `reference.patch_hue_shift` must mean model minus reference on the shortest circular interval `(-180, 180]`. `reference.patch_hue_error` must report its absolute value. Both centers must meet the chroma floor; a center below it must produce `below_chroma_floor`, not zero hue error. Omission of the floor must produce `hue_diagnostics_not_selected`.

For each authored region pair, `reference.pair_separation_delta_e00` must retain CIEDE2000 between the two model centers. Its `reference_relation` observation must retain the corresponding reference separation. `reference.pair_separation_change` must report signed model-minus-reference separation. `reference.pair_separation_departure` must report the absolute departure from the reference separation. A small model separation does not establish that either patch has the correct color. Pair diagnostics must identify both region endpoints and must not claim pixel correspondence between the images.

Version measurements must use the existing `baseline_distance`, `candidate_distance`, and `distance_delta` suffixes. Single-run measurements must use `simulation_distance`. A version `distance_delta` must remain the arithmetic candidate-minus-baseline observation, including for signed hue shifts; it is not a separately wrapped candidate-to-baseline hue angle.

Whole-chart evidence must retain all eligible regions and identify every summary population by its member identities and count. Color, neutral, and endpoint populations must remain separate. Tail summaries and worst regressions must remain visible beside focus-region improvements. This evidence must not create an automatic all-patch nonregression requirement or a universal acceptance score.

## Tone and contrast

Controlled tone probes must identify their input exposure values in stops or log exposure. The response must preserve the input-to-output curve in linear luminance and Lab lightness. It must retain baseline and candidate response curves and signed differences.

Local response slopes must identify the exposure interval and units. Toe and shoulder measurements must state their estimator and interval. The protocol must not report arbitrary curve knees as physical sensitometry. A density measurement must use the reference's declared instrument and density definition instead of a rendered RGB proxy.

Natural-scene tone diagnostics must report fixed-region lightness quantiles and signed differences. Shadow, midtone, and highlight masks must be derived from a frozen suite mask or the baseline under a declared rule. Candidate-dependent region selection must not change the comparison population silently.

Local contrast must be measured at declared spatial scales. The protocol must state whether it uses linear luminance or display lightness. High-frequency contrast must remain distinct from tone-curve contrast because grain, scene detail, blur, and sharpening also change local variance.

## Perceptual spatial difference

The standard protocol must support LDR-FLIP on the common bounded display copies. It must use a pinned [official FLIP implementation](https://github.com/NVlabs/flip). It must record pixels per degree and output size. It must retain the difference map and its mean, P95, P99, and maximum over each applicable region.

The suite must declare the virtual viewing condition. A different pixels-per-degree value must not share calibrated budgets without separate validation. FLIP results must describe perceived difference under that condition, not aesthetic quality or film fidelity.

A FLIP map may rank automatic detail regions. It must not replace semantic regions or dedicated grain and halation measurements. Cross-platform results must not promise exact bit identity unless verified for the pinned implementation and environment.

SSIM may be provided as an explicitly selected diagnostic. Its domain, window, constants, data range, and scale must be fixed. Its luminance, contrast, and structure terms must not be interpreted as independent film parameters. It must not substitute for color measurement. Learned perceptual or population metrics must not be silently added to the standard protocol.

## Grain

Grain probes must use uniform gray and color fields at declared levels. The protocol must identify the grain engine, film format, image geometry, and active downstream stages. Spatial effects that remain enabled must be treated as part of the measured chain.

Fixed-seed comparisons must retain pixel differences. Statistical grain evaluation must measure each seed separately and then aggregate the specified seed population. A stochastic realization difference alone must not produce a grain-quality failure.

Residual extraction must state its domain, detrending method, window size, and border exclusion. A flat-field mean subtraction and a high-pass residual from a textured scene must have different identities. Grain-off output may be used as a paired reference if the case explicitly supplies that render configuration. Disabling grain must not be inferred by the evaluator.

The protocol must report per-channel and luminance residual RMS, chromatic residual strength, and channel covariance or correlation. A correlation with zero variance must be unavailable rather than zero. Encoded RGB diagnostics must be labeled as such and must not be called linear-light noise.

The power spectral density must identify its window, normalization, frequency units, and valid sample dimensions. It must retain a two-dimensional spectrum to expose directionality or periodic structure. A radial spectrum must retain frequency bins and spectral power. A scalar grain-scale estimator must state its derivation and must not replace the spectrum.

Frequency and spatial-size units must use pixels unless an evidence-backed film-coordinate mapping is available. A physical mapping must account for crop, resize, orientation, and film format. Unknown scanner resolution must not be replaced with file DPI metadata.

The protocol must retain seed count, each seed's measurements, cross-seed mean, and sample standard deviation. Fewer than two seeds must make cross-seed sample standard deviation unavailable. A confidence interval must state its estimator and assumptions. Seed variation must not establish cross-scene generalization.

Real-scan grain regions must have declared flatness, tone range, and processing limitations. Registration copies must not be used to measure native grain spectra. JPEG compression, resizing, scanner sharpening, and noise reduction must remain identified as confounders.

Output RGB RMS must not be labeled diffuse RMS granularity. Comparison with manufacturer granularity must reproduce the declared density domain, measuring aperture, and exposure conditions. [Kodak's 48-micrometer measurement example](https://www.kodak.com/content/pdfs/motion/KODAK-VISION3-250D-5207-7207-technical-information.pdf) is an instrument-specific reference, not a universal RGB budget.

## Halation, diffusion, and edge response

Bright-source probes must identify source shape, exposure, background level, film-coordinate scale when known, and downstream tone processing. The protocol must retain per-channel radial profiles for points and normal profiles for edges.

The source core and background masks must be frozen from the probe or baseline. Source-core clipping must remain measured. Candidate-dependent threshold segmentation must not silently move the measurement origin or integration area.

Halation diagnostics must report channel excess, peak amplitude and position, declared profile-width estimates, and integrated spill outside the source core. Spill must state its domain, channel weighting, and integration limits. A red halo, neutral bloom, and sharpening overshoot must remain distinguishable in the retained profiles.

Edge or sinusoidal probes must use fixed exposure and input contrast. Edge response must retain overshoot, undershoot, and edge-spread evidence. SFR or MTF estimates must state sampling, units, and the estimator. An end-to-end nonlinear film, print, and scan response must not be presented as the emulsion's independent MTF.

Natural bright-source regions may supply diagnostic profiles if their geometry is documented. Unknown source exposure, optics, and scanner processing must prevent an unqualified physical fit.

## Numerical health

Each rendered master must report dimensions, sample count, finite-sample status, per-channel extrema, and fractions below zero and above one in its declared native domain. An invalid floating sample must retain its location count and produce an execution-quality finding.

Display conversion must report its own below-zero and above-one fractions before clipping. Saving and display clipping must remain distinct from film response. Intentional negative or super-white output must not fail merely because it is outside [0,1]. The policy must select the relevant constraint.

Aligned numeric comparisons must report maximum absolute difference and RMSE with their domain. These values must remain numerical diagnostics rather than perceptual or aesthetic scores. File hashes must identify artifacts and must not substitute for decoded-image comparison when metadata or container encoding can vary.

## Aggregation and calibration

Case and region statistics must identify their valid masks and counts. A suite summary must aggregate equally weighted cases unless the suite declares another fixed weighting. It must not silently weight larger images more heavily. It must retain worst cases and protected region failures.

A reference objective must compare baseline-to-reference and candidate-to-reference with the same eligible population and protocol. Improvement must not be reported if either distance is unavailable or uses a different region, reference transform, or metric version.

Budgets must be tied to the protocol identity, observation scale, case population, and region rule. Calibration evidence must distinguish same-version noise, seed variation, accepted changes, and known regressions. Human labels must distinguish visibility, acceptance, and reference suitability.

A bootstrap or other uncertainty estimate must identify the resampling unit. Pixels within one image must not be treated as independent scene samples. The benchmark must not invent a significance level or claim confidence from a small unqualified corpus.
