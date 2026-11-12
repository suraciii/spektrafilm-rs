# LUT workflow

## Authority

| Capability | Owns |
| --- | --- |
| This capability | LUT file export, LUT file import, workflow switching, and LUT preset lifecycle |
| [LUT look](../lut-look/spec.md) | LUT rendering semantics, LUT declaration, domain policy, fixed effect order, CPU/GPU evaluation |
| [LUT delivery](../lut-delivery/spec.md) | Deterministic spectral LUT baking, bundle topology, QA, artifact publication |
| [Presets](../presets/spec.md) | Preset library, names, groups, carriers, discovery, writes, browser interaction, thumbnails |
| [Film chain](../film-chain/spec.md) | Spectral film, print, and scan simulation |
| [Tone and color](../finishing/spec.md) | The shared grade response |
| [GUI workflow](../gui-workflow/spec.md) | Workflow selection, stage ownership, and inspector surfaces |
| [CLI](../cli/spec.md) | Command syntax, preset selectors, and canonical parameter discovery |

## Scope

A **spectral film workflow** renders the existing film chain. A **LUT workflow** renders an imported LUT look. They are peers.

Export and import are separate operations:

```text
spectral film look
  -> Export LUT… -> .cube file
  -> Import LUT… -> LUT workflow
  -> optional Save preset
```

Exporting a LUT MUST NOT import it, apply it, create a LUT preset, switch workflow, or change the current film render. Using a LUT MUST begin with an explicit import.

A LUT is a color transform, not a film simulation. The product MUST NOT present an exported or imported LUT as a film stock, a measured film response, or a spectral claim.

## Workflow selection

A workflow selector MUST offer exactly two choices: **Film** and **LUT**. It has one owner: the Look stage.

The header workflow summary is read-only and MAY locate the Look stage. It MUST NOT duplicate the editor.

The film route selector remains a separate control inside the film workflow. In LUT mode it is disabled and displays the retained route; its reset MUST remain disabled while the route is inactive.

- Selecting Film MUST make the spectral stages active and the LUT look controls inactive.
- Selecting LUT MUST make the LUT look controls active. Film, Print, and Scanner controls remain stored but inactive.
- Switching workflows MUST preserve both stored control sets and MUST NOT execute the inactive one.
- A manual workflow switch is one reversible adjustment.
- A manual workflow switch MUST clear the applied-preset identity. The stored controls remain available; re-applying a preset restores identity.
- Applying a preset that describes the other workflow MUST switch to that workflow as part of the same transaction.

The header MUST identify the active workflow and the active film pair or LUT preset.

## Export LUT

The film workflow MUST expose **Export LUT…**. The action writes a LUT file; it MUST NOT offer an "use now", "open in LUT workflow", or "save as preset" option.

The export form MUST expose:

- the destination `.cube` file;
- the resolution;
- the input and output encodings;
- the fixed pointwise transform that will be written.

The default encodings are display-encoded sRGB input and display-encoded sRGB output. The supported encodings are linear sRGB, display-encoded sRGB, and display-encoded Display P3.

### Export constraints

The simple export MUST write one complete 3D `.cube` file.

- The film look MUST resolve one print target.
- Output mix amount MUST be exactly 1.
- An active frame-dependent negative setup MUST fail the export.
- The input and output encodings MUST be LUT-look supported.
- Optical diffusion, halation, and stochastic grain are not representable in a pointwise LUT. They MUST be neutralized and named in the export result.
- Eligible Tone & Color follows [LUT delivery](../lut-delivery/spec.md) and is included once at its existing boundary.

The export result MUST identify the encodings and the neutralized operations. The `.cube` file itself does not carry the application's workflow or preset metadata.

The existing LUT delivery bundle remains a separate advanced publication path. It is not automatically imported or saved as a preset.

### Export failure

A failed export MUST preserve the current film image, active controls, workflow, and preset library. It MUST NOT leave a partial `.cube` file at the destination.

## Import LUT

