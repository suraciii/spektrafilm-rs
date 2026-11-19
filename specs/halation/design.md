# Halation model design

The [product contract](spec.md) owns halation behavior. This document owns the kernel/model boundary, parameter ownership, and invariants.

## Boundary

```text
Halation kernel
├── Film Halation
│   ├── input: Film Input
│   ├── no highlight mask
│   ├── optional scatter, highlight reconstruction, renormalization
│   └── output: modified Film Input
└── RGB Halation
    ├── input: RGB Input
    ├── maximum-channel highlight mask
    ├── positive-difference add
    ├── effect order: before or after LUT
    └── output: modified RGB Input
```

## Ownership

| Boundary | Owner |
| --- | --- |
| Shared spatial response | This capability |
| Film input preparation and film-stage placement | Film chain |
| RGB input preparation, mask, and LUT-stage placement | LUT transform |
| Mode activation | Render modes |
| Controls | GUI workflow |
| Preset capture | Presets |

## Parameter boundary

| Model | Input type | Mask | Output | Must not read |
| --- | --- | --- | --- | --- |
| Film Halation | Film Input | None | Film Input | RGB mask thresholds |
| RGB Halation | RGB Input | Maximum-channel | RGB Input | Film density, film exposure, film profile |

- `FilmHalationParams` and `RgbHalationParams` MUST be separate types.
- A shared `HalationParams` MUST NOT carry fields for both models.
- `boost_ev`, `boost_range`, and `protect_ev` belong to Film Halation highlight reconstruction.
- `mask_low`, `mask_high`, and `mask_transition` belong to RGB Halation.
- Film sigma and RGB sigma MUST convert through their own input pitch or reference scale.

## Invariants

1. Effect type follows input.
2. One spatial response serves both models.
3. Film and RGB parameters do not cross.
4. The RGB mask preserves hue and per-channel amplitude.
5. Effect order is a parameter, not a model.
6. Inactive model values are retained and unread.
