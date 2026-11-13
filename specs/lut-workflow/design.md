# LUT workflow design

The [product contract](spec.md) owns the export/import workflow and LUT preset lifecycle. [Render modes](../render-mode/spec.md) owns mode selection and activation. This document owns the boundary between LUT file export, LUT import, LUT presets, and the render dispatcher.

## Ownership

| Boundary | Owner |
| --- | --- |
| Spectral LUT bake and `.cube` publication | [LUT delivery](../lut-delivery/spec.md) |
| LUT declaration, validation, and render graph | [LUT transform](../lut-transform/spec.md) |
| Film, LUT, and Off mode selection and switching | [Render modes](../render-mode/spec.md) |
| LUT preset kind and lifecycle | This capability |
| Effects preset kind and lifecycle | [Presets](../presets/spec.md) |
| Film preset and section document kinds | [Presets](../presets/spec.md) |
| GUI mode selector, stage ownership, and surfaces | [GUI workflow](../gui-workflow/spec.md) |
| Canonical leaves and preset selection | [CLI](../cli/spec.md) |
| Film materials, print, and scan | [Film chain](../film-chain/spec.md) |
| LUT Grade response | [Tone and color](../finishing/spec.md) |

The implementation MUST NOT add a second LUT reader, a second spectral baker, a second preset resolver, or a second renderer. Export calls the existing `BundleBuilder`; import uses the existing LUT parser and evaluator; preset kinds extend the existing library.

## Mode boundary

[Render modes](../render-mode/spec.md) owns the `Film`, `Lut`, and `Off` dispatch and the activation table.

```rust
pub enum RenderMode {
    Film,
    Lut,
    Off,
}
```

The canonical parameter path is `workflow.mode` with values `film`, `lut`, and `off`. An absent value resolves to `film`.

The mode uses three retained groups:

```text
lut     -> declaration and strength
effects -> Diffusion, Halation, Grain
grade   -> LUT Grade
```

There MUST NOT be per-stage mode flags. Inactive values remain stored but MUST NOT trigger material preparation, profile reads, LUT reads, or validation of an inactive declaration.

`workflow.route` remains the spectral route and is active only in Film mode. `output_mix` and `cmyk` remain stored but inactive in LUT and Off.

The former `lut_look` group MUST migrate once into `lut`, `effects`, and `grade`. New writers MUST NOT emit `lut_look` or an alias.

## Export LUT

Export is a file operation. It MUST NOT mutate the active render, create a preset, switch mode, or load the exported file.

### Target

The simple export writes one complete `.cube` with `Topology::One` semantics: input RGB to output RGB. The builder rejects before writing when any of these is true:

- the source cannot produce a completed positive reading;
- the selected print target is unresolved;
- output mix amount is not exactly 1;
- `scanner.experimental_setup` is active;
- the requested input or output encoding is not a LUT transform encoding;
- the pointwise transform would silently omit a required operation.

The default encodings are display-encoded sRGB input and output. Linear sRGB and Display P3 remain selectable. The film simulation's current source encoding is not copied by default.

The existing `BundleBuilder` supplies the lattice. The product writes a single `.cube` and reports the encodings plus neutralized operations to the user. The `.cube` does not carry the application's preset or mode metadata.

### Neutralization

LUT delivery neutralizes optical diffusion, stochastic grain, and source-dependent operations. The simple export reuses those rules and exposes the result:

- active optical diffusion, halation, or grain is neutralized and named;
- output mix below 1 rejects the simple export;
- active frame-dependent setup rejects the export;
- eligible Tone & Color is evaluated once at the existing delivery boundary.

The export is not an import and does not create a LUT or a preset.

### Failure

A failed export preserves the current image, controls, mode, and preset library. Publication MUST be atomic, and no partial `.cube` MUST remain at the destination.

## Import LUT

The import path is:

```text
LUT_3D_SIZE / DOMAIN_MIN / DOMAIN_MAX / data
  -> LutDocument
  -> explicit LutSpec declaration
  -> LUT render graph
```

The importer MUST use the existing `.cube` reader. It MUST NOT infer a color space from the filename, profile, bundle, or image content. The declaration supplies input and output encodings, domain policy, shaper, and strength.

A newly imported LUT starts at strength one with all transform effects and LUT Grade disabled. Importing does not save a preset. The neutral gray ramp and color checker use the production LUT graph and display path.

The import surface separates primary photographic decisions from declaration evidence. Input and output spaces, domain policy, and strength remain visible.

The hash, per-axis domain values, and shaper declaration MAY be grouped as advanced evidence. The form states that the cube carries no color-space metadata and that sRGB is an assumption.

The domain policy defaults to clamp for a new import and for a replacement. Fail remains an explicit strict choice. The form explains that clamp reports its clipped fraction and that fail stops the render.

**Replace LUT** changes the external cube declaration and re-declares its input and output encodings, domain, domain policy, and shaper for the selected cube. It retains strength, LUT Grade enablement, LUT Grade values, and effect settings. Starting a new LUT is a separate action that resets those values.

The importer validates the file and declaration before replacing the active LUT transform. A missing file, malformed carrier, unsupported encoding, invalid domain, or out-of-range strength fails before LUT mode becomes active.

## LUT preset

A LUT preset is a user-owned complete preset. It references an external LUT file by path and SHA-256 and carries the LUT declaration, Effects, and optional LUT Grade. It does not embed LUT pixels or copy the file into an application-owned store.

### Document

