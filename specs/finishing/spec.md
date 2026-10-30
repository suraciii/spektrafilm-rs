# Common finishing

## Authority

This capability owns [common finishing](../../CONTEXT.md#language) after positive-image formation. [Film chain](../film-chain/spec.md) owns material formation and scanner interpretation. [Output mix](../output-mix/spec.md#output-mix) owns final composition with the prepared source.

## Scope and applicability

Finishing MUST provide one shared adjustment module across the eligible positive readings below.

| Reading | Applicability |
| --- | --- |
| Positive film transmission | Eligible |
| Negative converted to a positive image | Eligible |
| Photographic-paper reflection positive | Eligible |
| Imported-negative or saved-material continuation | Eligible only when its completed reading matches an eligible row |
| Negative transmission with preserved polarity | Inactive |
| Negative scan with base subtraction and preserved polarity | Inactive |
| Material output without a completed positive reading | Inactive |
| Film-free `input` workflow | Inactive |

Applicability MUST follow the resolved route, scanned medium, and delivered polarity. Changing to an inactive route MUST preserve stored finishing values and explain why they do not execute.

Finishing MUST NOT change material formation or scanner interpretation. It MUST NOT add negative conversion or print development to a route that lacks that operation.

Finishing MUST use fixed, image-independent responses. It MUST NOT measure automatic exposure, white balance, histogram endpoints, or subject masks.

Finishing operates on the RGB image supplied by [Image input](../image-input/spec.md). It MUST NOT change input-alpha handling or introduce alpha compositing. An RGB-only result MUST NOT be presented as proof of alpha preservation.

## Controls

The module MUST expose exactly four daily controls and optional [Expand](#expand).

| Control | Primary effect | Required limit |
| --- | --- | --- |
| Brightness | Raise or lower internal tones, with diminishing influence toward the reference endpoints | Not source exposure, linear EV, automatic exposure, or uniform whitening |
| Contrast | Increase or decrease internal tonal separation, with diminishing influence toward the endpoints | Not sharpening, texture, or a second endpoint crop |
| Warmth | Shift color along one fixed warm–cool direction, weighted by tone and existing chroma | Not Kelvin, source white balance, skin recognition, or split toning |
| Saturation | Increase or decrease color strength, with restrained amplification of strong colors | Not physical dye density, automatic warming, or added contrast |

Each daily control MUST have a neutral value of zero. The default module MUST leave the base positive unchanged, without an automatic range fit or fixed style curve.

Control effects MAY interact through their declared responses and downstream output processing. Help MUST NOT promise perceptual independence or protection of every skin tone, neutral, highlight, or saturated color.

Brightness and Contrast MUST preserve scalar tone ordering throughout their accepted parameter domain. Neutral Saturation MUST preserve color strength; Saturation alone MUST NOT color an achromatic input in the working representation.

Warmth MAY intentionally color neutral internal tones. Saturation follows Warmth and therefore MAY change the strength of that introduced color.

The module MUST NOT expose additional public finishing EV, Gamma, Midtones, Pivot, Vibrance, CMY, regional color wheels, output endpoints, or a curve editor. It MUST NOT provide selectable algorithm families or a before/after-mix mode.

Public control scales MUST have qualified photographic responses and validated numerical domains before publication. Research parameter limits MUST NOT become editor ranges solely because a numerical probe accepted them.

## Expand

**Expand** maps two input references to black and white in fixed sRGB encoding. Its black and white references are not scanner calibration points or output endpoints.

Expand MUST default to disabled, with black reference zero and white reference one. Disabling it MUST retain its references and skip range mapping exactly.

Enabling Expand MUST execute its full-strength hard-clamp mapping, including at the default references. It MUST NOT use endpoint equality as an implicit bypass.

Expand MUST reject nonfinite references, reversed or equal references, and spans below its declared numerical minimum. Validation MUST NOT silently reorder, widen, or replace the references.

Expand MUST apply the same mapping to each working RGB channel. It MUST NOT add per-channel endpoints, automatic statistics, soft compression, selectable modes, or an amount control.

Help MUST state that hard clipping destroys differences for that render. Downstream controls MUST NOT claim to recover those differences or guarantee an in-gamut final result.

## Editing and comparison

[GUI workflow](../gui-workflow/spec.md#stage-and-surface-ownership) owns the module's surface. Expand MUST remain a collapsible range group within Finishing.

Edit order MUST NOT change evaluation order. Every finishing edit MUST evaluate from the compatible base positive, never from an earlier finished or clipped result.

A row reset MUST restore only that control's default. A module reset MUST restore only finishing controls, including disabled Expand and its default references.

The module MUST provide a transient comparison that bypasses only common finishing. Comparison MUST preserve stored controls and MUST NOT become a history edit, preset value, or export setting.

Comparison MUST NOT bypass negative interpretation, material effects, output gamut processing, CMYK ink style, or final output mixing. Upstream randomness and accepted input identity MUST remain unchanged during comparison.

[Operator correction](../film-chain/lab-scan/spec.md#operator-correction) remains a distinct negative-conversion operation. Finishing MUST NOT reinterpret its stored values or substitute for its exposure-domain controls.

## Persistence and replay

Finishing MUST form one independently scoped, look-owned control group under [Presets](../presets/spec.md). Applying a complete look MUST replace its finishing values; applying an unrelated section MUST preserve them.

Film, paper, and route selection MUST preserve finishing values. They MUST NOT derive finishing adjustments from stock defaults or silently neutralize inactive stored values.

A versioned legacy complete document without finishing MUST restore neutral finishing. Sparse omission MUST remain no edit and MUST NOT create a finishing reset.

Saved photographs, recipes, GUI state, history, presets, and render snapshots MUST retain the same accepted finishing values. Their existing explicit-ownership and transactional loading rules MUST remain effective.

Loading invalid finishing MUST leave accepted controls, history, source documents, render identity, and published output unchanged.

Retired scanner points and withdrawn response paths MUST remain rejected under [Output boundary and retirement](../output-mix/spec.md#output-boundary-and-retirement). They MUST NOT become aliases or automatic conversions into finishing.

## Delivery

Finishing MUST affect only the processed branch before final [output mixing](../output-mix/spec.md#output-mix). The prepared-source endpoint MUST remain unchanged, including for negative captures.

Preview, Scan, Export, CLI, and final RGB diagnostics MUST use the same response definition and parameter snapshot. A reduced preview MUST NOT measure a different automatic finishing reference.

Preview reduction and nonlinear finishing MUST NOT be claimed to commute. Their numerical comparison follows [GPU execution](../gpu-execution/spec.md#acceptance).

[Photo export](../photo-export/spec.md) owns retained Save and independent Export. Saving MUST NOT apply finishing again.

[LUT delivery](../lut-delivery/spec.md) MUST include eligible pointwise finishing exactly once. It MUST retain finishing provenance without changing material-density wire meanings or waiving existing frame-setup and output-mix restrictions.

## Acceptance

1. Eligible positives expose the four declared adjustments and optional Expand without changing material formation, negative-conversion semantics, or the prepared-source endpoint.
2. Neutral finishing preserves the base result exactly; active finishing obeys its declared color interpretation, clipping boundary, and response domain across admitted consumers and backends.
3. Editing, comparison, persistence, and delivery preserve control ownership and replay the same accepted finishing definition without cumulative processing or silent parameter substitution.
