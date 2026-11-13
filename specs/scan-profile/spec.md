# Scan Profile

## Authority

This capability owns the built-in Scan Profile catalog, selection, `custom` state, and scan-stage appearance application.

A Scan Profile is a runtime scan stage, not a [feature profile](../feature-profiles/spec.md).

| Capability | Owns |
| --- | --- |
| [Film chain](../film-chain/spec.md) | Negative conversion and scanner interpretation |
| [Presets](../presets/spec.md) | Reusable look and section documents |
| [Tone and color adjustments](../finishing/spec.md) | Post-scan adjustment |
| Scan Profile | Built-in scan-stage appearance and `custom` state |

## Contract

A [Scan Profile](../../CONTEXT.md#language) is a named, built-in scan-stage appearance applied to a completed positive reading. It changes appearance after negative conversion and before Tone & Color.

It MUST NOT select film or print, change Conversion style, operator correction, scanner hardware, or Tone & Color.

The selector provides `No Scan Profile` and built-in entries. `custom` is the effective state when the transform does not exactly equal a catalog entry. `custom` is not a catalog row and has no persisted catalog identity.

The transform supports `curve` and `matrix_curve`. A `matrix_curve` catalog entry is built-in and read-only. A 3D LUT is not a Scan Profile transform; device characterization belongs to a Scanner Profile.

A reusable user combination MUST be saved as a look preset or section preset under [Presets](../presets/spec.md). The product MUST NOT add a second Scan Profile library, profile document, import surface, or profile-reference carrier.

## Applicability

| Reading | Applicability |
| --- | --- |
| Positive film transmission | Eligible |
| Negative converted to a positive image | Eligible |
| Photographic-paper reflection positive | Eligible |
| Imported-negative or saved-material continuation | Eligible only when its completed reading is a positive |
| LUT look output | Inactive |
| Negative transmission with preserved polarity | Inactive |
| Material output without a completed positive reading | Inactive |
| Film-free `input` workflow | Inactive |

Changing to an inactive route MUST preserve stored Scan Profile values and explain why they do not execute.

## Catalog

| Wire name | Display label | Claim | Default strength | Best for | Availability |
| --- | --- | --- | ---: | --- | --- |
| `none` | No Scan Profile | — | — | Any eligible reading | Always |
| `sp3000_inspired` | SP-3000 Inspired | Inspired | `0.6` | Color negative lab scans | Catalog |

- Names are stable serialized identifiers; display labels are localizable.
- `none` is a bypass state, not a transform.
- `custom` is not a catalog row.
- Built-in catalog values are versioned product data, not user assets.
- `Best for` is advisory. A mismatch MUST be shown without blocking the selection.
- Future `Matched` entries MAY use `curve` or `matrix_curve` as built-in read-only entries. They MUST NOT use `Inspired` or `Calibrated` labels.
- A `matrix_curve` entry MUST expose no editable matrix controls.
- A preset MAY carry a resolved `matrix_curve` that does not match a catalog entry; it is `custom` and read-only.

## Selection

Selecting `No Scan Profile` MUST bypass the scan-stage transform exactly and preserve every other control.

Selecting a built-in entry MUST replace only the Scan Profile transform and default strength. It MUST preserve film, print, workflow, Conversion style, operator correction, Tone & Color, and texture controls.

Editing any Scan Profile transform leaf MUST make the effective selection `custom`. The previous built-in name MUST NOT remain selected after an edit.

`No Scan Profile` and `Strength 0%` produce the same image result but different state:

| State | Transform selected | Strength | Result |
| --- | --- | --- | --- |
| No Scan Profile | No | Unused | Bypassed |
| Built-in or custom | Yes | `0` | No effect; selection is retained |

`Apply to selected` MUST apply the current strength to each selected photograph. Each photograph MAY override the applied strength. Applying a Scan Profile MUST mark the active look as modified and MUST NOT change film, print, Conversion style, operator correction, or Tone & Color.

## Transform

The Scan Profile transform is one of:

| Kind | Contents | Editing |
| --- | --- | --- |
| `bypass` | No transform | `No Scan Profile` |
| `curve` | Three per-channel one-dimensional curves | Editable in Advanced |
| `matrix_curve` | 3×3 matrix plus three per-channel curves | Built-in or preset-carried; read-only |

Curve rules:

- Each channel carries its own knots.
- Knot inputs MUST be strictly increasing and span the declared domain endpoints.
- Knot outputs MUST be monotone.
- The transform domain is `[0,1]`; values outside the domain use endpoint clamp.
- Strength MUST blend in linear light. `0` is an exact bypass; `1` is the full transform.

Matrix rules:

- The matrix operates in linear light before the curves.
- The matrix order MUST be fixed and MUST NOT be user-selectable.
- A `matrix_curve` entry MUST be read-only in the Scan Profile surface.
- A user edit MUST NOT detach the matrix or silently convert the entry to `custom`.

The transform MUST NOT include grain, halation, DIR, unsharp mask, lens blur, crop, or output settings.

The transform executes after negative conversion and before Tone & Color.

## Surfaces

### GUI

The Scanner surface MUST provide one Scan Profile selector. It MUST list `No Scan Profile`, built-in entries, and `custom` when effective.

The selector MUST NOT include a second library, import/export surface, or profile management.

Advanced editing MUST expose curve controls only. A `matrix_curve` entry or preset-carried matrix transform MUST NOT expose matrix or curve editing.

Saving a custom curve result MUST use the existing look or section preset action.

### CLI

The CLI MUST select the same names through the scanner parameter surface. Unknown names MUST fail before render.

`custom` MUST NOT be selectable by name. It is an effective state. A `matrix_curve` custom transform is read-only and cannot be produced by editing a built-in entry.

### Presets

A look preset MUST own the Scan Profile control group. A section preset MUST own it when the Scanner section is captured.

Preset capture MUST store the resolved transform and strength. A preset MUST NOT require a Scan Profile catalog entry or asset at replay.

Applying a complete look MUST replace the Scan Profile group with the captured values. Applying an unrelated section MUST preserve it.

A legacy document without the Scan Profile group MUST restore the bypass state.

## Persistence and replay

Saved photographs, recipes, presets, GUI state, history, and render snapshots MUST retain the same accepted Scan Profile transform and strength.

A catalog name MUST NOT be the replay authority. Replay MUST use the stored transform leaves. The effective name MUST be derived by exact comparison with the catalog:

- `none` when bypassed;
- a built-in name when the transform exactly equals that entry;
- `custom` otherwise.

Catalog changes MUST require an explicit product revision and MUST NOT silently reinterpret stored transform leaves.

Invalid transform or strength values MUST leave accepted controls, history, source documents, render identity, and published output unchanged.

## Acceptance

1. Every eligible positive exposes `No Scan Profile`, `SP-3000 Inspired`, and `Custom` for editable curves; selecting one changes only scan-stage appearance.
2. Curve and matrix-curve results preserve the resolved transform and strength without a Scan Profile asset dependency or catalog reinterpretation.
3. Inactive routes preserve stored values; user reuse uses look or section presets; no second Scan Profile library or import surface exists.
