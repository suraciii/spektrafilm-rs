# CMYK ink style design

## Ownership and boundary

The [CMYK ink style specification](spec.md) owns the product contract and the `cmyk_ink_v1` color definition. The shared runtime owns one implementation for Finished RGB and scanned outputs. The GUI and CLI must not implement separate color transforms.

CMYK ink style is a terminal RGB appearance boundary. It runs after all scanner-domain effects, including scanner finishing, output gamut processing, unsharp masking, transfer encoding, and Procedural grain, and before output mix and writing. It does not belong to the spectral film or photographic printing stages.

The runtime parameter group is `cmyk`. It contains only `active` and `strength` for this release. No general effect graph, new workflow syntax, press profile, proof profile, or fourth spectral stage is introduced.

Finished RGB must not use film, paper, spectral preparation, photographic print filters, metering, or film grain. Profile resolution may remain required by existing runtime construction, but changing resolved film or paper profiles must not change Finished RGB pixels. Input color primaries, white point, and transfer state remain explicit.

CMYK is an internal appearance concept. The runtime image buffer and all published output remain three-channel RGB. No CMYK file, separation, plate, press-ready artifact, ICC press workflow, or physical proof is produced.

## `cmyk_ink_v1` model

The selected style is a fixed deterministic Lab grade. The research `ink` grade is the source of the constants and equations; its offline FOGRA/YNSN paper model is not a runtime dependency.

For each pixel, the shared reference algorithm is:

```text
linear_rgb = decode(output_rgb, output_transfer) when encoded input is supplied
lab = rgb_to_lab_d50(linear_rgb, output_color_space)
C = hypot(lab.a, lab.b)
s = smoothstep((lab.L - 8) / 37)
mapped.L = max(4, 50 + (lab.L - 50) * 1.03 - 0.28 * C * s)
mapped.a = 1.40 * lab.a
mapped.b = 1.40 * lab.b
full_strength = lab_d50_to_rgb(mapped, output_color_space)
result_linear = linear_rgb + strength * (full_strength - linear_rgb)
result = encode(result_linear, output_transfer) when encoded output is requested
```

`rgb_to_lab_d50` and `lab_d50_to_rgb` must use the declared output color space and one declared chromatic-adaptation transform. They must not assume sRGB when the output color space differs. The transfer function must not be applied inside the Lab conversion twice.

`smoothstep(x)` is `t*t*(3-2*t)` with `t = clamp(x, 0, 1)`. The grade uses no toe lift and no black-point compensation. Strength is clamped and validated by the shared parameter contract. Strength zero bypasses all conversion and returns the input exactly.

The algorithm must define behavior for finite signed headroom at the appearance boundary. It must not clip input samples merely because they are outside the display interval. Existing writer rules own final integer clipping and float preservation. Nonfinite samples must follow the existing runtime validation policy; the feature must not create nonfinite output from finite input.

## Implementation alternatives and decision

Two implementation shapes were considered:

1. A runtime CMYK separation and spectral paper simulation. This would require a press characterization asset, a non-unique separation policy, TAC/GCR choices, a per-pixel iterative solve, and unresolved redistribution rights for FOGRA-derived data. It is not the first-release implementation.
2. A deterministic RGB point transform using the research-selected grade. This keeps the public contract RGB-only, has no third-party runtime asset, permits one CPU reference and one GPU implementation, and keeps spatial print structure outside the pointwise boundary.

The second option is selected for `cmyk_ink_v1`. An offline RGB→RGB LUT may represent this same point transform after its domain, transfer state, precision, model revision, and parameter digest are recorded. A LUT is an acceleration or delivery artifact, not a second color definition.

## Execution ownership

Core owns parameter validation, color-domain transitions, stage order, and the reference transform. The backend owns device representation and execution. The GUI owns labels, controls, state composition, and publication behavior. The CLI owns argument carriers and reports. LUT delivery owns lattice baking, role names, artifact metadata, and QA. No owner may duplicate the transform.

