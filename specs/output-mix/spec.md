# Output mix and material controls

## Product model

This is the final target scope for material-control simplification, Output lightness retirement, and overall effect mixing. It specifies implementation requirements, not completed runtime behavior. Existing material controls remain authoritative; this capability does not add a digital material-response layer, another workflow, or a second RGB film pipeline.

**Preflash** is an independent, spatially uniform exposure added at the print material plane before development. **Overall effect amount** blends the complete result with the prepared source; it does not scale material parameters. **Prepared source** means decoded, geometrically aligned input before simulated camera exposure and processing, not the Input viewer raster or an automatically inverted negative.

## Product decisions

The chosen design reuses existing material controls and adds only final output mixing. The rejected alternative adds symmetric Film/Print digital-response groups. That alternative offers independent tonal coordinates but introduces overlapping controls, new material-color definitions, calibration isolation, and persistence costs without an established user need. Reuse is a product scope decision, not a claim of mathematical or photographic equivalence.

| User need | Final decision | Deliberate limit |
|---|---|---|
| Shape photographic-paper highlights | Use existing preflash, print exposure, and material curves | No independent highlight onset; paper white and midgray need not remain fixed |
| Adjust film contrast and shoulder | Keep existing development, chemistry, gamma, and developer-exhaustion controls with their existing profile capabilities | No new universal Film highlight compressor; no fitted-model fallback for unsupported profiles |
| Adjust global print color | Keep existing M/Y shifts, calibrated neutral filter pack, and exposure | No new Cyan shift; retaining cyan calibration does not expose another creative slider |
| Adjust color strength | Do not add Film or Print Saturation in this capability | Neither DIR nor gamma is relabeled as a saturation substitute; independent saturation remains outside this scope |
| Make colors heavier/lighter | Do not add Color Density | The ambiguous appearance request does not justify new material-color coordinates |
| Color different tonal regions | Keep global filtration and independently filtered preflash | No three-region print toning; these controls are not claimed to replace arbitrary split toning |
| Set readout endpoints | Keep medium-specific Scanner points | No duplicate generic Levels or measured-density/display-endpoint conflation |
| Reduce the complete effect | Add Overall effect amount | Zero on a negative capture is still a negative |
| Compress final lightness | Retire the standalone Output lightness operation | No hidden replacement in gamut handling |
| Control optical softness, material grain, scan appearance | Preserve distinct existing mechanisms and correct optical ordering | Similar final appearance does not make different physical stages interchangeable |

The unshipped draft groups `film_render.response` and `print_render.response`, and draft `enlarger.c_filter_shift`, are not accepted parameter paths. They must not appear in runtime models, GUI, discovery, new presets, or compatibility aliases. Documents containing them fail normal unknown-field validation. No migration or reserved empty group is required for these drafts. Future proposals for independent film highlights or saturation require separate scope and evidence; this implementation has no unresolved algorithm selection for them.

## Preflash

Reuse `enlarger.preflash_exposure`, `enlarger.preflash_y_filter_shift`, and `enlarger.preflash_m_filter_shift` with their existing types, defaults, bounds, and section ownership. Preflash exposure is the existing linear multiplier of the integrated spectral flash vector, not EV; GUI help and CLI discovery must describe that actual unit without changing stored values. Filter shifts retain existing enlarger filtration units. Zero exposure disables preflash exactly while retaining the filter settings. Do not introduce a second amount, automatic flash optimization, highlight mask, or high-dynamic-range recovery mode.

On negative-to-photographic-paper printing, preflash supplies proportionally more exposure to the low-exposure regions that form paper highlights. Depending on the material curve, this can bring weak exposure differences into a responsive region and change highlight contrast. It cannot recover information absent from the negative. Excess preflash may add fog, lower paper white, alter midtones, and change color. It must not promise fixed white, fixed midgray, or a monotonic improvement in visible detail for every setting.

