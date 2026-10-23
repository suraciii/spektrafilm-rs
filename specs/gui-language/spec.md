# GUI product language

## Authority and boundary

This specification owns the language that the GUI presents to a person. It owns visible labels, choice labels, help text, workflow descriptions, status wording, and the separation between product language and technical identifiers.

[`CONTEXT.md`](../../CONTEXT.md) owns the domain glossary. [`Film chain`](../film-chain/spec.md) owns scanner operations, scanner stage boundaries, scan-output naming, and conversion-control naming. [`GUI workflow`](../gui-workflow/spec.md) owns control inventory, stage and surface ownership, editing behavior, and persisted-state behavior. [`GUI presentation`](../gui-presentation/spec.md) owns row and surface presentation.

The GUI must use the domain glossary and the owning operation specification as its vocabulary source. This specification must not redefine physical terms with shorter synonyms.

## Language layers

The GUI presents three language layers.

### Product language

Visible labels must name the current product concept or physical operation. A visible label must not expose a serialized path, snake-case enum, backend name, or implementation history unless that identifier is itself the product concept.

### Technical help

Tooltips and help text must preserve the exact technical meaning, domain, units, stage, calibration status, and limitations of the control. Technical help may use parameter paths and serialized values when that information helps a person understand the control.

### Serialized and diagnostic language

State keys, enum values, route strings, backend names, precision names, and telemetry identifiers must remain exact in state files, reports, diagnostics, and specifications. A presentation label must not change these identifiers or imply an alias.

A control may show a technical identifier in a tooltip or diagnostic detail. It must not use a second visible label to hide a persisted value's actual meaning.

## Physical vocabulary

The GUI must use these terms with the meanings defined by [`CONTEXT.md`](../../CONTEXT.md):

- `record exposure` means exposure received by one film or photographic-paper record before development.
- `record density` means optical density formed in one developed record before an output scan interprets it.
- `scanned medium` means developed film or photographic paper read for digital output.
- `scan sensor` means the colorimetric spectral response used to acquire transmitted or reflected light from a scanned medium.
- `scan capture` means RGB readings acquired by the scan sensor.
- `scanned density` means density derived from one channel of a scan capture relative to that channel's clear reference.
- `clear film base` means the unexposed base of the scanned medium, which reads as the output white reference of a scan.
- `negative conversion` means interpretation of a developed negative as a positive image.
- `exposure reconstruction` means negative conversion that estimates relative exposure through the selected film response.
- `digital output` means the color space, transfer encoding, geometry, and file representation used to deliver an image.

The GUI must not use the following substitutions when they would change a physical boundary:

- `capture response` for `scan sensor`;
- `interpretation` for `scan output` when the choices describe native polarity;
- `negative interpretation` for `negative conversion`;
- `simulation color space` for `output color space`;

A shorter label is permitted only when it preserves the same operation, domain, stage, and calibration claim.

## Scanner labels

Scanner labels must follow the scan-output and neutral-scan-model contracts in [`Film chain`](../film-chain/spec.md).

The GUI must display:

| Runtime concept | GUI label or choice |
|---|---|
| `scanner.scan_output = direct_scan` | `Preserve medium polarity` |
| `scanner.scan_output = negative_to_positive` | `Negative to positive` |
| `scanner.lens_blur` | `Lens blur (readout optics)` |
| `scanner.unsharp_mask` | `Unsharp mask (digital sharpening)` |
| `film_render.convert.scan_illuminant` | `Scan illuminant` in Input and capture → Convert |

The GUI must not offer a scanner light control. The readout illuminant is a product constant selected by the material category.

