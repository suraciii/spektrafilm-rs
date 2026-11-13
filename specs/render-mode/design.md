# Render mode design

The [product contract](spec.md) owns mode behavior and activation. This document owns the dispatch boundary, inactive-stage ablation, and shared implementation surfaces.

## Dispatch boundary

```text
RuntimeParams
└── workflow.mode
    ├── Film → spectral film/print/scan graph
    ├── Lut  → decode → Effects → LUT → LUT Grade → output
    └── Off  → decode → Effects → output
```

The mode is one top-level dispatch decision. A graph MUST NOT carry per-stage mode flags or execute an inactive branch.

The graph is selected before preparation. Profile resolution, LUT file access, and effect preparation MUST follow the selected mode only.

## Ablation

An inactive branch MUST be absent from the prepared graph. It MUST NOT be validated, loaded, or executed.

| Inactive branch | Ablated boundary |
| --- | --- |
| Film, Print, Scan, and print viewing glare | LUT and Off |
| LUT declaration and LUT Grade | Film and Off |
| Output mix and CMYK ink style | LUT and Off |
| Spectral camera and film-grain engines | LUT and Off |
| Effects stage groups | Film |

Stored inactive leaves remain in the parameter document and GUI state. Ablation MUST NOT clear, normalize, or reinterpret them.

Film and Off MUST NOT read a stored LUT declaration. A missing or invalid inactive LUT file MUST NOT block either mode.

## Shared surfaces

| Surface | Sharing rule |
| --- | --- |
| Input decode, color interpretation, crop, rotation, and resize | Shared by all modes under [Image input](../image-input/spec.md) |
| Output color space, encoding, and gamut processing | Shared by all modes |
| Photo export and viewer delivery | Shared by all modes |
| Effects definitions and evaluation | Shared by LUT and Off; separate parameter groups from Film grain |
| Finishing response | Shared implementation; Scan Tone & Color and LUT Grade retain separate values |
| Preset resolver and library | One resolver; the preset kind selects the mode |

Effects MUST NOT become a second film-density model. LUT and Off reuse effect definitions, not film material preparation.

LUT Grade MUST reuse the finishing response. It MUST NOT read or mutate Scan Tone & Color values.

## Mode and preset identity

A preset kind selects its required mode in the same transaction. A preset document MUST NOT carry a mode field.

A manual switch clears applied-preset identity. It MUST NOT alter inactive values or prepare the preceding mode after the switch.

## Invariants

1. Exactly one mode owns the prepared graph.
2. No inactive branch is validated, read, or executed.
3. Mode switching preserves every stored group.
4. Active-mode validation precedes output publication.
5. LUT and Off never enter a print or scan stage.
