# LUT workflow

## Authority

| Capability | Owns |
| --- | --- |
| This capability | LUT file export and import, LUT-stage actions, and LUT preset lifecycle |
| [Render modes](../render-mode/spec.md) | Film, LUT, and Off selection; switching; stage activation; inactive-value retention |
| [LUT transform](../lut-transform/spec.md) | LUT declaration, domain policy, fixed evaluation order, and Diffusion, Halation, and Grain semantics |
| [LUT delivery](../lut-delivery/spec.md) | Deterministic spectral LUT baking, bundle topology, QA, and artifact publication |
| [Presets](../presets/spec.md) | Preset kinds, library, names, groups, carriers, discovery, writes, browser interaction, and thumbnails |
| [Film chain](../film-chain/spec.md) | Spectral film, print, and scan simulation |
| [Tone and color](../finishing/spec.md) | The shared grade response |
| [GUI workflow](../gui-workflow/spec.md) | Mode-selector placement and LUT and Effects stage ownership |
| [CLI](../cli/spec.md) | Command syntax, preset selectors, and canonical parameter discovery |

## Scope

A **film simulation** renders the spectral film chain. A **LUT workflow** imports an external LUT transform for LUT mode. **Off mode** renders the Effects stage without a LUT.

Export and import are separate operations:

```text
film simulation
  -> Export film simulation as .cube… -> .cube file
  -> Import LUT… -> LUT mode
  -> optional Save preset
```

Exporting a LUT MUST NOT import it, apply it, create a LUT preset, switch mode, or change the current film render. Using a LUT MUST begin with an explicit import.

A LUT is a color transform, not a film simulation. The product MUST NOT present an exported or imported LUT as a film stock, a measured film response, or a spectral claim.

## Mode integration

[Render modes](../render-mode/spec.md) owns the global Film / LUT / Off selector and switching.

- LUT mode activates the LUT and Effects stages. It deactivates Film, Print, Scan, spectral camera controls, Scan Tone & Color, output mix, and CMYK.
- Film mode deactivates the LUT and Effects stages.
- Off mode deactivates Film, Print, Scan, LUT, LUT Grade, spectral camera controls, Scan Tone & Color, output mix, and CMYK.
- Input processing and Output color management remain active in every mode.
- Switching modes preserves all stored groups.
- Selecting LUT without a usable declaration requests import or replacement. Cancelling or failing leaves the preceding mode active.

A manual mode switch clears the applied-preset identity. Applying a Film, LUT, or Effects preset restores identity and selects its mode in the same transaction.

The header identifies the active mode and the active film pair, LUT file, or Effects preset as read-only context.

## Export LUT

The Film stage MUST expose **Export film simulation as .cube…**. The action writes a LUT file; it MUST NOT offer a "use now", "open in LUT mode", or "save as preset" option.

The export form MUST expose:

- the destination `.cube` file;
- the resolution;
- the input and output encodings;
- the fixed pointwise transform that will be written.

The default encodings are display-encoded sRGB input and output. The supported encodings are linear sRGB, display-encoded sRGB, and display-encoded Display P3.

### Export constraints

The simple export MUST write one complete 3D `.cube` file.

- The film simulation MUST resolve one print target.
- Output mix amount MUST be exactly 1.
- An active frame-dependent negative setup MUST fail the export.
- The input and output encodings MUST be LUT-transform supported.
- Optical diffusion, halation, and stochastic grain are not representable in a pointwise LUT. They MUST be neutralized and named in the export result.
- Eligible Tone & Color follows [LUT delivery](../lut-delivery/spec.md) and is included once at its existing boundary.

The export result MUST identify the encodings and the neutralized operations. The `.cube` file itself does not carry the application's mode or preset metadata.

The existing LUT delivery bundle remains a separate advanced publication path. It is not automatically imported or saved as a preset.

### Export failure

A failed export MUST preserve the current film image, active controls, mode, and preset library. It MUST NOT leave a partial `.cube` file at the destination.

## Import LUT

The LUT stage MUST expose **Import LUT…**, **New LUT from cube…**, and **Replace LUT…**. Each MUST load one 3D `.cube` file and require an explicit declaration of:

- the LUT file path and content hash;
- the input and output encodings;
- the domain minimum and maximum;
- the domain policy;
- the optional shaper declaration;
- LUT strength.

The import form MUST keep the input and output encodings, domain policy, and strength visible. It MAY place the content hash, domain values, and shaper declaration under one advanced disclosure.

The form MUST state that `.cube` files do not carry color-space metadata. The default sRGB spaces MUST be presented as an assumption, not as a file fact.

The domain policy MUST default to clamp for a new import or a replacement. The form MUST expose fail as a strict option and explain both policies inline.

A new LUT starts at strength one, with Diffusion, Halation, LUT Grade, and Grain inactive. Importing MUST NOT save a preset.

