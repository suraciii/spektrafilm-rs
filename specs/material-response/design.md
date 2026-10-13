# Film and print digital response design

## Ownership

The [product contract](spec.md) owns control behavior, domains, stage eligibility, retirement, and acceptance. `core::runtime` must remain the production entry point. Pipeline preparation must own route resolution, immutable baseline references, source geometry, and final publication. Film and Print preparation must own their effective response. Adapters must not implement color mathematics.

`film_render.response` and `print_render.response` must be separate typed values with shared internal primitives where their mathematics agrees. Print owns its tonal filtration controls; Film does not acquire unused print controls. `enlarger` owns global CMY. `output_mix` owns final composition. A common public stage graph or generic post-scan response module is not required.

## Prepared response

Preparation must resolve the baseline profile after existing chemistry, development, and base settings. It must retain an immutable baseline response for references and a separate prepared effective-response evaluator for actual material formation. This boundary must not require copying a full image or mutating source profile data.

The prepared response must contain material identity, polarity, reference exposure, neutral locus, reference readout, validated controls, and any bounded interpolation data. Numeric controls must be stored as f64. CPU and GPU execution must use their declared computation precision. Pixels must not allocate, fit profiles, construct matrices, or run unconstrained iterative color solvers.

Film processing must obtain baseline record exposures from the existing exposure stage, apply highlight and color response, then evaluate the material response. DIR and material grain must consume consistent effective response data, not combine an adjusted composite curve with stale layer curves. Existing sublayer sums, density normalization, and layer maxima must remain consistent. If a backend cannot evaluate that contract, dispatch must choose a faithful path before execution.

Print processing must derive baseline mask coordinates, apply local main-exposure filtration, combine main exposure and independent preflash using existing exposure conventions, then apply highlight and color response before material development. A one-dimensional curve per record is not sufficient to represent shared-exposure compression or material-relative chromatic coupling. Such operations must remain in the stage evaluator or a validated multidimensional stage LUT.

Active-neutral and disabled values must take the existing path before additional interpolation or color conversion. Material operations must not depend on output primaries, output encoding, viewport size, or image histograms. Existing spatial preparation and physical pixel pitch remain unchanged.

## Highlight mapping

Film must derive a common EV coordinate from linear record exposures normalized by the baseline neutral reference. The nonnegative record weights and normalization must be prepared once. Zero exposure must take the continuous zero-exposure limit rather than evaluate a logarithm of zero. A common multiplier must preserve the ratios of the original record exposures.

For a finite EV coordinate x above onset h, the mapping must be increasing, tangent-continuous at h, and have a slope in (0,1] that decreases as compression increases. Below h it is identity. It must have a finite evaluation strategy at the supported numeric extremes. The prepared definition must use the amount-zero identity branch exactly. A logarithmic soft-knee family can satisfy these constraints; its coefficients are digital normalization, not fitted chemical measurements.

Print must prepare a whiteward reference coordinate from its baseline neutral material response. The middle reference is 0 and material white is 1. This coordinate is internal material tone, not destination Oklab lightness or display RGB. The exposure direction must come from the selected material polarity. Preparation must reject a degenerate middle-to-white interval when an active nonneutral highlight control requires it.

The whiteward map must preserve onset and white, lower interior whiteward positions for positive compression, and join the identity map with continuous first derivative. A map with both endpoints fixed cannot have slope less than one everywhere inside the interval. The implementation must not claim otherwise: it compresses the selected interior interval and smoothly redistributes slope toward the endpoint. Outside the reference interval the extension must remain continuous and monotone. Blackward values must not change.

Both mappings must be defined before converting to material density. Changing print-white density or adding an Output tone curve is not an equivalent implementation. Fixed chemical developer exhaustion and creative highlight response must remain separately identifiable in preparation and provenance.

## Color coordinates

Each material must prepare a neutral locus from its reference illumination, sensitivity, curves, and spectral readout. One tone coordinate parameterizes that locus. Two transverse coordinates describe color departure for a color material. The coordinate transform must return the original response exactly on the bypass path and must have a declared valid domain and finite continuation policy.

Color density must change only the tone coordinate according to the unscaled color departure; saturation must then scale the transverse coordinates. Positive color density must move toward darker positive reference appearance for either material polarity. A common sign on raw CMY amounts is not sufficient. A monochrome material must bypass these controls without manufacturing chromatic coordinates.

The inverse mapping must produce representable material exposures and densities. A bounded prepared lookup may implement the mapping. It must not clamp individual channels to conceal an invalid model or silently saturate requested controls. The selected numerical definition must specify its reference readout, coordinate construction, direction, normalization, and validity bounds as versioned metadata shared by image rendering and LUT baking.

No fixed Oklab offset or RGB desaturation rule is prescribed for material color. A candidate is admissible only when it satisfies the product invariants and the measured numerical acceptance below. The implementation must publish the actual numerical definition with its evidence before claiming this capability complete; an unspecified color transform or a post-scan approximation is not an implementation of this design.

## Print exposure composition

One derivation must calculate the global main filter pack as `(C_neutral+C_shift, M_neutral+M_shift, Y_neutral+Y_shift)`. Neutral-database lookup must retain existing explicit-leaf protection. Main cyan shift must not enter the preflash derivation.

