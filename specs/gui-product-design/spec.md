# GUI product design

## Purpose

This specification defines the product-facing GUI design direction for SpektraFilm. It improves comprehension and discoverability without replacing the physical film workflow, the five-page ownership model, or the existing retained-output and viewer boundaries.

The design must make the following workflow legible:

```text
source image → exposure → development → print or scan → digital output
```

The GUI must not reduce this workflow to an undifferentiated look, effect, or filter.

## Product boundaries

The GUI must retain these boundaries:

- MAIN owns image intake, workflow selection, profiles, camera, enlarger, scanner, output, and export.
- FILM owns film material and film-domain effects.
- PRINT owns photographic-print material and print-domain effects.
- ADVANCED owns processing controls that most sessions do not change.
- CONFIG owns session, diagnostics, GUI, and display settings.
- The viewer is the primary surface for retained input, output, and Paper back presentation.
- The control panel is the only surface that edits parameter values.
- The preset browser remains a separate collapsible region.
- Operation status appears on one surface: the window header.

A product-design change must not add a second editable surface for an existing persisted value.

## Information hierarchy

The GUI must present information in this order:

1. current materials and physical workflow;
2. retained image and viewer state;
3. page and section ownership;
4. current parameter values and factory-state differences;
5. technical execution detail and diagnostics.

The header must not force users to decode serialized route syntax before they know the selected workflow. The exact route must remain available, and the GUI may add a direct physical description beside it or in its help text.

The control panel must not expose backend, precision, or internal cache terminology as the primary explanation of a normal photographic control.

## Workflow presentation

The workflow selector must offer exactly the routes accepted by the workflow validator. Each route choice must have:

- its exact serialized route value;
- a direct physical description;
- a tooltip that identifies the scanned medium and delivered polarity when those facts are conditional.

A route description must not invent a creative category. It must describe process sequence.

Examples:

| Exact route | Direct description |
|---|---|
| `input` | Color-manage an already rendered RGB image. |
| `input > film > scan` | Expose film and scan the selected film medium. |
| `input > film > print > scan` | Expose film, print it photographically, then scan the paper. |
| `input > convert-film > scan` | Convert the input to a negative capture, then scan the film. |
| `input > convert-film > print > scan` | Convert the input to a negative capture, print it photographically, then scan the paper. |
| `input > convert-film > scan-minus-base` | Convert the input and scan the film with its base removed. |

The selector must not silently change route terminology when a profile or stock changes. If a stock change resets an ineligible route or scanner selection, the header status must state what was normalized or cleared.

## Scanner product surface

