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
    ├── stage: before LUT
    └── output: modified RGB Input
```

## Shared kernel

The shared kernel owns the spatial response.

The response is a multi-bounce Gaussian sum with per-channel sigma, bounce count, and decay:

\[
L_c = \sum_{k=1}^{K} w_k\,G_{\sigma_c\sqrt{k}} * S_c, \qquad
w_k = \frac{\rho^{k-1}}{\sum_{j=1}^{K}\rho^{j-1}}.
\]

Here `S_c` is the model-prepared source, `sigma_c` is the first sigma, `K` is the bounce count, and `rho` is the decay.

The kernel returns `L_c` and MUST NOT add the result to its input.

The kernel MUST NOT own input masks, strength, scatter, source compression, renormalization, or parameter names.

Each model supplies its own source and output composition.

The kernel MUST use a positive pitch and MUST NOT clamp it to a fixed minimum.

A zero first sigma is a delta kernel.
The model wrapper owns activation and renormalization.

## Film Halation model

The Film Halation model owns input preparation and output composition for the Film Input.

It applies the shared spatial response to the whole Film Input.

It MAY apply in-emulsion scatter, highlight reconstruction, and source compression before the response.

It adds the response to the Film Input.

It MAY renormalize per channel.

Its parameters are film-plane values.

| Value | Effective value |
| --- | --- |
| Halation amplitude | `halation_strength[c] * halation_amount` |
| First sigma | `halation_first_sigma_um[c] * halation_spatial_scale` |
| Scatter core sigma | `scatter_core_um[c] * scatter_spatial_scale` |
| Scatter tail decay | `scatter_tail_um[c] * scatter_spatial_scale` |

The scatter mixture MUST preserve energy: `(1 - scatter_amount) * I + scatter_amount * ((1 - tail_weight) * G_core + tail_weight * E_tail)`.

The Film model uses the Film Input as the halo source.
It MUST NOT clamp the source or apply an RGB mask.

The model activates the response only when at least one effective amplitude is positive and at least one effective first sigma is positive.
When no effective first sigma is positive, the model returns the Film Input unchanged and does not renormalize.
Within an active response, a zero first sigma uses the near-delta floor of `1e-6` pixels and remains in renormalization.

Scatter runs when `scatter_amount` is positive and at least one channel width is positive.
A zero width uses the near-delta floor.
The scatter mixture MUST preserve energy: `(1 - scatter_amount) * I + scatter_amount * ((1 - tail_weight) * G_core + tail_weight * E_tail)`.

Renormalization MUST divide each channel by one plus its effective halation amplitude.
A zero-amplitude channel is divided by one.

A zero bounce count produces no response.
Active Film Halation MUST reject nonfinite values, `scatter_amount` outside `[0, 1]`, `scatter_tail_weight` outside `[0, 1]`, negative widths, scales, amounts, or strengths, and `halation_bounce_decay` outside `[0, 1]`.

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
H_c = s_c\,L_c,
\]

where `s_c` is the per-channel strength and `L_c` is the shared spatial response applied to `m_c`.
`L_c` uses the per-channel first sigma, bounce count, and bounce decay from the [shared kernel](#shared-kernel).

The RGB model MAY use a two-scale Gaussian approximation of `L_c`.
The approximation MUST be fitted to the Film response and verified on the reference fixture.
It MUST preserve the declared bounce count and decay response; it MUST NOT ignore those controls.

The model MUST NOT renormalize the result.

When every RGB first sigma is zero, RGB Halation MUST be an exact bypass.
Within an active response, a zero sigma channel uses a near-delta response.

The model adds:

\[
\max(0,H_c-m_c)
\]

to `I_c`.

The model MUST preserve source hue and per-channel amplitude.

The model MUST NOT use luminance alone as the mask source.

The model MUST NOT read film density, film exposure, or a film profile.

The RGB model runs before the LUT.

The [LUT transform](../lut-transform/spec.md#rgb-halation) owns the parameter names, storage, and stage placement.

`LutHalationParams` is the wire type for RGB Halation; the domain term is RGB Halation.

## Visual equivalence

Film Halation is the visual reference for both schemes.

The reference fixture is a linear-light image on the fixed 35 mm frame.
It contains a neutral background, a small white highlight, and saturated red and blue highlights.
Grain, diffusion, light leak, and LUT-grade changes are inactive.

Visual equivalence means the two halos have the same spatial extent, the same hue direction, and no perceptible difference in peak or integrated amplitude at the reference viewing size.
The reference viewing size is the fixture at 100% display scale.
The target applies to the before-LUT RGB position.

The RGB model matches the reference by calibrating its mask thresholds and per-channel strengths.
The calibration MUST use the declared reference fixture.
It MUST NOT read film density, film exposure, or a film profile at runtime.

Pixel equality, spectral equality, and density equality are outside the target.
The RGB model MUST NOT claim them.

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
- Film sigma and spatial lengths MUST convert through the working film pitch.
- RGB sigma and spatial lengths MUST convert through the fixed 35 mm reference pitch.
- The RGB reference pitch MUST derive from the full working long edge before crop.
- The stored camera film format MUST NOT change the RGB reference pitch.

## Invariants

1. Effect type follows input.
2. One spatial response serves both schemes.
3. Film and RGB parameters do not cross.
4. The RGB mask preserves hue and per-channel amplitude.
5. RGB Halation runs before the LUT.
6. Inactive scheme values are retained and unread.
7. All-zero RGB sigma is an exact bypass; a zero channel within an active response is near-delta.
8. The kernel returns a halo layer without strength, scatter, compression, or renormalization.
9. A zero Film first sigma is a near-delta response; when every Film first sigma is zero, the response is inactive and no renormalization runs.
