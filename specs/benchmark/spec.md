# Film simulation benchmark

## Purpose and evaluation principles

The benchmark must support two users of the same evidence: an agent that detects and locates changes, and a person who inspects the rendered appearance.

The benchmark must distinguish change magnitude, closeness to a named reference, and human preference. It must not combine these questions into a universal film-quality score. A difference from a baseline must not imply a loss of quality. A smaller distance to one lab scan must not imply a more accurate film material model.

The benchmark must expose measurements and their evidence before an interpretation. An agent must be able to locate a reported change without reading logs or opening every image. A person must be able to inspect the same change in the full image and at native detail resolution.

This capability owns appearance comparison, evaluation decisions, and review artifacts. [CLI parameters](../cli-parameters/spec.md) owns rendering parameter resolution. [Image input](../image-input/spec.md) owns decoding. [Photo export](../photo-export/spec.md) owns image encoding and publication. [Telemetry](../telemetry/spec.md) owns render execution observations. The benchmark must not redefine these contracts.

The delivered benchmark must run as a Rust CLI without a Python runtime. [Rust delivery](rust-delivery/spec.md) defines distribution and migration acceptance. This requirement does not change the evaluation tracks, measurement meanings, or human-review behavior.

## Terms

A **case** identifies an input, a rendering configuration, named regions, and applicable measurements.

A **suite** is a versioned, nonempty collection of cases with a declared purpose and measurement protocol.

A **target** identifies one renderer, its data dependencies, its execution conditions, and explicit changes to case settings.

A **run** is an immutable collection of case render results for one target and suite.

A **baseline** is a run explicitly selected as the comparison anchor. It is not an automatically updated best result.

A **candidate** is a run evaluated against a baseline or a reference.

A **reference** is external evidence with a declared source, processing history, and permitted comparison scope.

A **simulation** is a rendered result of the selected input and configuration. A **comparison pair** identifies the left and right image roles and the evidence that permits their comparison.

A **region** is a named area or mask in a declared image coordinate system. A corresponding scene region is not necessarily a pixel-aligned region.

A **comparison** contains measurements between identified results, applicability decisions, and findings.

A **policy** identifies protected measurements, expected changes, reference objectives, and calibrated budgets.

A **finding** identifies a case, region, measurement, decision, and inspectable evidence.

A **review report** presents an existing comparison without changing its measurements or decisions.

## Evaluation tracks

### Local input and simulation

The benchmark must accept a user-selected local raster or RAW file without a built-in chart identity or fixture layout. An optional local reference must remain independent of the simulation input. Large input, prepared-image, run, and report assets must remain outside the source repository.

Single-image execution must produce the same case and run contracts as suite execution. A source-to-simulation comparison must report processing changes. A reference-to-simulation comparison must report distance to the selected reference. Neither comparison must imply acceptance without an applicable policy.

The benchmark must accept externally prepared images as new inputs. It must not infer the original framing, exposure, or film scale of such an image. Optional preparation provenance must identify the original dependency and operations without executing them.

Simulation crop and resize must use the renderer's case-owned parameters and preserve its metering and spatial-scale semantics. The benchmark must not implement another simulation crop or transform engine. Measurement and review regions must be extracted from completed results. A cropped simulation must remain identified separately from a detail crop.

A single-run comparison must support input-to-simulation, reference-to-simulation, or both pairs. Raster and RAW input comparisons must use an observed floating source result from the renderer's input route with the same decoding and geometry. An input-route result must remain labeled as decoded input, not an alternative film simulation.

Reference correspondence must use the existing reference scopes. Equal image dimensions must not establish correspondence. Visual references must remain viewable without pixelwise scores. Region correspondence and frozen geometric transforms must not alter simulation input. Warped reference copies must remain separate from native grain and sharpness evidence.

### Version regression

Version regression must compare results derived from the same source samples. It must preserve the suite's source interpretation, geometry, and seed protocol.

The comparison must expose differences in effective parameters, profiles, data, backend, and precision. An intentional difference must be declared by the selected targets. An undeclared condition difference must prevent an unqualified regression pass. A declared condition difference must remain visible in the result.

The benchmark must support executable changes, parameter changes, profile changes, and data changes. It must not assume that a source revision alone identifies rendered behavior.

Version regression must report the size, direction, and location of applicable appearance changes. It must not register, grade, histogram-match, or normalize the candidate to reduce the measured difference.

### Reference alignment

Reference alignment must compare a rendered result with named external evidence. It must identify the scope of the reference: material measurement, paired scene appearance, regional appearance statistics, or visual reference only.

A material measurement must identify its exposure, illuminant, processing, instrument, and measurement units. A finished scan must remain identified as the combined result of film, development, optics, digitization, inversion, and rendering where these stages apply.

Pixelwise measurements must require a verified geometric correspondence and a valid common-area mask. A feature-match count or a successful transform alone must not establish correspondence quality. Uncertain motion, occlusion, parallax, and invalid borders must be excluded.

A scene pair without reliable pixel correspondence must use separately identified corresponding regions, distribution measurements, or human review. An unrelated scan must not receive a full-reference image score.

