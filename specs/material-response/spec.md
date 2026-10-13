# Film and print digital response

## Product model

A **baseline material response** is the selected profile with its existing development, chemistry, and base settings, before the added digital response controls. An **effective material response** adds creative tone and color adjustments to that baseline and forms the densities used by downstream stages. These adjustments are digital extensions of a material model, not claims about measured chemical controls. Source profiles must remain unchanged.

Film and Print must own their respective effective responses. Scan must read and interpret the resulting medium. Output must not provide creative lightness compression. This capability must not add a generic Reproduction node, another workflow, or a second RGB film pipeline.

## Film response

`film_render.response.active` must default to false. Film response must act after film exposure and before density formation, DIR, and material-grain formation. Its adjusted record densities must feed printing and scanning. Existing camera exposure, development, and chemistry meanings must remain distinct. Developer exhaustion must not be used as an alias for the new highlight control.

`film_render.response.highlight_start_ev` must be finite in [0,16] and default to 3. It is the highlight onset in stops above the fixed baseline film reference exposure. `film_render.response.highlight_amount` must be finite in [0,1] and default to 0. Zero must bypass the highlight mapping exactly. Increasing the amount must reduce the exposure-response slope above the onset, with a continuous tangent at the onset. The map must remain monotone and must not hard-clip exposure. Values at or below onset, including the reference exposure, must remain unchanged.

Highlight mapping must apply a common exposure multiplier to the three normalized film records. It must not independently clip or compress the three record channels. It must preserve their exposure ratios before the material response, not promise unchanged final saturation. The operation must support sampled profiles without requiring a fitted chemistry model.

Film response must run only where the route develops simulated film. Recovering densities from a real negative capture is not film development. Convert-film routes must not redevelop that input or change its conversion solve in response to these controls. Stored inactive controls must remain available on return to a film-development route.

## Print response

`print_render.response.active` must default to false. Print response must modify printing exposure or material response before print density is delivered to Scan. It must work with the admitted photographic-paper and printing-film profiles. It must be inactive on routes without print development.

`print_render.response.highlight_start` must be finite in [0,1) and default to 0.75. Its reference coordinate must place the baseline middle reference at 0 and the material-white endpoint at 1. It is not a display RGB value or a scanner point. `print_render.response.highlight_amount` must be finite in [0,1] and default to 0. Increasing the amount must compress the whiteward tonal interval above the onset. The map must preserve the middle reference and material-white endpoint, remain strictly monotone inside the interval, and join the unchanged lower interval smoothly. A smaller local slope at onset must not be achieved by lifting minimum density across the whole image.

Highlight direction must follow the selected material's positive appearance. On negative-to-paper printing, whiteward motion is toward lower paper exposure and density. It must not be implemented as an unconditional change to the high-exposure end of every print curve. Finite out-of-reference coordinates must have a declared continuous extension without clipping.

Film and Print adjustments must compose in stage order. A Print adjustment must not change film density. A Film adjustment may change the exposure and density subsequently formed by Print.

## Color response

Both response groups must contain `saturation` and `color_density`. Saturation must be finite in [0,2] and default to 1. Color density must be finite in [-1,1] and default to 0. These controls must use the same prepared material-relative neutral reference and color coordinates in their owning stage.

Saturation must scale departure from the material's neutral response, not multiply all dye densities. Zero must place the response on the prepared neutral locus. One must preserve the unadjusted color component. The neutral locus must remain unchanged for every saturation value. A neutral locus must come from the selected material and baseline reference illumination, not an assumption that equal CMY amounts are colorimetrically neutral.

Positive color density must move chromatic responses toward a darker positive appearance under the declared fixed reference readout. Negative values must move them toward a lighter appearance. The operation must be zero on the neutral locus. It must change the tone component as a function of color departure, not substitute a saturation reduction or a global exposure change. Saturation must then scale the color component using that same reference. The reference and masks must not be recomputed from the adjusted pixel or image histogram.

These controls must not claim invariant perceptual hue, chroma, or lightness under every later print and scan configuration. The selected response definition must declare its reference readout and supported material domain. Impossible densities must not be produced or silently repaired by clipping. Monochrome profiles must retain their monochrome response; saturation and color density must be reported as inactive there.

