# Simulation method design

The [product contract](spec.md) owns method behavior. This document owns the mapping boundary and invariants.

## Boundary

```text
workflow.mode
├── film → Spectral
│   ├── look source: film chain
│   ├── effect input: Film Input
│   └── halation: Film Halation
├── lut → RGB
│   ├── look source: LUT
│   ├── effect input: RGB Input
│   └── halation: RGB Halation
└── off → RGB
    ├── look source: Identity
    ├── effect input: RGB Input
    └── halation: RGB Halation
```

## Preset mapping

| Preset kind | Method | Render mode |
| --- | --- | --- |
| Film preset | Spectral | Film |
| LUT preset | RGB | LUT |
| Effects preset | RGB | Off |
| Section preset | Spectral | Film |

## Invariants

1. Exactly one render mode is active.
2. Method follows the active render mode.
3. The product adds no fourth render mode.
4. A method label does not change a serialized mode value.
5. Inactive branches are not read, validated, or executed.
6. A chained spectral-to-RGB workflow runs one method per stage.