A reference with brightness or histogram matching must not qualify as an unmodified tone target. An unknown processing history must remain unknown. A transformed reference may qualify only for a specifically declared diagnostic scope.

The report must distinguish distance to a selected reference from a claim about physical accuracy. Fitting cases and held-out evaluation cases must remain distinguishable. A held-out result must not use parameters fitted to that case. Cases from the same capture sequence or roll must share a split group.

## Suite coverage

The benchmark must support controlled probes, natural scenes, and external references as distinct case kinds.

Controlled probes must cover exposure and gray response, color patches and gradients, flat fields at multiple levels, bright points or edges, and edge or fine-line response. The suite must identify the active effects and the property each probe isolates. An RGB chart must not be represented as a measured spectral reference.

Natural-scene coverage must identify skin, neutral surfaces, foliage, sky, saturated objects, shadows, highlights, mixed illumination, and fine texture where present. Each case must state its semantic regions and applicable measurements.

A quick suite must contain fixed controlled probes and a named subset of scenes. A complete review suite must retain the declared spatial resolution and effects. Substituting a reduced preview for a full-resolution spatial case must not count as completion of that case.

Missing assets and unsupported cases must remain visible. The report must state selected, completed, failed, and unavailable case counts. Empty selection must fail. A subset must remain identified as a subset and must not receive the complete suite's qualification.

## Standard configuration entry point

[Standard benchmark configurations](standard-configurations/spec.md) define reusable isolated color-and-tone and full-appearance setups. They extend fixture generation and support the existing local-image and suite workflows. They must not introduce a film profile, preset kind, implicit baseline, or universal quality score.


## Reproducible rendering

