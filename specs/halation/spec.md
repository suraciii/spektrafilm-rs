# Halation models

## Authority

| Capability | Owns |
| --- | --- |
| This capability | The Film Halation and RGB Halation schemes; the shared spatial response; effect-order language. |
| [Film chain](../film-chain/spec.md) | Film Halation stage placement, film source response, and parameter storage. |
| [LUT transform](../lut-transform/spec.md) | RGB Halation stage placement, effect-order control, and parameter storage. |
| [Render modes](../render-mode/spec.md) | Mode activation. |
| [GUI workflow](../gui-workflow/spec.md) | Control inventory and stage ownership. |
| [Presets](../presets/spec.md) | Preset ownership and capture. |

## Scope

Two halation schemes exist: **Film Halation** and **RGB Halation**.

They share one spatial response.

They do not share input preparation, masks, output composition, or parameter values.

Effect type follows input.

### Visual equivalence

Film and RGB Halation are not pixel-identical.
They share one spatial response and one visual target.
Given the declared reference fixture and each model's calibrated parameters, the RGB halo MUST match the Film halo in extent, amplitude, and hue.

Visual equivalence is evaluated after the display transform on the halo difference between halation on and off.
Pixel equality, spectral equality, and density equality are outside the target and MUST NOT be claimed.

Film Halation is the visual reference.
The RGB model is calibrated to that reference with its own mask thresholds and strengths.

## Shared contract

| Scheme | Input | Highlight mask | Output composition |
| --- | --- | --- | --- |
| Film Halation | Film Input | None | Add to Film Input; optional renormalization |
| RGB Halation | RGB Input | Maximum-channel highlight mask | Positive difference added to RGB Input |

Film Input and RGB Input follow [CONTEXT.md](../../CONTEXT.md#language).

Film and RGB Halation parameters MUST NOT be interchangeable.

## Film Halation

### Input

Film Halation reads the Film Input.

The Film Input is not camera RAW.

### Behavior

- Film Halation runs before development and scan.
- It MAY include in-emulsion scatter, highlight reconstruction, source compression, and renormalization.
- Its first sigma and strength are per-channel film-plane values.
- It MUST use the Film Input as the halo source; it MUST NOT apply an RGB mask or an implicit source clamp.
- Highlight reconstruction runs only while Film Halation is active.
- A zero `halation_spatial_scale` produces no halation response and no renormalization.
- Its highlight reconstruction fields MUST NOT serve as RGB mask thresholds.

### Parameters

Film Halation owns film-plane sigma, bounces, decay, strength, scatter, highlight reconstruction, source compression, and renormalization.

[Film chain](../film-chain/spec.md#film-halation-source-response) owns the parameter names, units, and source-response behavior.

### Constraints

- Film Halation MUST NOT read the RGB Input.
- Film Halation MUST NOT read RGB mask thresholds.

## RGB Halation

### Input

RGB Halation reads the RGB Input.

### Behavior

- RGB Halation selects highlights by the maximum channel of the RGB Input.
- It scales each channel by that channel's source amplitude.
- Its mask thresholds are visual calibration values, not film exposure thresholds.
- The mask MUST preserve source hue.
- A saturated red source MUST produce a red halation.
- A saturated blue source MUST produce a blue halation.
- Above the low threshold and below the high protection range, the added halation MUST increase with source level.
- The effect MUST add only the positive difference between the spatial response and the mask.
- The effect MUST NOT darken the RGB Input.
- RGB Halation MAY run before or after the LUT.
- The effect order MUST be explicit.
- The default order MUST be before the LUT.

### Parameters

RGB Halation owns mask low, high, transition, per-channel spatial response and strength, bounce count, bounce decay, and effect order.

[LUT transform](../lut-transform/spec.md#rgb-halation) owns the parameter names, storage, and stage placement.

### Spatial response

Film Halation sigma and spatial lengths are film-plane micrometres. They convert through the working film pitch.

RGB Halation sigma and spatial lengths are micrometres on a fixed 35 mm reference frame. They convert through a reference pitch derived from the full working image's long edge before crop.

The stored camera film format and source metadata MUST NOT change the RGB reference pitch.

### Constraints

- RGB Halation MUST NOT read film density, film exposure, or a film profile.
- RGB Halation MUST NOT use a luminance-only mask.

## Activation

| Render mode | Active halation scheme |
| --- | --- |
| Film | Film Halation |
| LUT | RGB Halation |
| Off | RGB Halation |

An RGB workflow always uses RGB Input, including when its LUT was baked from a spectral simulation.

## Parameter separation

A shared parameter MUST NOT change both schemes.

An inactive scheme MUST retain its stored values and MUST NOT be read or validated.

## Acceptance

1. Film mode executes Film Halation; LUT and Off execute RGB Halation.
2. RGB Halation preserves red and blue source hue and increases with source level above the low threshold and below the high protection range.
3. Film and RGB Halation parameters remain independent; neither scheme reads the other scheme's fields.
4. The RGB halo matches the Film halo in extent, amplitude, and hue on the reference fixture.
5. A zero Film spatial scale produces no halation response and no renormalization.
