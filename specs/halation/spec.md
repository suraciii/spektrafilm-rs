# Halation models

## Authority

| Capability | Owns |
| --- | --- |
| This capability | The Film Halation and RGB Halation schemes; the shared spatial response. |
| [Film chain](../film-chain/spec.md) | Film Halation stage placement, film source response, and parameter storage. |
| [LUT transform](../lut-transform/spec.md) | RGB Halation stage placement and parameter storage. |
| [Render modes](../render-mode/spec.md) | Mode activation. |
| [GUI workflow](../gui-workflow/spec.md) | Control inventory and stage ownership. |
| [Presets](../presets/spec.md) | Preset ownership and capture. |

## Scope

Two halation schemes exist: **Film Halation** and **RGB Halation**.

They share one spatial response.

They do not share input preparation, masks, output composition, or parameter values.

Effect type follows input.

### Appearance relationship

Film Halation is a film-stage model.
RGB Halation is an appearance model.
They share one spatial response.

They do not share masks, values, or a measured visual target.

An RGB workflow MUST NOT claim numerical or visual equivalence to Film Halation.
A built-in look pair MAY share an appearance intent; the pair identity is not equivalence evidence.

Pixel equality, spectral equality, density equality, and cross-method numerical equality are outside the target and MUST NOT be claimed.

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
- Its mask thresholds are appearance controls, not Film calibration values or film exposure thresholds.
- The mask MUST preserve source hue.
- A saturated red source MUST produce a red halation.
- A saturated blue source MUST produce a blue halation.
- Above the low threshold and below the high protection range, the added halation MUST increase with source level.
- The effect MUST add only the positive difference between the spatial response and one maximum-channel mask guard.
- The effect MUST NOT darken the RGB Input.
- RGB Halation runs before the LUT.

### Parameters

RGB Halation owns mask low, high, transition, per-channel spatial response and strength, bounce count, and bounce decay.

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
2. RGB Halation preserves red and blue source hue, increases with source level above the low threshold and below the high protection range, never darkens the input, and does not invert a red-dominant response into a green fringe.
3. Film and RGB Halation parameters remain independent; neither scheme reads the other scheme's fields.
