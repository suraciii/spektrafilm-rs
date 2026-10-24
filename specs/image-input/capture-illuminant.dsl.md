# Capture illuminant DSL

A reviewable example of the **existing** parameter language for the capture illuminant leaf `camera.capture_illuminant`.

This document is not a schema, a parser, a new carrier, or a second configuration surface. It is a worked example of the [CLI parameter language](../cli/spec.md) — the unified `--set` option with its inline assignment, sparse TOML, and sparse JSON sources — applied to one optional runtime leaf.

Every assignment below MUST parse, merge, and validate under the rules that contract already defines. Where a rule here appears to disagree with that contract, that contract wins and this document is the defect.

The leaf's semantics — optional value, profile inheritance, explicit override, validation, inactive routes, and report fields — are owned by [Image input: Capture illuminant](spec.md#capture-illuminant). This document demonstrates them; it does not redefine them.

## The leaf

| Fact | Value |
| --- | --- |
| Canonical path | `camera.capture_illuminant` |
| Type | optional string |
| Unset | TOML omission, inline `null`, JSON `null`, or an omitted preset leaf |
| Unset resolves to | the selected film profile's `reference_illuminant` |
| Explicit values | a catalog illuminant name, or `BB<kelvin>` |
| Dedicated `process` flag | none — `--set` is the canonical surface |
| Editing surface parity | Input and capture → Camera in the GUI, per [Stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership) |

The accepted names come from the shared illuminant vocabulary that `describe --field` already reports for `enlarger.illuminant`: the catalog sources in `data/illuminants/catalog.json`, the bundled `T`, `TH-KG3`, `TH-KG3-L`, and `K75P` selections, and the analytic `BB<kelvin>` token.

`BB` MUST be followed by a temperature the spectral service accepts; a name it does not accept fails with the permitted values and never falls back to the profile reference.

Nothing in this language selects the capture illuminant from EXIF or RAW metadata, from RAW white balance, or from image content. Those are out of scope for version 1.

## Examples