## Print color head and split toning

The Enlarger color head must filter the main printing illuminant before transmission through film and integration against print sensitivity. `enlarger.c_filter_shift` must default to zero and use the same units, finite-value rules, and effective filter domain as the existing M and Y shifts. The shift must add to, not overwrite, the calibrated cyan neutral reference. Calibration, normal printing, converted-film printing, and internal spectral acceleration must use the same main filter derivation.

Print exposure must remain the overall density control. Existing midgray-reference normalization must retain its meaning. It must not be labeled per-pixel Preserve Exposure or imply image-average luminance preservation. Preflash must retain its independent exposure and filter pack.

`print_render.response.shadows_tone`, `midtones_tone`, and `highlights_tone` must be finite in [-1,1] and default to zero. Positive values must warm the corresponding region and negative values must cool it under the declared baseline print readout. These controls must modulate main printing exposure through a modeled local filter adjustment before development. They must not tint completed RGB or claim a measured chemical bath or spatially masked darkroom process.

The three smooth region weights must be nonnegative and sum to one. They must come from one local untoned baseline print evaluation before spatial propagation, including main exposure, global filtration, and preflash. No weight may depend on another region's modified result. The local filtration must affect only main exposure; preflash must not acquire the local filter pack. Zero tone controls must bypass the extra evaluation. Image coordinates and neighboring pixels must not affect these weights. The subsequently propagated exposure may depend on neighboring samples.

Warm/cool print-tone controls must be inactive for monochrome print materials. They must not claim colored output from a spectrally neutral material or add an RGB tint as a fallback. Stored values must remain available for a color print profile.

Print must execute this order:

1. Evaluate the local baseline masks without spatial propagation or creative response.
2. Apply global and local main-exposure filtration and evaluate the transmitted main exposure.
3. Apply enlarger optical diffusion to the linear main exposure.
4. Add the independently prepared preflash at the material plane.
5. Apply Print highlight mapping, color density, and saturation to the received total exposure, then form material density.

Preflash is modeled as a spatially uniform material-plane exposure with its own filtration. It must not be filtered or blurred as part of the main image. Its existing exposure-unit and normalization conventions must remain unchanged. This capability does not add a nonuniform preflash source or another optical-path selector. Tonal effects must reach print density even when scanner finishing is disabled.

## Scan reference isolation

Scanner calibration, inverse curves, point ranges, and automatic-setup references must exclude the new creative response adjustments. Changing only a response control must not change those references or compensate the adjustment away. Existing baseline profile and development changes retain their existing reference behavior.

Where automatic scan setup depends on simulated samples, those samples must use the same input and baseline material settings with the new response groups bypassed. The resulting immutable setup must be shared by adjusted rendering, Preview, Scan, and Export. Manual scanner corrections must remain independently editable.

A stock-specific exposure inverse must use the baseline response rather than invert the effective creative response. A common reference conversion, when admitted by the route, must retain its common model and derive any stock reference anchors without the creative response. Actual receiver capture must still read effective densities. This rule does not enable an otherwise unsupported scan method or route.

Native negative output must remain negative. Existing scanner black and white points retain their interpretation coordinates. This capability must not add generic Levels or redefine measured minimum and maximum density as display endpoints.

## Output boundary and retirement

The runtime, GUI, discovery, shaders, and new serialized documents must not expose or execute `io.output_gamut_compress.lightness_compression`. There must be no Output start, limit, or softness controls and no hidden replacement brightness curve in gamut processing. The remaining gamut algorithm and chroma controls retain their own meanings and placement. This change does not remove required negative-conversion tone responses or redefine optional scanner and Magazine appearances.

A versioned legacy complete-document migration may discard an absent or null retired value. It must reject every explicitly stored numeric triplet, including a value ignored by the old gamut algorithm. The error must identify the retired path and explain that the user must remove it explicitly and select Film or Print response controls; no equivalent parameter conversion is claimed. Current sparse sources and inline assignments must reject the retired path even when its value is null. Failed migration must leave source documents, active state, and published output unchanged.