The LUT workflow MUST expose **Import LUT…**. It MUST load one 3D `.cube` file and require an explicit declaration of:

- the LUT file path and content hash;
- the input and output encodings;
- the domain minimum and maximum;
- the domain policy;
- the optional shaper declaration;
- LUT strength.

A newly imported LUT starts at strength one with diffusion, halation, grade, and grain disabled. Importing MUST NOT save a preset.

The import preview MUST provide the neutral gray ramp and color checker required by [LUT look](../lut-look/spec.md#comparison-and-preview). It MUST NOT auto-fit exposure, white balance, or contrast.

A missing file, malformed carrier, unsupported encoding, or invalid domain MUST fail before the LUT look becomes active. A failed import MUST preserve the current workflow and retained image.

## LUT presets

A LUT preset is a whole-look preset for the LUT workflow. It is user-owned; built-in LUT presets are outside this contract. LUT section presets are also outside this contract.

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
- optional halation, diffusion, and grain groups;
- optional grade;
- `provenance`, the SpektraFilm implementation and model versions.

It MUST NOT carry film or print profile references, spectral parameter groups, a film route, a workflow selection field, LUT pixels, or an embedded image.

The preset kind is derived from the carried shape. A document carrying both a LUT declaration and film or print references MUST be rejected. Unknown fields, missing required declaration fields, and a mixed kind MUST fail validation.

The LUT file is external to the preset. The preset stores its path and hash but not its pixels. A missing file or hash mismatch MUST fail validation; the product MUST NOT substitute another file.

### Apply

Applying a LUT preset MUST validate the preset and its LUT file before changing the active workflow.

- It MUST switch to the LUT workflow.
- It MUST replace the LUT declaration, effects, and grade together.
- It MUST preserve stored film, print, scanner, route, and other inactive controls.
- It MUST create exactly one undoable transaction and schedule a Preview.

A missing file, hash mismatch, invalid declaration, or malformed file MUST fail the application without changing workflow, controls, applied identity, or retained image.

Applying a film look preset while in LUT mode MUST switch to the film workflow and preserve the stored LUT look. A section preset remains film-only and MUST NOT be applied while LUT mode is active.

A LUT preset MUST NOT be mixed with a film look preset or section preset in one CLI preset set.

### Save and library actions

**Save preset…** captures the active workflow.

- In the film workflow, it retains the existing film look capture.
- In the LUT workflow, it captures the current LUT file reference, declaration, effects, and grade. It requires a validated imported LUT.
- With no validated LUT, the LUT save action MUST be unavailable with an explanation.
- Saving a LUT preset MUST NOT capture film or print profiles, inactive spectral controls, the current image, or a render result.

Name conflict, replacement, rename, duplicate, delete, import, export, and reveal follow the existing preset library contract. Deleting or renaming a LUT preset MUST NOT delete or modify its LUT file.

A LUT preset document is exported or imported without its LUT file. The UI MUST state that the LUT file remains an external dependency and MUST be available at the stored path.

### Browser

The preset browser MUST present film look presets and LUT presets in one list under the same `Built-in` and `My Presets` groups. It MUST NOT introduce separate Film and LUT categories or a workflow-type filter; a card MUST identify its content by its name and its carried material or LUT file context, not by a workflow type badge.

- Search MUST match a LUT preset's name, referenced LUT file name or title, and the LUT path.
- Applying a LUT card MUST switch to the LUT workflow. Applying a film card MUST switch to the film workflow.
- An applied LUT preset remains identifiable when search hides its row.
- A missing or changed LUT file MUST appear as a library fault or card-unavailable state. It MUST NOT show a stale thumbnail as current.
- A missing LUT file is not an empty library.

Thumbnails for a LUT preset MUST render through the LUT workflow and MUST NOT prepare spectral materials. Film thumbnails MUST continue to use the spectral workflow and MUST NOT load a LUT. A thumbnail result MUST retain the preset kind and LUT file hash in its candidate identity.

### Schema and compatibility

LUT presets MUST use preset schema version 8. Film look presets and section presets retain schema version 7 and remain valid without migration. The two versions coexist by document kind.

A version 8 document without a valid LUT declaration MUST be rejected. A version 8 document MUST NOT carry film or print profile references. Readers that do not understand the LUT kind MUST reject the document rather than approximate it as a film look.

## UI

The LUT workflow surface MUST expose:

- LUT import and replace;
- the declared input and output encodings, domain, policy, shaper, and strength;
- independent diffusion, halation, grade, and grain controls;
- a clear empty state with **Import LUT…**.

The LUT stage MUST identify the look as non-spectral.

Film, Print, and Scanner stages MUST remain reachable in the LUT workflow with their headings, stored values, an explanation, and disabled bodies collapsed by default. Expanding an inactive stage MUST NOT execute or enable it.

The Look stage remains active for the workflow selector and LUT look, including its independent grade group. The retained film Tone & Color module remains in Scan and is inactive in LUT mode; it MUST NOT be duplicated into Look.

LUT diffusion, halation, grade, and grain are independent from the film-chain controls of the same names. Sharing a module vocabulary MUST NOT imply shared values or a shared execution domain.

Output color space, encoding, and gamut processing remain active. Output mix and CMYK ink style remain inactive film-chain operations and retain their stored values.

**Export LUT…** belongs to the film workflow. It MUST NOT be presented as an editable LUT control or as a way to run film stages inside LUT mode.

## CLI

The canonical parameter language MUST expose the workflow mode, LUT declaration, effects, grade, and LUT preset selection.

- `process --preset SELECTOR` MUST resolve a LUT preset and render the LUT workflow. It MUST NOT require film or print profile selectors for that preset.
- A LUT preset MUST NOT be combined with a film look preset, a section preset, `--film`, `--paper`, `--route`, or `--scan-film` in one invocation.
- LUT canonical leaves MUST remain addressable through `--set` and discoverable through `describe`.
- `lut build` retains its existing LUT publication contract. It MUST NOT create or save a preset.
- `preset list` MUST distinguish film look, LUT preset, and section kinds. `preset show` MUST validate the LUT file path and hash without reading LUT pixels into the document.
- LUT preset export and import are document-only; the LUT file remains external.

Exact command names and option spellings belong to the [CLI contract](../cli/spec.md).

## Persistence and failure

Runtime parameters, GUI state, render recipes, and presets MUST retain the workflow mode and the LUT look declaration. A saved LUT look MUST retain its file path, hash, declaration, effects, grade, and provenance. A LUT file is never embedded.

A LUT preset is invalid while its file is missing or its hash differs. It MUST NOT fall back to a film look, another LUT, a default LUT, or a neutral transform. A failed preset application or export MUST preserve the preceding accepted image, workflow, controls, applied identity, and existing library files.

A film look preset is not made invalid by a missing LUT file. A LUT preset is not made invalid by an invalid or unavailable film profile. Each workflow validates only its active declaration.

## Acceptance

1. Export a LUT from a film look. The product writes one `.cube` file and changes neither the active workflow nor the preset library.
2. Import the exported `.cube` with an explicit declaration. The LUT workflow becomes active only after the user imports it; no preset is created automatically.
3. Save the imported LUT as a preset, apply it, and confirm that it switches workflow, preserves inactive film values, and creates one Undo transaction.
4. Confirm that a missing or changed LUT file fails before output, preserves the current image, and never substitutes a film look or another LUT.
5. Confirm that a LUT preset document import/export does not include LUT pixels and states the external file dependency.
6. Confirm that LUT thumbnails do not prepare spectral materials and film thumbnails do not load a LUT.
7. Confirm that a LUT preset and a film preset cannot be mixed in one CLI invocation, while `process --preset` can run a LUT preset without film or print selectors.
8. Confirm that manual workflow switching preserves both control sets and clears only the active applied-preset identity.