The Scanner surface must follow the physical sequence from [`Film chain`](../film-chain/spec.md#scan-medium-and-scanner-controls):

1. scanner acquisition through a scan receiver;
2. scan output polarity and negative conversion;
3. automatic scan setup;
4. optional scanner finishing;
5. downstream digital output.

This sequence may be explained with a short static help line. It must not create additional parameter groups or rename the canonical scanner controls.

The surface must distinguish:

- receiver acquisition from negative conversion;
- record density from receiver capture;
- automatic scan setup from receiver calibration;
- scanner finishing from film development and print exposure;
- scanner white and black correction from film-scan black and white points;
- lens blur from unsharp mask.

Approximate or modeled choices must carry their qualification in the visible choice or immediate help. A measured-device claim requires measurement and calibration provenance.

## Viewer product surface

The viewer must make the retained image boundary obvious:

- `Input` shows retained input.
- `Output` shows retained output.
- `Paper back` shows the viewer-only virtual paper-back rendering.

The viewer must explain that Paper back, display ICC, display transform, canvas, interpolation, zoom, comparison, and reveal affect viewer presentation only. They must not change retained, saved, or exported pixels.

The footer must group controls by target:

- rotation;
- zoom and fit;
- before/after comparison;
- read-only zoom status.

The GUI must not introduce pointer gestures that conflict with numeric-field editing or control-panel scrolling without a separate interaction specification and acceptance coverage.

## Control-panel product surface

Collapsed sections must state their contents using canonical terms. Summaries must name physical controls, not internal field paths.

A section modified state and row modified state must use the section-reset factory baseline. The visible cue must not rely on color alone. The help must state that reset restores the section's factory values.

The filter must remain a transient control-panel operation. It must report the owning page and section for cross-page results. It must not change parameters or persisted state. A clear action, placeholder, or broader search vocabulary requires an explicit implementation change to the control-search contract.

The GUI must not add a second page navigation surface. Search navigation may switch to the owning page because it reveals ownership rather than flattening the workflow.

## Save, Export, and default-state actions

The action surface must distinguish:

- Preview: bounded retained render;
- Scan: full-resolution retained render;
- Save: write retained output without rerendering;
- Export: independent full-resolution render and write;
- Save current as default: replace the startup GUI state;
- Restore factory default: replace current controls with factory state and remove the saved startup default.

Restore factory default is destructive to current session configuration. Before applying it, the GUI must state what it replaces. A confirmation is required unless the action is made reversible by an explicit undo contract.

Save current as default must report that the startup state was replaced. It may proceed without confirmation when the operation is not destructive to the current session.

Export settings must remain independent of Save settings. The UI must not imply that changing display ICC, viewer interpolation, Paper back, or viewer zoom changes either output path.

## Preset browser

A preset entry must expose its film and print stocks using the same visual stock identifiers used by the header and profile selectors. Applied, modified, and missing states must remain distinguishable without color alone.

Existing preset management actions must be reachable by pointer and keyboard. Context menus may remain available, but they must not be the only route to update, revert, rename, duplicate, export, reveal, or delete an existing user preset.

Preset thumbnails, favorites, search, sorting, and collections are outside this product-design scope.

## Discoverability improvements

The following improvements are part of the product-design target:

1. add accurate help for route, scanner, display, CCTF, Save, Export, and viewer-only boundaries;
2. expose route/profile normalization in the single header status surface;
3. provide an explicit no-match state for control filtering;
4. make filter clearing discoverable without changing its transient semantics;
5. provide keyboard-reachable preset actions;
6. document drag-and-drop image import if the existing behavior remains supported;
7. consider canvas wheel zoom only after resolving its interaction with numeric fields and control-panel scrolling.

The last item is not approved for implementation by this specification. It requires a separate interaction decision.

## Visual design constraints

The current three-region layout, token layer, spacing rhythm, right control panel width, and page ownership are retained. A language or discoverability change must not trigger a general visual redesign.

The GUI must preserve:

- one label column and one value column for parameter rows;
- readable wrapped summaries for collapsed sections;
- row alignment at the supported 1460 × 980 application layout;
- visible action-bar controls without clipping;
- accessible labels independent of decorative status text;
- non-color state cues.

A layout change must be verified with the real offscreen layout suite and a real GUI review.

## Implementation slices

### Slice A: language authority

Add one GUI-owned presentation vocabulary map. It must draw domain definitions from `CONTEXT.md` and operation definitions from the owning specifications. It must separate visible labels, help text, serialized values, and diagnostics.

### Slice B: safe presentation improvements

Implement only changes that preserve physical meaning:

- direct physical route descriptions beside exact route values;
- accurate tooltips for scanner and display boundaries;
- explicit Save and Export scope;
- CCTF expansion at first use;
- Paper back viewer-only explanation;
- descriptive reset and modified help;
- clear filter no-match feedback.

### Slice C: destructive-action and preset discoverability

Add factory-reset confirmation and keyboard-reachable preset actions. Preserve the one-surface status rule and existing preset ownership.

### Slice D: interaction proposals

Treat wheel zoom, new viewer layer names, and broader search matching as separate proposals. Do not implement them as incidental copy edits.

## Non-goals

This specification does not authorize:

- a new component framework or egui upgrade;
- a new physical-process vocabulary;
- Digital response, Reproduction, saturation, color-density, split-toning, or retired output-lightness controls;
- route or parameter behavior changes;
- silent migration aliases for scanner or output terminology;
- changes to Save or Export pixels;
- preset thumbnails, favorites, sorting, collections, or preset search;
- prototype-only gestures without a decided interaction contract.

## Acceptance

1. A person can identify the selected materials, physical route, retained viewer layer, and last operation without reading serialized parameter paths.
2. Technical help preserves the exact physical operation, domain, stage, units, and calibration status.
3. Scanner labels match the film-chain contracts and do not conflate acquisition, conversion, setup, finishing, or digital output.
4. Save, Export, Preview, Scan, display transform, and viewer-only state remain distinct and testable.
5. Route and profile normalization reports its effects without changing the intended runtime behavior.
6. Factory reset, preset management, search feedback, and drag-and-drop affordances are keyboard-reachable or explicitly documented as not yet supported.
7. Existing page ownership, persisted-state compatibility, headless behavior, layout behavior, and Save/Export pixel behavior remain unchanged unless a separate specification authorizes the change.
