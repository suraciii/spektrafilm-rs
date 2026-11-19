# Halation models

## Authority

| Capability | Owns |
| --- | --- |
| This capability | Film Halation and RGB Halation model distinction; input classification; shared spatial response; effect order language. |
| [Film chain](../film-chain/spec.md) | Film Halation placement, source response, scatter, highlight reconstruction, and renormalization. |
| [LUT transform](../lut-transform/spec.md) | RGB Halation placement, highlight mask, positive-difference add, and effect order. |
| [Render modes](../render-mode/spec.md) | Mode activation. |
| [GUI workflow](../gui-workflow/spec.md) | Control inventory and stage ownership. |
| [Presets](../presets/spec.md) | Preset ownership and capture. |

## Scope

Two halation models exist: **Film Halation** and **RGB Halation**.

They share one spatial response.

They do not share input preparation, masks, output composition, or parameter values.

## Model contract

| Model | Input | Highlight mask | Output composition | Parameter owner |
| --- | --- | --- | --- | --- |
| Film Halation | Film Input | None | Add to film input; optional renormalization | Film chain |
| RGB Halation | RGB Input | Maximum-channel highlight mask | Positive difference added to RGB input | LUT transform |

A **Film Input** is the simulated linear film raw exposure inside a spectral simulation.

An **RGB Input** is the rendered RGB image inside an RGB simulation.

Effect type follows input.

Film Halation MUST NOT use an RGB highlight mask.

RGB Halation MUST NOT read film density, film exposure, or a film profile.

Film and RGB halation parameters MUST NOT be interchangeable.

The spatial response MUST have one implementation.

## Film Halation

Film Halation runs on the film input before development and scan.

It MAY include in-emulsion scatter, highlight reconstruction, and renormalization.

Its first sigma and strength are per-channel film-plane values.

Its highlight reconstruction fields MUST NOT serve as RGB mask thresholds.

## RGB Halation

RGB Halation runs on the RGB input.

It MUST select highlights by the maximum channel of the RGB input.

It MUST scale each channel by that channel's source amplitude.

A saturated red source MUST produce a red halation.

A saturated blue source MUST produce a blue halation.

The mask MUST preserve source hue.

Above the low threshold, the added halation MUST increase with source level.

The effect MUST add only the positive difference between the spatial response and the mask.

The effect MUST NOT darken the RGB input.

RGB Halation MAY run before or after the LUT.

The effect order MUST be explicit.

The default order MUST be before the LUT.

## Activation

| Render mode | Active halation model |
| --- | --- |
| Film | Film Halation |
| LUT | RGB Halation |
| Off | RGB Halation |

A spectral output that enters an RGB workflow carries RGB input; the RGB stage executes RGB Halation.

A chained spectral-to-RGB workflow MAY execute both models in stage order.

## Parameter separation

Film Halation owns film-plane sigma, bounces, decay, strength, scatter, highlight reconstruction, and renormalization.

RGB Halation owns mask low, high, transition, mask source, RGB spatial response, strength, opacity, and effect order.

A shared parameter MUST NOT change both models.

An inactive model MUST retain its stored values and MUST NOT be read or validated.

## Acceptance

1. Film mode executes Film Halation; LUT and Off execute RGB Halation; a chained workflow executes both in stage order.
2. RGB Halation preserves red and blue source hue and increases with source level above the low threshold.
3. Film and RGB halation parameters remain independent; neither model reads the other model's fields.