Raster cases must provide the [source declaration](../cli-parameters/spec.md#input-declaration). Embedded ICC data must remain recorded as provenance and must not silently override the renderer's explicit declaration. An external reference's color interpretation must be resolved separately before comparison.

The run must retain the source configuration, resolved settings, selected seed, actual dimensions, renderer identity, dependency identities, and observed execution conditions. Dry run must not be presented as proof of decoding, device availability, or completed rendering.

Color and tone probes must support a declared deterministic configuration with stochastic effects disabled. Final-appearance cases must retain their selected effects. Disabling effects must create a different case configuration, not an undocumented evaluator correction.

Stochastic cases must support a fixed-seed comparison and a specified multi-seed statistical protocol. A changed random realization must not alone establish a grain regression. CPU, GPU, and precision differences must not imply bit identity.

The benchmark must preserve floating output for numerical analysis. Lossy previews must not be measurement sources. Numeric health measurements must preserve out-of-range samples before display clipping.

A detail region must be extracted from the completed full-image render. Rendering an input crop separately must not substitute for that region unless the case explicitly evaluates crop behavior.

## Measurements and decisions

The [measurement protocol](metrics.md) owns domains, algorithms, aggregation, and availability rules. The [machine interface](interface.md) owns serialized inputs, results, and command status.

Every finding must identify its measurement population and applicable region. Signed color and tone changes must use candidate minus baseline. A reference-distance improvement must use candidate distance minus baseline distance and must identify that a negative value means closer.

The comparison must preserve per-case results, critical regions, and worst cases. An improvement in one case must not cancel a protected failure in another case. Full-image averages must not replace region and tail measurements.

A comparison without a policy must describe changes without claiming acceptance. A policy must distinguish expected changes from protected properties. Declaring an expected change must not disable a protected constraint.

A required measurement that is unavailable must prevent a qualified pass. Missing measurements must not become zero. A valid comparison may conclude that evidence is insufficient. Human preference must remain separate from numerical acceptance.

A budget must identify its calibration population and protocol. A universal color-difference, pixel-error, or perceptual threshold must not be assumed. Budgets must be calibrated with repeat runs, stochastic variation where applicable, controlled perturbations, and independently reviewed cases. Calibration must distinguish visibility, acceptability, and reference suitability.

The benchmark must not change policy budgets, source declarations, targets, or baselines to make a candidate pass. The agent may select an existing policy and explain evidence. Updating a baseline or budget must remain a separate explicit action with recorded approval and identity.

## Human review

A review report must work offline with relative local artifact links. Report generation must not render images or recompute comparison measurements.

The report must show source input, baseline, candidate, and a reference when applicable. It must label each role, rendering configuration, geometry, and reference limitation. Different scene captures must not be presented as exact A/B correspondence.

The report must provide synchronized pan and zoom for pixel-aligned A/B images. It must support manual A/B alternation and default to a split view with a directly draggable divider. The divider must also support a range control and keyboard movement. An unaligned reference must use labeled side-by-side views instead of an exact-match slider and identify each crop's left and right dimensions independently. A full-screen control must be available for the selected image view and remain keyboard operable. Narrow layouts must not introduce horizontal page overflow.

Detail sections must include fixed semantic regions and automatically selected difference regions. User-selected crops and automatically selected difference crops must appear as directly clickable thumbnails below the main view. Activating a crop must update the large image and expose that region's full-image locator, exact coordinates, selection method, matching A/B crops, applicable findings, and individual image links. When no crop is available, the report must show an explicit empty state rather than a broken control.

Native detail must map one source image pixel to one device pixel at 100%. The viewer must account for device pixel ratio. The report must also offer 200% and 400% nearest-neighbor views. A fit-to-container view must not be labeled 100%.
A generated human review report must open with the selected case and seed's image-first main view in request order, followed by accessible navigation for every other case and seed, including failures. A single-run report must open input/simulation when no reference pair is selected and reference/simulation when that pair is selected. Reports with multiple pairs must provide a pair selector. Version reports must open baseline/candidate. Labels, measurements, difference maps, and detail crops must belong to the selected pair. The initial view must show the selected imagery before supporting evidence. It SHOULD include a short retained-condition summary and at most four rounded full-image metrics with their units, domains, and direction. Supporting evidence SHOULD remain folded.

Detail crops must use lossless files. Reduced full-image previews may use lossy files if their role is explicit. Previews must not replace the numerical masters or lossless detail evidence.

Display copies must use 16-bit RGB sRGB PNG without quantization dither. Fit previews must be derived from floating display samples with Lanczos resampling in encoded sRGB to a maximum edge of 1600 pixels before 16-bit quantization. Artifacts must record bit depth, source dimensions, resampling, quantization, and display role. Native zoom must use the full-size image; lossless detail crops must preserve its 16-bit channel values. Browser and display precision must not be presented as guaranteed 16-bit output. Measurements must continue to use floating masters.

Comparisons with different source resolutions must separate a declared common observation scale from native-pixel inspection. Every resampling operation must be identified. Registered color-comparison copies must remain separate from unwarped grain and sharpness inspection copies.

Difference maps must show their units, scale, clipping, and amplification factor, and provide a distinct difference-map mode alongside normal images. Comparable panels must use the same scale. A per-image automatically stretched map must not be presented as a comparable magnitude scale. Normal images must remain available beside difference maps.

Automatic regions must identify their ranking measurement and merge overlapping detections. They must not displace fixed critical regions. Ties and region ordering must be deterministic. The report must retain rejected or unavailable comparisons rather than hiding them behind attractive examples.

Source attribution and redistribution restrictions must accompany external reference artifacts. Local research access must not imply permission to publish originals or derivatives.

## Acceptance scenarios

1. Render the same deterministic case twice through the real renderer. Compare the outputs and distinguish repeatability from a cross-backend equality claim.
2. Change exposure, color balance, and tone contrast separately. Report the affected direction, response region, and inspectable image detail.
3. Change a grain seed without changing the grain model. Preserve the pixel difference and evaluate the multi-seed statistics separately.
4. Change grain amount or scale. Detect the corresponding flat-field strength or frequency change without treating scene texture as grain.
5. Change halation around a small bright source. Retain a regional finding even when the full-image mean is small.
6. Supply an unaligned scene reference and a tone-matched reference. Enforce each reference's allowed measurement scope.
7. Request GPU rendering and observe CPU execution. Retain the actual route and prevent an unqualified GPU result.
8. Fail a required case or measurement. Preserve its evidence and prevent a qualified gate pass. Reject an empty suite.
9. Generate a review from existing results with the renderer unavailable. Open directly on the selected images, activate user-selected and automatic crop thumbnails, inspect full-image location and synchronized native detail, toggle labeled difference maps and full-screen mode, and retain lossless crops on desktop and narrow layouts without horizontal page overflow.
10. Reuse a run and then change a renderer or data dependency. Reject stale cached evidence. Refuse replacement of an existing completed result.
11. Evaluate a protected skin region and an expected highlight change. Keep the skin failure visible despite the expected change.
12. Validate machine documents and execute CLI examples through the implemented interface. Reject unknown fields, invalid bounds, nonfinite values, unsafe artifact paths, and missing evidence references.
13. Run an arbitrary local raster and RAW input without a suite document. Compare decoded input with simulation and inspect their observed matching geometry and lossless detail.
14. Supply a local reference and select input, reference, and both comparison pairs. Preserve pair-specific roles, measurements, maps, and crops. A visual-only or regional reference must not gain pixelwise correspondence from equal dimensions.
15. Run an externally prepared crop and a renderer crop as distinct cases. Preserve their input identities and renderer-resolved geometry; keep full-render detail crops separate.
16. Build and inspect a multi-pair offline report with original input files and renderer unavailable. Preserve request order and switch pairs on desktop and narrow layouts.
17. Preflight a valid suite and an invalid renderer parameter across multiple seeds. Preserve resolved configurations and grouped failures without decoding, rendering, or publishing a completed run. Execute the valid configuration and compare its resolved parameters with preflight.
18. Inspect a retained version or single-run comparison after original runs are unavailable. Filter exact case, region, and metric identities; preserve unavailable reasons and separate per-seed observations from seed aggregates.
19. Generate and validate a pinned target independently of fixtures. Reject existing output and changed executable identity; use the target with a local input.
20. Open a report location for a case, seed, pair, and region offline. Reload and use browser history on desktop and narrow layouts; activate detail controls by keyboard and expose invalid locations clearly.
