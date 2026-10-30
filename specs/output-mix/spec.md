# Output mix and material controls

## Product model

This is the final target scope for material-control simplification, Output lightness retirement, and overall effect mixing. Existing material controls remain authoritative; this capability does not add a digital material-response layer, another workflow, or a second RGB film pipeline.

**Preflash** is an independent, spatially uniform exposure added at the print material plane before development. **Simulation blend** blends the complete result with the prepared source; it does not scale material parameters. **Prepared source** means decoded, geometrically aligned input before simulated camera exposure and processing, not the Input viewer raster or an automatically inverted negative.

## Product decisions

This capability reuses existing material controls and adds final output mixing. Independent [common finishing](../finishing/spec.md#scope-and-applicability) is not a Film or Print digital-response group.

| User need | Final decision | Deliberate limit |
|---|---|---|
| Shape photographic-paper highlights | Use existing preflash, print exposure, and material curves | No independent highlight onset; paper white and midgray need not remain fixed |
| Adjust film contrast and shoulder | Keep existing development, chemistry, gamma, and developer-exhaustion controls with their existing profile capabilities | No new universal Film highlight compressor; no fitted-model fallback for unsupported profiles |
| Adjust global print color | Keep existing M/Y shifts, calibrated neutral filter pack, and exposure | No new Cyan shift; retaining cyan calibration does not expose another creative slider |
| Adjust color strength | Do not add Film or Print Saturation in this capability | [Common finishing saturation](../finishing/spec.md#controls) remains independent of material controls |
| Make colors heavier/lighter | Do not add Color Density | The ambiguous appearance request does not justify new material-color coordinates |
| Color different tonal regions | Keep global filtration and independently filtered preflash | No three-region print toning; these controls are not claimed to replace arbitrary split toning |
| Map positive input range | Use independent [Expand](../finishing/spec.md#expand) | No scanner point aliases or measured-density/display-endpoint conflation |
| Reduce the complete effect | Add Simulation blend | Zero on a negative capture is still a negative |
| Compress final lightness | Retire the standalone Output lightness operation | No hidden replacement in gamut handling |
| Control optical softness, material grain, scan appearance | Preserve distinct existing mechanisms and correct optical ordering | Similar final appearance does not make different physical stages interchangeable |

The unshipped draft groups `film_render.response` and `print_render.response`, and draft `enlarger.c_filter_shift`, are not accepted parameter paths. They MUST NOT appear in runtime models, GUI, discovery, new presets, or compatibility aliases. Documents containing them fail normal unknown-field validation.

No migration or reserved empty group is required for these drafts. Common finishing does not introduce independent film highlights or Film or Print saturation.

## Preflash

Reuse `enlarger.preflash_exposure`, `enlarger.preflash_y_filter_shift`, and `enlarger.preflash_m_filter_shift` with their existing types, defaults, bounds, and section ownership. Preflash exposure is the existing linear multiplier of the integrated spectral flash vector, not EV; GUI help and CLI discovery MUST describe that actual unit without changing stored values. Filter shifts retain existing enlarger filtration units.

Zero exposure disables preflash exactly while retaining the filter settings. Do not introduce a second amount, automatic flash optimization, highlight mask, or high-dynamic-range recovery mode.

On negative-to-photographic-paper printing, preflash supplies proportionally more exposure to the low-exposure regions that form paper highlights. Depending on the material curve, this can bring weak exposure differences into a responsive region and change highlight contrast. It cannot recover information absent from the negative.

Excess preflash MAY add fog, lower paper white, alter midtones, and change color. It MUST NOT promise fixed white, fixed midgray, or a monotonic improvement in visible detail for every setting.

Preflash remains an exposure operation on admitted print-development routes, including printing recovered negative density. Other print materials retain their own exposure-response polarity; the interface MUST NOT promise identical highlight behavior for printing film and photographic paper. On routes without print development it has no effect; route changes retain stored controls.

The main exposure uses the existing global filter pack. Preflash uses its own M/Y shifts around the calibrated neutral pack, not the main M/Y shifts. Uniform here means spatially uniform, not spectrally neutral.

Existing print-exposure gain and normalization conventions MUST remain unchanged; an independent optical source does not imply independence from every existing overall exposure gain.

Print order is main filtration and transmitted exposure, optical propagation in linear exposure, independent material-plane preflash addition, then the existing nonlinear material response and development. [Print exposure composition](design.md#print-exposure-composition) owns numerical derivation and equivalence conditions. Print exposure MUST NOT include local tone masks or a digital material-response evaluator.

Scanner calibration and automatic setup retain their existing material-setting semantics; this change MUST NOT introduce a new baseline/effective-reference split to make existing controls look stronger.

## Output boundary and retirement

The runtime, GUI, discovery, shaders, and new serialized documents MUST NOT expose or execute `io.output_gamut_compress.lightness_compression`. There MUST be no Output start, limit, or softness controls and no hidden replacement brightness curve in gamut processing. The remaining gamut algorithm and chroma controls retain their own meanings and placement.

This change does not remove required negative-conversion tone responses or redefine CMYK ink style. [Common finishing](../finishing/spec.md#scope-and-applicability) is independent of the retired Output lightness operation.

A versioned legacy complete-document migration MAY discard an absent or null retired value. It MUST reject every explicitly stored numeric triplet, including a value ignored by the old gamut algorithm. The error MUST identify the retired path and require explicit removal; no equivalent parameter conversion is claimed.

It MUST NOT direct users to withdrawn digital-response controls. Current complete documents, sparse sources, and inline assignments MUST reject the retired path even when its value is null. Failed migration MUST leave source documents, active state, accepted render identity, and published output unchanged.

The discarded draft paths `reproduction.*`, `io.finished_rgb_output_processing`, and the Output response preset section MUST NOT become aliases. The `input` workflow retains its existing color-management path and MAY use overall effect mixing without enabling film or print development. Floating headroom and integer-writer clipping remain governed by [Photo export](../photo-export/spec.md).

Removing lightness compression does not guarantee that every highlight fits the output gamut.

## Output mix

`output_mix.amount` MUST be f64, finite in [0,1], and default to 1. It mixes the prepared source with the complete result after [common finishing](../finishing/design.md#color-and-order) and the existing output tail, before saving conversion. Both branches MUST use the same output-space linear RGB and the same orientation, rotation, crop, and resize.

This is one look-owned Output control, not an additional workflow, per-stage opacity, or CMYK Strength alias.

The prepared source includes decoding, RAW white balance, and requested lens correction. It excludes virtual camera metering and exposure compensation, film, print, scanner interpretation, common finishing, output gamut processing, and CMYK ink style. It uses the existing input-to-output color-space conversion, without a new creative tone curve.

A negative-capture source remains negative. Final mixing does not guarantee an in-gamut or nonnegative result; float headroom and normal integer clipping rules still apply.

Amount 0 returns the prepared source exactly; amount 1 returns the complete result exactly without retaining an extra full-image source copy. Intermediate amounts linearly interpolate the branches without reseeding or recomputing stochastic effects. Endpoint shortcuts do not waive recipe, profile, route, or output validation.

GUI Preview, Scan, Export, CLI, resident execution, and final diagnostic collection share the same composition rule and immutable parameter snapshot.

A diagnostic injection without a source branch MUST reject amounts below 1 for final RGB collection. Intermediate density and exposure taps do not perform final mixing. A segmented LUT has no source branch at its scan boundary; [LUT delivery](../lut-delivery/spec.md) therefore limits amounts below 1 to complete source-to-output bakes and rejects incompatible targets before publication.

## User operation

Control placement MUST follow [Stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership) in the continuous inspector. Print / Enlarger MUST expose existing print exposure and M/Y filtration. Print / Preflash MUST expose the three existing preflash controls with their independent filtration.

The inspector MUST NOT add Film or Print digital-response panels or a Reproduction section. Preflash help MUST describe photographic-paper highlight use and loss-of-white/color risks without renaming it HighLight Recovery or promising equivalence to a digital compressor.

Film and Print chemistry MUST remain distinct from optical exposure. Existing developer exhaustion MUST remain a model-specific development control, not a universal high-dynamic-range slider. Profile and route capability feedback MUST identify inactive controls without clearing values or fabricating support.

Scanner references MUST retain their [medium-specific meanings](../film-chain/spec.md#scan-medium-and-scanner-controls). Output / Simulation blend MUST expose `output_mix.amount` and identify the negative-source endpoint when applicable. Existing gamut controls MUST NOT have duplicate editors.

Row and module resets MUST follow [Section reset](../gui-workflow/spec.md#section-reset). Simulation blend MUST reset `output_mix.amount` to its factory value of 1. A module reset MUST affect only the visible canonical paths it owns, not the wider serialized preset section.

Enlarger and Preflash MUST therefore reset their own visible controls even though both belong to the `enlarger` preset section. Moving controls MUST NOT change their numerical semantics or add a response reset. [Presets](../presets/spec.md) owns version migration, complete look capture, and the single primary menu anchor for each unchanged section scope.

[CLI parameters](../cli/spec.md) owns canonical paths, sparse edits, discovery, and inactive-edit feedback.

Optical and grain labels MUST follow [Film chain](../film-chain/spec.md#scan-optics-and-print-viewing-glare) and [Film grain](../film-grain/spec.md). Scan / Print viewing glare MUST identify readout light, not paper exposure, without changing its `print` preset ownership. Procedural is post-scan appearance, not material-grain formation.

These distinctions remain required even though the digital-response proposal is withdrawn.

## Acceptance

1. Existing material exposure, development, scanner calibration, and optical ordering retain their contracts while preflash applies only on eligible routes.
2. Output mix preserves the exact prepared-source and complete-result endpoints and applies intermediate blending in linear output RGB.
3. Retired fields remain rejected under their compatibility rules, and Save, Export, diagnostics, and LUT delivery retain their respective boundaries.