Preflash remains an exposure operation on admitted print-development routes, including printing recovered negative density. Other print materials retain their own exposure-response polarity; the interface must not promise identical highlight behavior for printing film and photographic paper. On routes without print development it has no effect; route changes retain stored controls.

The main exposure uses the existing global filter pack. Preflash uses its own M/Y shifts around the calibrated neutral pack, not the main M/Y shifts. Uniform here means spatially uniform, not spectrally neutral. Existing print-exposure gain and normalization conventions must remain unchanged; an independent optical source does not imply independence from every existing overall exposure gain.

Print order is main filtration and transmitted exposure, optical propagation in linear exposure, independent material-plane preflash addition, then the existing nonlinear material response and development. [Print exposure composition](design.md#print-exposure-composition) owns numerical derivation and equivalence conditions. No local tone masks or digital color-response evaluator is introduced. Scanner calibration and automatic setup retain their existing material-setting semantics; this change must not introduce a new baseline/effective-reference split to make existing controls look stronger.

## Output boundary and retirement

The runtime, GUI, discovery, shaders, and new serialized documents must not expose or execute `io.output_gamut_compress.lightness_compression`. There must be no Output start, limit, or softness controls and no hidden replacement brightness curve in gamut processing. The remaining gamut algorithm and chroma controls retain their own meanings and placement. This change does not remove required negative-conversion tone responses or redefine optional scanner and Magazine appearances.

A versioned legacy complete-document migration may discard an absent or null retired value. It must reject every explicitly stored numeric triplet, including a value ignored by the old gamut algorithm. The error must identify the retired path and require explicit removal; no equivalent parameter conversion is claimed. It must not direct users to withdrawn digital-response controls. Current complete documents, sparse sources, and inline assignments must reject the retired path even when its value is null. Failed migration must leave source documents, active state, accepted render identity, and published output unchanged.

The discarded draft paths `reproduction.*`, `io.finished_rgb_output_processing`, and the Output response control section must not become aliases. Finished RGB retains its existing color-management path and may use overall effect mixing without enabling film or print development. Floating headroom and integer-writer clipping remain governed by [Photo export](../photo-export/spec.md). Removing lightness compression does not guarantee that every highlight fits the output gamut.

## Output mix

`output_mix.amount` must be f64, finite in [0,1], and default to 1. It mixes the prepared source with the complete result after output gamut processing, scanner-domain effects, and optional Magazine appearance, before saving conversion. Both branches must use the same output-space linear RGB and the same orientation, rotation, crop, and resize. This is one parameter of the settable set, not an additional workflow, per-stage opacity, or Magazine Strength alias.

The prepared source includes decoding, RAW white balance, and requested lens correction. It excludes virtual camera metering and exposure compensation, film, print, scanner interpretation, output gamut processing, and Magazine appearance. It uses the existing input-to-output color-space conversion, without a new creative tone curve. A negative-capture source remains negative. Final mixing does not guarantee an in-gamut or nonnegative result; float headroom and normal integer clipping rules still apply.

Amount 0 returns the prepared source exactly; amount 1 returns the complete result exactly without retaining an extra full-image source copy. Intermediate amounts linearly interpolate the branches without reseeding or recomputing stochastic effects. Endpoint shortcuts do not waive recipe, profile, route, or output validation. GUI Preview, Scan, Export, CLI, resident execution, and final diagnostic collection share the same composition rule and immutable parameter snapshot.

A diagnostic injection without a source branch must reject amounts below 1 for final RGB collection. Intermediate density and exposure taps do not perform final mixing. A segmented LUT has no source branch at its scan boundary; [LUT delivery](../lut-delivery/spec.md) therefore limits amounts below 1 to complete source-to-output bakes and rejects incompatible targets before publication.

## User operation

Retain MAIN, FILM, PRINT, ADVANCED, and CONFIG, existing section menus, and existing material-control ownership. Do not add Digital response panels or a Reproduction section. MAIN Enlarger exposes existing print exposure, M/Y filtration, and preflash with its independent filter controls. Preflash help describes photographic-paper highlight use and loss-of-white/color risks without renaming it HighLight Recovery or promising equivalence to a digital compressor.

Film and Print chemistry remain distinct from optical exposure. Existing developer exhaustion stays a model-specific development control, not a universal high-dynamic-range slider. Profile and route capability feedback must identify inactive controls without clearing values or fabricating support. Existing Scanner points retain medium-specific meanings. MAIN Output exposes Overall effect amount with the existing Output section menu and identifies the negative-source endpoint when applicable. Existing gamut controls must not be duplicated across pages.

Reset and section menus retain their current ownership: no new response reset or control section. The existing Output section owns mix reset to 1; Enlarger reset follows its existing scope. [Presets](../presets/spec.md) owns version migration and whole-look capture. [CLI parameters](../cli/spec.md) owns canonical paths, sparse edits, discovery, and inactive-edit feedback.

Optical and grain labels follow [Film chain](../film-chain/spec.md#scan-optics-and-print-viewing-glare) and [Film grain](../film-grain/spec.md). Print viewing glare is readout light, not paper exposure; Procedural is post-scan appearance, not material-grain formation. These distinctions remain required even though the digital-response proposal is withdrawn.

## Acceptance

1. With amount 1 and no retired value, existing CPU f64, native-f32, and admitted GPU results remain within their existing budgets, except for the explicit [optical-order cutover](../film-chain/spec.md#optical-order-cutover). Existing film chemistry, scanner calibration, profile assets, grain algorithms, and stochastic seeds are unchanged by this scope reduction. No source-copy or extra color round trip occurs at amount 1.
2. Preflash checks use actual admitted paper profiles, an exposure wedge, chromatic patches, and a high-range photograph. Record zero, moderate, and excessive flash under fixed scan settings; show print-density and final-output changes, detail behavior and white/color tradeoffs. Zero must reproduce the no-flash path. Main-filter edits must not change the prepared flash vector; flash-filter edits must not change main exposure. Existing overall exposure gains remain identifiable. Printing-film evidence must be separate from paper claims.
3. A spatial edge and constant-field image, including tile boundaries, must prove optical propagation precedes nonlinear development and uniform flash remains uniform. CPU f64 references, native-f32, resident, stage, and accelerated paths must honor the same order under [GPU execution](../gpu-execution/spec.md#algorithm-alignment). Preserve the separate scanner optical-order acceptance; do not call a stage declaration a measured pass.
4. Real CLI and GUI scenarios cover film-print-scan, direct film-scan, admitted negative conversion, converted-negative printing, and Finished RGB. Routes without printing retain preflash values but do not execute it. Unsupported chemistry remains unsupported; no reference-isolation pass or hidden replacement effect may be added to compensate for a limitation.
5. Mix checks compare both exact endpoints and a known intermediate linear mixture, including encoded output, float headroom, RAW, negative capture, and non-square rotation/crop/resize. Diagnostic missing-source rejection, intermediate-tap behavior, LUT target rejection and complete-bake equivalence must be exercised. Save does not rerun processing; Export uses its immutable snapshot.
6. Preset v1/v2 migration, current v3 complete looks and sparse sections, GUI state, recipe, parameter files, inline edits and LUT consumers must obey their format contracts. Cover legacy absent/null/numeric lightness values, current retired-path rejection, withdrawn draft keys, invalid mix numbers and bounds, and atomic failure. Old omitted section values must not reset unrelated controls.
7. Numerical and photographic acceptance follows the [benchmark contract](../benchmark/spec.md). Evidence identifies binary/model revision, source, profile, route, controls, backend/precision, measured budgets, density taps and images. Product reuse decisions do not claim Dehancer parity, independent digital highlight coverage, or completed runtime acceptance. Specification/link checks and analytic examples alone are not production evidence.
