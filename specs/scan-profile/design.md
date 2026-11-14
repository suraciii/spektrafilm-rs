# Scan Profile design

## Boundary

A Scan Profile is a runtime stage, not a feature profile, look preset, scanner profile, or profile library.

It owns one look-owned scanner group and the built-in catalog that initializes it. [Feature profiles](../feature-profiles/spec.md) remains a selection layer over existing runtime groups; it does not own a new stage.

The stage order is fixed:

```text
negative conversion
  → Scan Profile transform
  → Tone & Color
  → output
```

On a paper readout, viewing glare belongs to the readout and precedes the transform.

The transform MUST NOT change material formation, negative interpretation, Conversion style, operator correction, or output encoding.

## Runtime group

The runtime group is `scanner.scan_profile`. It carries the complete scan-stage appearance:

```text
ScanProfileParams {
    transform: ScanProfileTransform,
    strength: number,
}
```

The group is look-owned. It does not carry a profile reference, library path, display thumbnail, favorite state, or catalog identity.

`No Scan Profile` is represented by the `bypass` transform kind, not by a saved profile document.

## Transform

The transform is a typed union:

```text
ScanProfileTransform =
    { kind: "bypass" }
  | {
    kind: "curve",
    input_space: linear-sRGB,
    output_space: linear-sRGB,
    domain: [0, 1],
    outside_domain: clamp,
    curves: {
        interpolation: monotone-linear,
        channels: [ChannelCurve, ChannelCurve, ChannelCurve],
    },
    }
  | {
    kind: "matrix_curve",
    input_space: linear-sRGB,
    output_space: linear-sRGB,
    domain: [0, 1],
    outside_domain: clamp,
    matrix: [[number, number, number], [number, number, number], [number, number, number]],
    matrix_order: before_curve,
    curves: {
        interpolation: monotone-linear,
        channels: [ChannelCurve, ChannelCurve, ChannelCurve],
    },
    }

ChannelCurve {
    x: [number],
    y: [number],
}
```

Invariants:

1. The union is discriminated by `kind`; `bypass` carries no transform.
2. Each channel has its own knots; R, G, and B DO NOT share x.
3. `x` is strictly increasing with at least 1/1024 between adjacent knots; `y` is finite, monotone, and within `[0,1]`.
4. The first and last `x` values cover the domain endpoints.
5. `matrix_curve` applies a finite 3×3 matrix in linear light before its curves.
6. `matrix_order` is fixed to `before_curve`; it is not user-selectable.
7. The transform is finite and resolution independent.
8. Strength blends the transformed and input linear-light values.

The catalog data owner supplies exact curve values. A catalog revision is explicit. This design does not duplicate those values.

## Strength

`strength` is a per-photograph value; the [contract](spec.md#transform) owns its domain and blend. The catalog stores a default. A preset stores the captured value, and a photograph MAY override it.

## Catalog

The built-in catalog is product data. Its wire names are stable; display labels are localizable.

The catalog entries, names, and user-facing claims are owned by the [contract](spec.md#catalog). A `none` entry is a bypass; the built-in appearance entry carries the curve asset and default strength recorded there.

The catalog MUST NOT be a user-editable library. A catalog revision MUST NOT silently reinterpret stored resolved values.

A future `Matched` entry MAY use a read-only `curve` or `matrix_curve`. It MUST carry provenance. It MUST NOT be imported as a user profile.

A `matrix_curve` entry is built-in and read-only. Its matrix and curves MUST NOT be exposed as editable controls.

A preset MAY carry a resolved `matrix_curve` that does not match a catalog entry. It reports `custom` and remains read-only.

## Custom

`custom` is an effective state, not a catalog entry. It applies to any transform that does not exactly match a catalog entry. Only a `curve` custom transform is editable.

The selector derives the effective name by exact comparison:

```text
if bypass:
    none
else if transform exactly equals a catalog entry:
    that entry
else:
    custom
```

The comparison MUST NOT use tolerance. A direct curve edit MUST NOT leave the previous built-in name selected. A `matrix_curve` custom transform has no editor.

Selecting a built-in entry replaces the transform and sets its default strength. Selecting `No Scan Profile` restores bypass.

## Presets and replay

The look preset owns the Scan Profile group. Capture stores the resolved transform and strength; it does not store a catalog reference.

Replay uses the stored transform. A missing built-in catalog entry MUST NOT make a saved preset unreadable.

A legacy document without the group restores bypass. A section preset that excludes the Scanner section MUST preserve the active Scan Profile group.

## Validation

Validation MUST reject:

- unknown catalog names;
- nonfinite transform or strength values;
- adjacent `x` closer than 1/1024;
- non-monotone `y`;
- `y` outside `[0,1]`;
- missing domain endpoints;
- strength outside `[0,1]`;
- a transform that is not the declared kind.

Validation MUST preserve stored values on failure and MUST NOT substitute `No Scan Profile`.

An inactive route preserves stored values and does not execute the transform.

## Outside this capability

| Capability | Boundary |
| --- | --- |
| 3D LUT | A Scanner Profile device characterization, not a Scan Profile transform |
| User profile library | Rejected; reusable combinations are look or section presets |
| Import/export | Rejected; preset carriers remain the sharing surface |
| Grain, halation, DIR, USM, lens blur | Separate material or texture controls |
| Matrix editing | Built-in only; no user matrix editor |

## Verification design

Focused tests MUST cover:

- catalog resolution and exact custom matching;
- curve and matrix-curve round-trip;
- per-channel knot round-trip and endpoint coverage;
- matrix order and finite values;
- strength `0`, `1`, and intermediate values;
- bypass and inactive-route preservation;
- preset capture and replay without catalog dependency;
- GUI/CLI name parity and unknown-name failure;
- CPU/GPU parity for the same transform and strength;
- invalid transform rejection without state change.

Behavioral verification MUST compare a catalog selection with the equivalent explicit transform leaves under the same executable, backend, seed, and input.

## Evidence boundary

The catalog is a positive-image appearance asset. It is not an SP-3000 calibration, device profile, or claim of scanner equivalence. The `Inspired` claim remains part of the catalog contract.