The Convert scan-illuminant help must identify the light used to acquire an imported RGB negative. It must distinguish that declaration from the fixed product readout illuminant and the Camera capture illuminant. Conversion-style choices must follow the [lab scan style-name contract](../film-chain/lab-scan/spec.md#tone-and-style-names). The GUI must not present `neutral` as a fixed built-in style or expose automatic setup as a separate control.

The GUI must explain that `negative conversion` interprets a developed negative as a positive image through the film's own density response. It must not describe negative conversion as a file conversion, a scan finish, or a positive-scan alias.

The GUI may add a short explanatory line grouping the scanner controls by acquisition and conversion. Such a line is explanatory only; it must not rename the runtime concepts or create a second control taxonomy.

## Color and output labels

The GUI must distinguish these boundaries:

- `input color space` and input transfer decoding describe the declared source image;
- `output color space` and output transfer encoding describe the runtime digital output;
- `saving color space` and saving transfer encoding describe file conversion;
- `display transform` and display ICC selection describe viewer-only composition.

The GUI must keep `CCTF` when it names the transfer-function setting. The first relevant help text must expand it as `color component transfer function`. The GUI must not replace it with a phrase that hides whether the samples are encoded or linear.

The GUI must call the terminal modeled appearance `CMYK ink style`. Its help must state that the result remains three-channel RGB and is not a press proof or a separation. `Strength` is reserved for `cmyk.strength`; the control must not expose paper, ink, TAC, GCR, UCR, halftone, texture, or registration parameters.

The GUI must keep `Simulation blend` as the name of the full-result/prepared-source mix. It must not rename it to opacity, intensity, saturation, or strength.

The GUI must keep `Print viewing glare`, `Lens blur (readout optics)`, `Unsharp mask (digital sharpening)`, material grain, and Procedural post-scan grain distinct. Similar visual effects must not share a label that implies the same physical stage.

The unified Grain selector must retain the names Spektra, Dye, and Procedural. Its choice descriptions must identify Spektra and Dye as material grain and Procedural as post-scan grain appearance. Procedural help must identify Film Resolution as part of that appearance algorithm, not scanner lens resolution.

Camera diffusion and Enlarger diffusion must retain their distinct optical owners. Halation must remain a film-domain effect. Preflash exposure and its independent Magenta and Yellow filter shifts must remain printing controls. Print viewing glare help must identify readout stray light, not preflash or a change in material density. Visible placement follows [stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership), not serialized section names.

## Viewer language

The viewer layer labels remain:

```text
Input
Output
Paper back
```

`Paper back` is a viewer-only virtual rendering generated from the retained image. It is not a second editable material stage and it must not alter retained, saved, or exported pixels. Help must state this boundary before any future label change is considered.

The GUI must not rename `Paper back` to `Paper surface` or another physical term without a domain decision that defines the represented viewing geometry and updates the owning specification.

The viewer must distinguish:

- `fit`: return to fitted zoom without changing pan;
- `reset view`: return to fitted zoom and recenter pan;
- exact zoom percentages: source-pixel-to-device-pixel scale;
- `compare`: before/after viewer composition only;
- `select`: the Shift or secondary-button drag rectangle that zooms to its region on release.

Viewer hints must name the gestures in this vocabulary: click zooms at the pointer, primary-button drag pans, Shift or secondary-button drag selects, and double click fits. A hint must not claim an effect on saved or exported pixels.

## Action language

Actions must use one stable verb and one explicit target:

| Action | Meaning |
|---|---|
| `Open` | Open image intake choices without changing their RGB and RAW meanings. |
| `Undo` | Restore the preceding accepted edit transaction. |
| `Redo` | Reapply the next undone edit transaction. |
| `Settings` | Open session, display, and diagnostics settings. |
| `Export` | Open the independent Export dialog without starting a render. |
| `Select RGB image…` | Choose and load a standard raster image. |
| `Select RAW file…` | Choose and load a camera RAW file through the RAW path. |
| `Preview` or `PREVIEW` | Render the bounded preview using current runtime controls. |
| `Scan` or `SCAN` | Render the original working resolution using current runtime controls. |
| `Save…` or `SAVE` | Write retained output without rerendering. |
| `Export file…` | Start independent export rendering and writing. |
| `Save preset…` | Capture the current look in a preset. |
| `Save current to file` | Write GUI state to a selected state file. |
| `Load from file` | Load GUI state from a selected state file. |
| `Save current as default` | Replace the startup GUI state. |
| `Restore factory default` | Restore factory GUI state and remove the saved startup default. |

Action labels may use uppercase casing. Tooltips and status messages must preserve the meanings in this table.

`Save` and `Export` must never be described as the same operation. Save uses retained output; Export rerenders independently.

Undo and Redo wording must follow [edit history](../gui-workflow/spec.md#edit-history). Their help must not imply that they reverse file writes or preset-library writes. Opening Settings or Export must not be described as changing pipeline stages.

## Header and status language

The header must show the selected film, selected print material, exact workflow route, and one operation status surface. The exact route string remains available because it identifies the physical sequence and is required for diagnostics.

The header may add a direct physical description beside the exact route. The description must translate the route without inventing a look category. For example, `Film → print → scan` may describe `input > film > print > scan`; `Film print look` must not replace it.

Normal success status must describe the accepted operation and its dimensions or target. Backend and precision detail belongs in Diagnostics unless it explains an actionable limitation. A diagnostic label such as `f64 reference` must not be the only normal success message.

A running render's status names the operation and the executing work unit in product language. Work-unit wording must name the physical operation, for example `Film development`, `Grain`, `Printing`, and `Scanning`. It must not expose serialized work-unit identifiers. A running render without a named work unit shows its name alone.

A status that reports route or profile normalization must identify which stored controls became inactive or were cleared. It must not claim that a value was applied when the selected route cannot execute it.

## State, reset, and search language

`Modified` must mean that a row or module differs from its exact factory reset projection. It must not mean that the value differs from the selected preset or startup default. Factory comparison follows [presentation design](../gui-presentation/design.md#factory-projection-and-modified-state). Reset behavior follows [complete control access](../gui-workflow/spec.md#complete-control-access).

A row reset must be named `Reset to factory` with the row target available in accessible text or help. A module reset must name the module and its factory baseline. A section-preset menu whose stored payload spans more than one module must explicitly name its full scope. It must not describe the wider action as a module-only reset.

The inspector stage names must be `Input and capture`, `Film`, `Print`, `Scan`, and `Output`. A stage jump must identify the heading it reaches, not a page it replaces. `Settings` and `Export` must name dialogs rather than additional stages. The interface must not use MAIN, FILM, PRINT, ADVANCED, or CONFIG as page labels.

`Search controls` must label the global control locator under [control search](../gui-workflow/spec.md#control-search). Help must state that it matches control labels, module names, and canonical parameter paths across editing surfaces. It must not describe search as a filter limited to the current stage. A no-match message must include the entered text and must not imply that parameter values changed.

A result must name the control, its owning module, and its stage or dialog destination. Canonical paths may appear as secondary technical detail. The action must describe navigation to the real editor, not application of a value. A Settings or Export result must explicitly name that dialog. A result must not present a second editable copy.

An inactive reason must identify the route, medium, or capability condition that prevents use. It must distinguish a retained value from one that was cleared by required normalization. A disabled value must not be labeled as applied when its stage does not execute.

## Help-text rules

Help text must:

1. state the physical operation or data boundary;
2. state the stage that owns the control;
3. state route or medium conditions when the control is conditional;
4. identify approximate, modeled, or uncalibrated behavior;
5. state Save/Export/viewer-only isolation when relevant;
6. state limitations before promising a visual result;
7. use exact units and canonical parameter terminology.

Help text must not:

- promise recovery of information absent from the source;
- call a modeled convention measured or physical;
- call a viewer decoration a saved material;
- call a scanner sensor a look preset;
- use a former implementation name as a product concept;
- use color as the only state cue.

## Acceptance

1. Every scanner label and choice matches the scanner contracts and preserves the serialized value.
2. Every visible technical term has a tooltip or linked documentation definition when its meaning is not self-evident.
3. Search result labels, accessible labels, state keys, enum values, and diagnostics remain distinct layers.
4. Save, Export, retained output, and viewer-only state remain distinguishable in labels and help.
5. A terminology review against `CONTEXT.md` rejects any rename that changes operation, domain, stage, or calibration meaning.
6. Presentation-only language changes must preserve headless behavior, layout accessibility, Save/Export pixels, and persisted-state contracts.
