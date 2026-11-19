# Halation model design

The [product contract](spec.md) owns halation behavior. This document owns the shared kernel, the two model boundaries, parameter ownership, and invariants.

## Boundary

```text
Halation kernel
├── Film Halation model
│   ├── input: Film Input
│   ├── no highlight mask
│   ├── optional scatter, highlight reconstruction, source compression, renormalization
│   └── output: modified Film Input
└── RGB Halation model
    ├── input: RGB Input
    ├── maximum-channel highlight mask
    ├── positive-difference add
    ├── effect order: before or after LUT
    └── output: modified RGB Input
```

## Shared kernel

The shared kernel owns the spatial response.

The response is a multi-bounce Gaussian sum with per-channel sigma, bounce count, and decay.

Each model supplies its own per-channel strength.

The kernel MUST NOT own input masks, output composition, strength, or parameter names.

## Film Halation model

The Film Halation model owns input preparation and output composition for the Film Input.

It applies the shared spatial response to the whole Film Input.

It MAY apply in-emulsion scatter, highlight reconstruction, and source compression before the response.

It adds the response to the Film Input.

It MAY renormalize per channel.

Its parameters are film-plane values.

The [Film chain](../film-chain/spec.md#film-halation-source-response) owns the parameter names, units, and source-response behavior.

## RGB Halation model

The RGB Halation model owns input preparation and output composition for the RGB Input.

It builds a maximum-channel highlight mask in linear light.

The maximum channel is:

\[
M(x)=\max(R(x),G(x),B(x)).
\]

The gate is:

\[
g(x)=\operatorname{smoothstep}(low,low+t,M(x))\,(1-\operatorname{smoothstep}(high+t,high+2t,M(x))).
\]

The per-channel mask is:

\[
m_c(x)=g(x)\,\frac{\max(0,I_c(x)-low)}{1-low}.
\]

Here `low` is the low threshold, `high` is the high threshold, `t` is the transition, and `I_c` is the input channel.

The halo is:

\[
H_c = s_c\,(G_\sigma * m_c),
\]

where `s_c` is the per-channel strength.

The model adds:

\[
\max(0,H_c-m_c)
\]

to `I_c`.

The model MUST preserve source hue and per-channel amplitude.

The model MUST NOT use luminance alone as the mask source.

The model MUST NOT read film density, film exposure, or a film profile.

The effect order is a model parameter, not a model variant.

The [LUT transform](../lut-transform/spec.md#rgb-halation) owns the parameter names, storage, and stage placement.

`LutHalationParams` is the wire type for RGB Halation; the domain term is RGB Halation.

## Ownership

| Boundary | Owner |
| --- | --- |
| Shared spatial response | This capability |
| Film Input preparation and film-stage composition | Film chain |
| RGB Input preparation, mask, and LUT-stage composition | LUT transform |
| Mode activation | Render modes |
| Controls | GUI workflow |
| Preset capture | Presets |

## Parameter boundary

| Scheme | Input type | Mask | Output | Must not read |
| --- | --- | --- | --- | --- |
| Film Halation | Film Input | None | Film Input | RGB mask thresholds |
| RGB Halation | RGB Input | Maximum-channel | RGB Input | Film density, film exposure, film profile |

- `FilmHalationParams` and `RgbHalationParams` MUST be separate types.
- `LutHalationParams` is the RGB wire type and MUST NOT carry film-only fields.
- A shared `HalationParams` MUST NOT carry fields for both schemes.
- `boost_ev`, `boost_range`, and `protect_ev` belong to Film Halation highlight reconstruction.
- `mask_low`, `mask_high`, and `mask_transition` belong to RGB Halation.
- Film sigma and RGB sigma MUST convert through their own input pitch or reference scale.

## Invariants

1. Effect type follows input.
2. One spatial response serves both schemes.
3. Film and RGB parameters do not cross.
4. The RGB mask preserves hue and per-channel amplitude.
5. Effect order is a parameter, not a scheme.
6. Inactive scheme values are retained and unread.