CPU and GPU must use the same constants, output color-space matrices, adaptation, transfer state, headroom policy, and Strength blend. An explicit GPU request must fail before publication when the complete operation cannot execute on the device. Automatic selection may choose the complete CPU route when no usable GPU is available and must report the choice. After GPU selection, a missing CMYK implementation or device failure must not restart the image operation on CPU.

The feature must not allocate a four-channel image buffer, execute Python, parse research NPZ files, run a per-pixel CMYK solver, or execute YNSN/spectral paper integration at render time.

## Persistence and migration ownership

The implementation must record the exact owner-specific version transition in each migration module and test it at the carrier boundary. A parameter-document version bump does not implicitly migrate presets, GUI state, RenderRecipe, LUT metadata, telemetry, or stage evidence. Each carrier must reject its retired name in the new schema, accept it only in the explicitly listed legacy versions, and emit only canonical `cmyk` data after a successful migration.

The complete-parameter schema, CLI recipe schema, GUI-state schema, preset carriers, and render diagnostics each own their version boundary. Their migration code may translate the retired `magazine_print_color` group and route only when the carrier version permits migration. The exact legacy-version allowlist and new version value must be recorded in the implementation change for each carrier; this specification does not reuse one carrier's version number for another.

Current-schema runtime parsing is strict. It accepts `cmyk.active` and `cmyk.strength`, rejects `magazine_print_color.*`, and rejects the retired route. A document that supplies old and new fields together must fail before mutation. A failed migration must preserve source bytes, accepted GUI state, render identity, and retained output. Recognition of a retired spelling alone never authorizes migration.

Migration must be idempotent. Re-serializing a migrated document must emit only `cmyk` and the canonical base route. It must preserve unrelated settings and must not alter scan polarity, profile references, exposure, film, paper, output color, or output encoding.

The retired route `input > film > scan > magazine` is not a runtime alias. A versioned complete document may migrate it to `input > film > scan` and set `cmyk.active = true`. A current-schema route selector, sparse source, inline assignment, or recipe must reject the retired route and identify the replacement.

## LUT boundary

LUT delivery must represent CMYK ink style independently from photographic film and print stages. The final RGB stage role must use the canonical `cmyk_` naming after the cutover; it must not use retired magazine role names. Metadata must carry the base route, style identifier `ink`, model revision, Strength, output space, transfer state, and parameter digest.

The baker must preserve an `input` base route as an input-to-output transform. It must not silently add film, print, or scan stages because CMYK ink style is enabled. A segmented consumer and a complete collapsed transform must not apply the style twice.

Pointwise CMYK ink style may be baked into a LUT. Halftone, texture, registration, dot gain, and other spatial structure must remain outside that LUT. No LUT provenance may imply press certification, ICC proofing, or physical-print equivalence.

## Product language

The visible product name is `CMYK ink style`. Help must state that it is a modeled RGB appearance, that output remains RGB, and that it does not create separations or a press-ready file. `Printing` remains reserved for the real photographic printing operation. The retired Magazine label is not a product name for this feature.

The GUI must display the feature next to base workflow controls, retain Strength while disabled, show Finished RGB as the film-free input workflow, and state that resolved film and print profiles do not affect Finished RGB pixels. It must not imply that CMYK ink style reverses a negative or selects photographic paper.

## Acceptance evidence

Reference probes must cover encoded and linear output, neutral and chromatic samples, the smoothstep boundaries, strength endpoints, signed headroom, and multiple output color spaces. A fixed disabled and Strength-zero identity test must bypass conversion rather than merely compare approximate output.

Behavior evidence must cover Finished RGB, every supported scan route, Preview, Scan, Export, CLI processing, resident and per-stage execution, and LUT evaluation. Migration evidence must cover legacy fields, old route, old/new conflicts, idempotence, unknown-field rejection, and failed-import retention.

Visual evidence must use the fixed research image plus neutral and chromatic photographic probes at declared working resolution. It may establish visual intent and regression behavior. It must not claim physical-print accuracy, FOGRA conformance, or ICC equivalence.

GPU evidence must pair CPU f64 reference, native f32, and actual WGPU execution with declared budgets and observed execution paths. A shader name, backend label, or dispatch count alone is not evidence of GPU execution.
***