The discarded draft paths `reproduction.*`, `io.finished_rgb_output_processing`, and the Output response preset section must not become compatibility aliases. Finished RGB must retain its existing color-management path and must not execute Film or Print response controls. Floating headroom and integer-writer clipping remain governed by Photo export. Removing lightness compression does not guarantee that every highlight fits the output gamut.

## Output mix

`output_mix.amount` must be finite in [0,1] and default to 1. It must mix the prepared source with the complete result after Magazine appearance and before saving conversion. Both branches must use the same output-space linear RGB and the same rotation, crop, and resize.

The prepared source must include decoding, RAW white balance, and requested lens correction. It must exclude virtual camera metering and exposure compensation, film, print, scanner interpretation, creative response, output gamut processing, and Magazine appearance. On a negative-capture input it must remain the prepared negative, not an invented positive interpretation.

Amount 0 must return the prepared source. Amount 1 must return the complete result exactly without retaining an extra full-image source copy. Intermediate amounts must linearly interpolate the buffers without reseeding or recomputing stochastic effects. A diagnostic injection without a source branch must reject amounts below 1 for final RGB collection. Intermediate density and exposure taps must not perform final mixing.

## User operation

FILM and PRINT must each expose a Digital response group with its own bypass and reset. The existing Film and Print section preset menus must carry their response controls. Reset must not reset the other material, chemistry, source profile, scan setup, or output mix. The GUI must distinguish Film onset in EV from Print onset toward material white.

MAIN Enlarger must expose global CMY and print exposure. PRINT must expose the three tone regions. MAIN Scanner must retain acquisition and interpretation controls. Output must expose Overall effect amount, not creative lightness compression. Existing gamut controls must not be duplicated across pages. There must be no new Reproduction panel or Finished RGB response opt-in.

Route changes must preserve inactive values. Finished RGB must identify both material groups as inactive. Convert-film must distinguish recovered film density from a route that actually develops new film. The mix control must identify its negative-source endpoint before rendering.

## Acceptance

1. With neutral new controls and no retired lightness value, existing CPU f64, native-f32, and admitted GPU results must remain within their existing budgets, except for the declared [optical-order cutover](../film-chain/spec.md#optical-order-cutover). Disabled and active-neutral response paths must avoid numerical round trips; they must not restore the old optical order. Failure and bypass checks must cover sampled and fitted profiles.
2. Film highlight tests must prove onset continuity, monotonicity, unchanged middle reference, retained record-exposure ratios, and changed film-density output. Print tests must prove correct whiteward direction for each supported polarity, unchanged middle reference and material white, and changed print density without changed film density.
3. A neutral wedge and chromatic patches must prove neutral-locus invariance, saturation-zero behavior, signed color-density direction, and finite boundary behavior. Tests must identify the reference readout. Invalid material coordinates must fail before publication, not clamp silently.
4. Print tone tests must prove smooth partition weights, main/preflash isolation, no sequential remasking, and changed material output before Scan. Full-frame and isolated identical patches must agree when spatial effects are disabled. With diffusion active, a bright edge and nonuniform tonal mask must distinguish filtration-before-diffusion and diffusion-before-response from the reversed orders. Uniform preflash must remain uniform, including image and tile boundaries.
5. Changing only creative response must leave prepared scanner references and automatic setup unchanged. Adjusted densities must produce an observable change under fixed scanning on nondegenerate patches. A matching inverse of the adjusted response must fail this acceptance intent.
6. Real CLI and GUI scenarios must cover film-print-scan, reversal film-scan, admitted negative conversion, native negative output, converted-negative printing, and Finished RGB. Inactive-route controls must not execute. Resident, per-stage, and diagnostic paths must share stage placement and report faithful fallback.
7. Mix tests must compare both exact endpoints and a known intermediate linear mixture. Non-square crop, resize, rotation, RAW, and negative capture must verify source alignment. Save must not reapply response or mix; Export must consume its immutable snapshot.
8. Preset, state, recipe, CLI, and LUT consumers must exercise the migration and retirement rules. Numerical acceptance must include measured budgets and photographic evidence under the benchmark contract. Formula probes, parser checks, and this specification are not production acceptance.