Each example is a complete, valid `process` invocation fragment under the [parameter contract](../cli/spec.md#parameter-contract). Input declarations, `--film`, `--output`, and writer options are elided where they are not the point of the example.

### 1. Inherited — no edit

Omission is the normal case. Every existing preset, stock, and invocation is in this state, and it changes no pixels.

```bash
spektrafilm-f64 process input.png --output output.tif \
  --film kodak_portra_400 --paper kodak_portra_endura \
  --input-color-space sRGB --input-cctf-decoding true \
  --preset 'Kodak Portra 400' \
  --dry-run
```

The effective capture illuminant is the selected film profile's `reference_illuminant`, and the report records it as inherited rather than as a value anyone supplied.

The equivalent explicit statement of the same state is an inline clear:

```bash
--set camera.capture_illuminant=null
```

This is the unset state, not an empty illuminant and not a deletion of the field.

### 2. Explicit `BB4000`

A blackbody token overrides the profile reference for the camera-side conversion into film exposure only.

```bash
spektrafilm-f64 process input.png --output output.tif \
  --film kodak_portra_400 --paper kodak_portra_endura \
  --input-color-space sRGB --input-cctf-decoding true \
  --set camera.capture_illuminant=BB4000 \
  --dry-run
```

The value is plain text, so it needs no JSON quotes; the whole source is still one shell argument. A catalog name works identically:

```bash
--set camera.capture_illuminant=T
```

### 3. Explicit catalog name in a sparse TOML file

`capture.toml`:

```toml
[camera]
capture_illuminant = "T"
```

```bash
spektrafilm-f64 process input.png --output output.tif \
  --film cinestill_800t_cs41_approx \
  --input-color-space sRGB --input-cctf-decoding true \
  --set capture.toml \
  --dry-run
```

TOML omission means no edit, so a `[camera]` table without `capture_illuminant` leaves the inherited value alone.

### 4. Clear with JSON `null`

`clear.json`:

```json
{"camera":{"capture_illuminant":null}}
```

```bash
spektrafilm-f64 process input.png --output output.tif \
  --film kodak_portra_400 \
  --input-color-space sRGB --input-cctf-decoding true \
  --set capture.toml --set clear.json \
  --dry-run
```

The two sources apply in appearance order. The first sets `T`; the second clears it back to unset, which again inherits the profile reference. JSON `null` sets an optional leaf to its unset value; it does not delete the field, and it MUST NOT reach a non-optional leaf.

### 5. Invalid value — expected rejection

```bash
spektrafilm-f64 process input.png --output output.tif \
  --film kodak_portra_400 \
  --input-color-space sRGB --input-cctf-decoding true \
  --set camera.capture_illuminant=BB900 \
  --dry-run
```

`BB900` is below the accepted blackbody range, so the command MUST fail before output publication. The diagnostic MUST name `camera.capture_illuminant`, state the supplied value, and identify the accepted vocabulary or range. It MUST NOT fall back to the profile reference, and it MUST NOT retry with a different value.

An unsupported catalog name fails the same way:

```bash
--set camera.capture_illuminant=Sunset
```

Both forms fail identically in dry run, in normal processing, in sparse TOML and JSON files, in preset documents, and in imported GUI state. A later `--set` source does not rescue an invalid earlier one.

### 6. Report shape

`--dry-run` reports the requested value, the effective value, and the source that supplied it, separately from every other light source in the chain.

Inherited, with no edit:

```json
{
  "capture_illuminant": {
    "requested": null,
    "effective": "D55",
    "source": "unset",
    "profile_reference_illuminant": "D55",
    "status": "active"
  }
}
```

Explicit override:

```json
{
  "capture_illuminant": {
    "requested": "BB4000",
    "effective": "BB4000",
    "source": "set",
    "profile_reference_illuminant": "D55",
    "status": "active"
  }
}
```

Supplied by a preset:

```json
{
  "capture_illuminant": {
    "requested": "T",
    "effective": "T",
    "source": "preset",
    "profile_reference_illuminant": "T",
    "status": "active"
  }
}
```

Explicit on a route that computes no film exposure from scene RGB — the `input` route or a real negative-capture `input > convert-film > ...` route:

```json
{
  "capture_illuminant": {
    "requested": "BB4000",
    "effective": "D55",
    "source": "set",
    "profile_reference_illuminant": "D55",
    "status": "inactive",
    "reason": "route computes no film exposure from scene RGB"
  }
}
```

Field meanings:

- `requested` — the composed leaf value, `null` when unset.
- `effective` — the illuminant in force for this render. An unset leaf reports the inherited profile reference; an inactive leaf reports the profile reference too.
- `source` — `unset`, `set`, or `preset`. RAW white balance, the readout illuminant, `enlarger.illuminant`, and the output white point never appear here as the source.
- `profile_reference_illuminant` — the selected film profile's own reference illuminant, reported so an inherited value is never confused with an authored one.
- `status` — the same `active`, `inactive`, `constrained`, `overridden` classification the [report contract](../cli/spec.md#discovery-and-dry-run) uses for `edits`, with the same precedence.
- `reason` — `null` for an active value; otherwise the responsible condition.

A winning non-active `--set` edit warns on stderr, names the canonical path, and does not enable, clear, or substitute another value.

## What this example does not add

- No new schema, carrier, parser, or path. `camera.capture_illuminant` is an ordinary optional runtime leaf.
- No dedicated `process` flag, no shortened path, no alias.
- No Capture & Scan Setup document, no universal camera or scanner registry, no receiver-matrix authoring.
- No automatic inference from EXIF, RAW metadata, RAW white balance, or image content.
- No change to RAW white balance, the film profile reference, the readout illuminant, the enlarger illuminant, the output white point, or the scan sensor.
- No new 800T profile and no stock-specific value written into a generic look.

## Acceptance

1. Every assignment in this document parses, merges, and validates as the existing `--set` language already defines, with no added syntax.
2. Omission, inline `null`, and JSON `null` all reach the unset state, and each produces pixels identical to the same configuration without the leaf.
3. `BB4000`, `T`, and any other accepted catalog name resolve to that illuminant and change only the camera-side conversion into film exposure.
4. `BB900` and `Sunset` fail before output publication with the path, the supplied value, and the accepted vocabulary, identically in dry run, rendering, sparse files, presets, and GUI state import.
5. The report distinguishes requested, effective, and source, and names the inherited profile reference illuminant rather than presenting it as an authored value.
6. The GUI's Input and capture → Camera control writes this same leaf with the same type, validator, and conditions, and the canonical `--set` path remains addressable through `describe`.