**Replace LUT…** MUST preserve the current LUT strength, LUT Grade `active` choice and values, and effect `active` choices and values. The replacement form MUST re-declare the selected cube's input and output encodings, domain, domain policy, and shaper.

The replacement cube supplies its path, content hash, and title. A separate new-LUT action MUST reset strength and LUT Grade values.

The import preview MUST provide the neutral gray ramp and color checker required by [LUT transform](../lut-transform/spec.md#comparison-and-preview). It MUST NOT auto-fit exposure, white balance, or contrast. The form MUST state that this preview checks the cube declaration, not the current photograph.

A missing file, malformed carrier, unsupported encoding, or invalid domain MUST fail before LUT mode becomes active. A failed import MUST preserve the current mode and retained image.

## LUT presets

A LUT preset is a complete preset for LUT mode. It is user-owned; built-in LUT presets and LUT section presets are outside this contract.

### Document

The LUT preset MUST use the preset library's name rules, groups, carriers, discovery, locks, and atomic writes.

It MUST carry:

- the preset schema version;
- `name` and optional `author` and `description`;
- the LUT file path;
- the LUT file content hash;
- the input and output encodings;
- the domain minimum and maximum;
- the domain policy;
- the optional shaper declaration;
- strength;
- optional `effects` for Diffusion, Halation, and Grain;
- optional `grade` for LUT Grade;
- `provenance`, the SpektraFilm implementation and model versions.

It MUST NOT carry film or print profile references, spectral parameter groups, a film route, a mode field, LUT pixels, or an embedded image.

The preset kind is derived from the carried shape. A document carrying both a LUT declaration and film or print references MUST be rejected.

Unknown fields, missing required declaration fields, and a mixed kind MUST fail validation.

The LUT file is external to the preset. The preset stores its path and hash but not its pixels.

A missing file or hash mismatch MUST fail validation. The product MUST NOT substitute another file.

### Apply

Applying a LUT preset MUST validate the preset and its LUT file before changing the active mode.

- It MUST switch to LUT mode.
- It MUST replace the LUT declaration, Effects, and LUT Grade together.
- It MUST preserve stored film, print, scanner, and route values.
- It MUST create exactly one undoable transaction and schedule a Preview.

A missing file, hash mismatch, invalid declaration, or malformed file MUST fail the application without changing mode, controls, applied identity, or retained image.

Applying a Film preset MUST switch to Film mode and preserve the stored LUT declaration, Effects, and LUT Grade. Applying an Effects preset MUST switch to Off mode and preserve the stored film, print, LUT, LUT Grade, scanner, and route values.

A section preset remains Film-only and MUST NOT be applied while LUT or Off mode is active.

A LUT preset MUST NOT be mixed with a Film preset, an Effects preset, or a section preset in one CLI preset set.

### Save and library actions

**Save preset…** captures the active mode.

- In Film mode, it captures a Film preset.
- In LUT mode, it captures the current LUT file reference, declaration, Effects, and LUT Grade. It requires a validated imported LUT.
- In Off mode, it captures an [Effects preset](../presets/spec.md#effects-presets).
- With no validated LUT in LUT mode, the save action MUST be unavailable with an explanation.
- Saving a LUT preset MUST NOT capture film or print profiles, inactive spectral controls, the current image, or a render result.

Name conflict, replacement, rename, duplicate, delete, import, export, and reveal follow the existing preset library contract. Deleting or renaming a LUT preset MUST NOT delete or modify its LUT file.

A LUT preset document is exported or imported without its LUT file. The UI MUST state that the LUT file remains an external dependency and MUST be available at the stored path.

### Browser

The preset browser MUST present Film, LUT, and Effects presets in one list under the same `Built-in` and `My Presets` groups. It MUST NOT introduce separate Film, LUT, or Effects categories or a mode-type filter; a card MUST identify its content by its name and carried context, not by a mode badge.

- Search MUST match a LUT preset's name, referenced LUT file name or title, and LUT path.
- Applying a LUT card MUST switch to LUT mode; applying a Film card MUST switch to Film mode; applying an Effects card MUST switch to Off mode.
- An applied LUT preset remains identifiable when search hides its row.
- A missing or changed LUT file MUST appear as a library fault or card-unavailable state. It MUST NOT show a stale thumbnail as current.
- A missing LUT file is not an empty library.

Thumbnails for a LUT preset MUST render through LUT mode and MUST NOT prepare spectral materials. Film thumbnails MUST use Film mode and MUST NOT load a LUT. Effects thumbnails MUST use Off mode and MUST NOT prepare spectral materials or load a LUT.

A thumbnail result MUST retain the preset kind and its carried declaration, Effects, and LUT Grade in its candidate identity.

### Schema and compatibility

LUT presets MUST use preset schema version 8. Film presets and section presets retain schema version 7. Effects presets use schema version 9.

The versions coexist by document kind.

A version 8 document without a valid `lut` group MUST be rejected. A version 8 document MUST NOT carry film or print profile references. Readers that do not understand the LUT kind MUST reject the document rather than approximate it as a Film preset.

## UI

The mode selector belongs to [Render modes](../render-mode/spec.md). The LUT workflow surface MUST expose:

- LUT import, new LUT, and replace;
- the declared input and output encodings, domain, policy, shaper, and strength;
- LUT Grade;
- a clear empty state with **Import LUT…**.

The LUT stage MUST identify the transform as non-spectral. Action labels MUST describe their outcome: import a cube as a LUT, replace the LUT, or export the film simulation as a `.cube`.

The LUT stage MUST show all three domain axes. It MUST show a persistent warning with the clipped percentage when the latest accepted clamp render clips pixels. The warning MUST appear next to the LUT controls and MUST NOT rely on a truncated header or hover text.

The Effects stage MUST expose independent Diffusion, Halation, and Grain controls. It MUST be active in LUT and Off and inactive in Film. LUT Grade belongs to the LUT stage and is active only in LUT.

Film, Print, and Scan stages MUST remain reachable in LUT and Off with their headings, stored values, an explanation, and disabled bodies collapsed by default. Expanding an inactive stage MUST NOT execute or enable it.

The retained film Tone & Color module remains in Scan and is inactive in LUT and Off. It MUST NOT be duplicated into the LUT or Effects stage.

Output color space, encoding, and gamut processing remain active. Output mix and CMYK ink style remain inactive film-chain operations and retain their stored values.

**Export film simulation as .cube…** belongs to the Film stage. It MUST NOT be presented as an editable LUT control or as a way to run film stages inside LUT mode.

## CLI

The canonical parameter language MUST expose the mode, LUT declaration, Effects, LUT Grade, and preset selection.

- `process --preset SELECTOR` MUST resolve a LUT preset and render LUT mode. It MUST NOT require film or print profile selectors for that preset.
- `process --lut PATH` MUST import one `.cube` as the LUT transform in one invocation, reading the title, domain, shaper declaration, and content hash from the file. Its input and output encodings MUST default to display-encoded sRGB, its domain policy MUST default to clamp, and `--set` MUST remain available for declaration and effect edits.
- `--lut` MUST conflict with `--preset`, `--film`, `--paper`, `--route`, `--scan-film`, and `--feature-profile`. An explicit non-LUT `workflow.mode` edit MUST fail with `--lut`.
- A LUT preset MUST NOT be combined with a Film preset, an Effects preset, a section preset, `--film`, `--paper`, `--route`, or `--scan-film` in one invocation.
- Dry-run JSON and text MUST report the resolved mode, LUT declaration, Effects, and LUT Grade. In LUT mode they MUST NOT report film grain or other spectral stage controls as active.
- In Off mode they MUST report no film, LUT, or LUT Grade, and MUST report the active Effects.
- LUT canonical leaves MUST remain addressable through `--set` and discoverable through `describe`. Legacy `lut_look` paths MUST NOT be accepted.
- `lut build` retains its existing LUT publication contract. It MUST NOT create or save a preset.
- `preset list` MUST distinguish Film, LUT, Effects, and section kinds. `preset show` MUST validate the LUT file path and hash without reading LUT pixels into the document.
- LUT preset export and import are document-only; the LUT file remains external.

Exact command names and option spellings belong to the [CLI contract](../cli/spec.md).

## Persistence and failure

Runtime parameters, GUI state, render recipes, and presets MUST retain the mode plus the `lut`, `effects`, and `grade` groups. A saved LUT MUST retain its file path, hash, declaration, Effects, LUT Grade, and provenance. A LUT file is never embedded.

A LUT preset is invalid while its file is missing or its hash differs. It MUST NOT fall back to a Film preset, another LUT, a default LUT, or a neutral transform.

A failed preset application or export MUST preserve the preceding accepted image, mode, controls, applied identity, and existing library files.

A Film preset is not made invalid by a missing LUT file. A LUT preset is not made invalid by an invalid or unavailable film profile. An Effects preset requires neither.

Each mode validates only its active declaration.

## Acceptance

1. Export a `.cube` from Film mode; import it explicitly; confirm that Film mode and the preset library are unchanged until import, and that import enters LUT mode only after a valid declaration.
2. Save and apply a LUT preset; confirm that it switches LUT mode and preserves stored film, print, scanner, and route values. Confirm that it replaces Effects and LUT Grade, creates one Undo transaction, and fails without fallback when the LUT file is missing or changed.
3. In Off mode, confirm that Effects render without a LUT or LUT Grade, a missing stored LUT does not block output, and an Effects preset applies Off mode without loading spectral materials or a LUT.