The untoned baseline sample must include global filtration and existing exposure normalization, but exclude Film/Print creative response when it is used as a calibration reference. Per-pixel print-tone masks must instead use the actual incoming film sample and the untoned baseline print response, so changed film density naturally affects the print. These are different inputs for different purposes and must not share a mutable scratch reference.

Local tonal filtration must derive all three weights once. The warm/cool filter direction must be prepared under the baseline positive print readout. It must not be an assumed sign of a single density channel. The combined local filter pack must satisfy the same effective filter-domain rules as global CMY. Out-of-domain active configurations must fail explicitly rather than clamp the filter pack.

Only main spectral exposure is recomputed with the local filter. Preflash is then combined under the existing exposure order. The same operation must be available to the normal spectral path and internal LUT acceleration. Three sequential remasked RGB grades, a changed preflash, or a second print simulation after Scan are not permitted substitutes.

## Reference preparation

The baseline reference identity must include the existing profile, material settings, illuminants, route, and scanner controls. It must exclude only the newly added creative response values. An effective render identity must additionally include those values and the response-definition version. Effective changes must invalidate dependent rendered stages without changing baseline reference data.

Exposure-reconstruction inverses must be prepared from baseline curves. For a forward creative warp W and baseline curve D, reconstructing with D inverse retains W; reconstructing with the inverse of D composed with W cancels the effect. The latter must not be used for these controls. Coupled receiver sampling may still use actual dye spectra; its density-to-exposure reference must exclude the creative mapping.

A common reference conversion, when available, must keep its receiver and tone convention and derive its stock anchors from the baseline. This design does not add a conversion method, rename existing methods, or expand their route admission. Scanner white/black references and print-normalization references must also exclude creative response.

Automatic setup that reads simulated samples must evaluate the baseline response on the same full-input measurement domain. This measurement pass may reuse existing bounded metering/setup machinery. It must not retain a second full-resolution rendered image. Crop and preview changes must not create a new baseline look. Stored or cached setup is reusable only under the existing input and baseline-reference identity rules.

Convert-film input recovery must retain its original reference and solve. Film response is inactive unless a later stage actually develops film. Print response still operates when the converted density proceeds to printing. Diagnostic input taps must admit a response only if their downstream path contains its owning operation.

## Output composition

The output compressor must remove its standalone lightness parameter and operation. Destination-specific chroma/gamut handling retains its existing implementation except for this explicit deletion. It must not gain a hidden highlight replacement. Scanner conversion and optional named appearances retain their independent tone conventions.

All final RGB paths, including Finished RGB, resident rendering, and final diagnostic collection, must meet at the same output-mix boundary. A source branch must be prepared before simulated camera exposure with the same geometry as the processed branch. Viewer rasters and display-ICC output must not supply source pixels.

For amount m, linear output is `(1-m)*source + m*result`. Endpoint branches must bypass unnecessary arithmetic and transfer round trips. Amount 1 must not retain another source image. Other amounts must restore the requested transfer only after mixing. Intermediate density/exposure taps must not perform this final operation. A missing source branch must cause rejection rather than a substitute image.

## Persistence and delivery

The [preset contract](../presets/spec.md) owns version migration and look capture. [CLI parameters](../cli-parameters/spec.md) owns discovery and sparse composition. [LUT delivery](../lut-delivery/spec.md) owns bake topology and provenance. Complete GUI state must use the versioned Rust extension for new leaves instead of inserting unsupported keys into Python-compatible sections.

Retired-field migration must inspect explicit legacy presence before typed defaults can erase it. Null or absence is eligible only in a versioned legacy complete document. New runtime schemas and sparse-edit validators must not carry a deprecated optional field. A failed conversion must not partially commit active controls or rewrite its source file.

Film and Print stage LUTs must include their effective response exactly once. The scan LUT must use baseline reference interpretation of the resulting density. Bundles and render reports must distinguish baseline identity, effective response definition, and backend precision. Output mixing below 1 requires a full source-to-output bake; a scan-stage input alone has no source branch.

## Numerical acceptance

The [benchmark contract](../benchmark/spec.md) owns measured numerical and photographic evidence. Response development must compare at least neutral and chromatic wedges, skin and saturated patches, monochrome, high-range scenes, and both print and direct-film routes. CPU f64 is the reference; native-f32 and GPU budgets must be measured rather than inferred.

Evidence must record the exact response definition, profiles, fixed scanner settings, baseline/effective density taps, reference identities, and final images. Highlight onset continuity, material-white preservation, color direction, neutral-locus behavior, density validity, layer consistency, and no reference cancellation are required independently of visual preference. Saturated or degenerate samples must not be used as the only evidence of an active effect.

There is no requirement for pixel parity with Dehancer. Its control names do not define material-domain behavior. Kodak [E-4021](https://125px.com/docs/paper/kodak/e4021-200909.pdf) supports the optical printing exposure and filter-pack boundary, not the digital response mathematics. Dehancer's [Film Compression](https://www.dehancer.com/learn/article/film-compression) and [Color Head](https://www.dehancer.com/learn/article/cmy-color-head-and-print-toning) describe creative controls, not calibration for this model.