LUT presets use preset schema version 8. Film presets and section presets retain schema 7. Effects presets use schema 9 under [Presets](../presets/spec.md#effects-presets).

The library distinguishes the kinds before applying version-specific parsing.

The root contains:

```text
schema_version
name
author? / description?
lut: path, sha256, input_space, output_space, domain_min, domain_max,
     domain_policy, shaper, strength
effects?: halation, diffusion, grain
grade?: LUT Grade group
provenance: implementation_version, model_version
```

The document MUST NOT contain film or print profile references, spectral parameter groups, a route, a mode field, LUT pixels, or an embedded image.

The path is stored exactly as selected. The runtime resolves it without searching for a similarly named file.

The hash is validated before rendering while LUT mode is active. A changed or missing file is a validation failure.

### Apply transaction

One core resolver call validates the LUT preset and its file, then returns a candidate containing:

```text
mode = Lut
lut declaration
effects
grade
stored film groups retained unchanged
```

The GUI commits the candidate only after resolution. The transaction includes the mode switch, the LUT transform replacement, the applied identity, and dependent render invalidation. A failure leaves the preceding mode, controls, identity, and retained image intact.

A manual mode switch is a separate transaction. It preserves all groups and clears the applied identity because no preset describes the new mode. Applying a preset restores identity and MAY switch mode in the same transaction.

A Film preset switches to Film and preserves the stored LUT declaration, Effects, and LUT Grade. An Effects preset switches to Off and preserves the stored film, LUT, and Grade groups. A section preset remains Film-only and MUST be rejected while LUT or Off mode is active.

### Save and export

In LUT mode, **Save preset…** captures the current LUT file path, hash, declaration, Effects, and LUT Grade. It requires a validated imported LUT. It MUST NOT capture film or print profiles, inactive spectral controls, image data, or a render result.

In Off mode, **Save preset…** captures an Effects preset under [Presets](../presets/spec.md#effects-presets).

Preset export and import are document-only. The LUT file is not included. The UI MUST identify the external dependency; a missing file remains a validation failure.

Deleting or renaming a LUT preset MUST NOT delete or modify the LUT file.

## Browser and thumbnails

The browser remains one library and two groups. It presents Film, LUT, and Effects documents in one list without a mode-type category, cue, or filter. Search includes the LUT file name, title, and path; the derived kind remains a resolver fact.

Candidate preparation branches before material resolution:

- Film candidate: existing whole-preset resolver, profile preparation, spectral graph.
- LUT candidate: LUT file hash validation and LUT graph; no film or print preparation.
- Effects candidate: Off graph; no LUT file, film, print, or spectral preparation.
- LUT or Effects candidate with missing data: unavailable state, no render attempt, no fallback.

Candidate identity includes the preset kind, LUT file hash or Effects payload, declaration, Effects, LUT Grade, source generation, execution backend and precision, and presentation dependencies. Kinds are never interchangeable.

LUT thumbnails use the production LUT graph and the same display path. Film thumbnails MUST NOT load a LUT. Effects thumbnails MUST NOT prepare spectral materials or load a LUT.

## CLI

The CLI reuses the same preset resolver and LUT file references.

- A LUT preset in `--preset` selects LUT mode and requires no film or print selector.
- An Effects preset selects Off mode and requires no film, print, or LUT selector.
- A LUT preset mixed with a Film preset, Effects preset, or section preset is rejected.
- `--film`, `--paper`, `--route`, and `--scan-film` conflict with a LUT or Effects preset.
- `lut build` retains its existing export contract. It does not create or save a preset.
- `preset list` reports the kind and LUT file status; `preset show` validates the path and hash and prints the declaration, not LUT pixels.
- LUT preset export and import move only the preset document; the LUT file remains external.
- `process --lut PATH` builds one temporary LUT preset from the cube declaration in the same resolver path. It reads the path, title, domain, shaper declaration, and content hash from the file.
- It uses display-encoded sRGB input and output and clamp by default; `--set` MAY override those declarations, Effects, and LUT Grade.
- `--lut` conflicts with preset selection and with the film and spectral route selectors so one invocation cannot mix modes.
- Dry-run reports the resolved mode. In LUT mode, film-stage and film-grain reports are inactive or omitted from text.
- In Off mode, film, LUT, and LUT Grade reports are inactive; Effects are reported.
- `--set workflow.mode=lut` and the `lut`, `effects`, and `grade` leaves remain available for parameter-only invocations. `lut_look` is rejected.

## Validation and failure

Validation is mode-specific:

| Condition | Decision |
| --- | --- |
| Mode `lut` without a LUT declaration | Reject |
| Mode `film` or `off` with a stored LUT declaration | Ignore the LUT; do not read its file |
| LUT preset without a path or hash | Reject |
| Missing file or hash mismatch while LUT mode is active | Reject; preserve the preceding output |
| LUT preset with film/print references | Reject |
| Film preset with a LUT group | Reject |
| Effects preset with a LUT or profile reference | Reject |
| Simple export with unresolved print target | Reject before writing |
| Simple export with output mix below 1 | Reject before writing |
| Simple export with frame-dependent setup | Reject before writing |
| Unsupported input/output encoding | Reject |
| Inactive mode invalid declaration | Do not validate for the active render; retain it |

A failed export, preset write, or preset application MUST NOT publish a partial file/preset or replace the preceding accepted image.

## Implementation dependencies

The simple export writes one 3D `.cube`. Import uses the same `.cube` carrier and the LUT transform's optional 1D shaper; no second carrier is added.

The following MUST be pinned before implementation crosses its owning boundary:

- canonical runtime and serialization names for `workflow.mode`, `lut`, `effects`, and `grade`;
- Effects preset schema 9 field spellings;
- effect parameter defaults and validation ranges under the LUT transform design;
- CPU/GPU parity for tetrahedral lookup and spatial effects;
- clipped-fraction reporting for the explicit clamp policy.

These are implementation dependencies, not alternative product workflows.
