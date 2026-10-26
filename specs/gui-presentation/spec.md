# GUI presentation
## SpektraFilm design language

This specification is the authoritative entry point for the SpektraFilm GUI design language. It owns the product's visual grammar, token roles, component presentation, state cues, and accessibility presentation. The [GUI workflow specification](../gui-workflow/spec.md) owns workflow behavior, stage and surface ownership, and the control inventory.

The [GUI product design specification](../gui-product-design/spec.md) owns product direction, information hierarchy, and discoverability boundaries. The [GUI product language specification](../gui-language/spec.md) owns visible labels, help text, and status wording.

SpektraFilm defines its own palette, surface hierarchy, workflow layout, component names, and state meanings. Industry design systems MAY inform reusable methods such as semantic tokens, explicit interaction states, and perceivable feedback, but they are background research and are not normative references for this product.

The product visual grammar is:

```text
material stage  →  parameter workbench  →  retained image  →  delivery
stage heading  →      module/row       →      viewer       →  export
```

Every interactive presentation decision MUST identify the material or workflow stage, the value or state under inspection, the action owner, and what remains unchanged while work is pending or unavailable.

The interface is process-first, material-aware, layered, quiet, explicit, precise, reversible, and native. It presents film simulation as a material process, treats the viewer as an inspection surface, and keeps the control panel as the parameter workbench. It uses a restrained warm accent for selection and action and explains route capability and retained values in product language.

The presentation target is a quiet photo-first hierarchy over restrained surfaces.

| Concern | Target |
| --- | --- |
| Hierarchy | The retained image leads every layout decision under [visual hierarchy](#visual-hierarchy). |
| Surfaces | Panels use restrained dark warm surfaces rather than competing frames. |
| Inactive values | A retained inactive value stays readable beside its reason under [states](#states). |
| State cues | Selection, modified, and focus states never rely on color alone under [states](#states). |
| Viewer footer | Footer actions group by target under [viewer footer](#viewer-footer). |
| Preset browser | The left browser owns whole-look presets under [Presets](../presets/spec.md#contract). |
| Components | One shared implementation serves each control kind under [presentation design](design.md#surface-and-component-boundaries). |

## Semantic role vocabulary

The native token vocabulary belongs to SpektraFilm:

| Family | Roles | Intent |
| --- | --- | --- |
| Surfaces | `BG`, `SURFACE`, `SURFACE2`, `SURFACE3` | Separate the work surface, containers, sections, and nested interaction surfaces. |
| Structure | `BORDER`, `BORDER_ACTIVE` | Show containment and focus without using decoration as status. |
| Content | `FG`, `FG_SECONDARY`, `FG_MUTED` | Set readable content hierarchy for values, labels, descriptions, units, and hints. |
| Action | `ACCENT`, `ACCENT_DIM`, `CONTRAST_ON_ACCENT` | Identify selection, focus, action, and readable content on an accent fill. |
| Feedback | `ERROR`, `WARNING` | Distinguish invalid input from inactive capability, experimental behavior, or retained-value notices. |

The exact palette values and toolkit mapping remain in [`theme.rs`](../../crates/spektrafilm-gui/src/theme.rs). It MUST provide the shared [native layout baseline](#native-layout-baseline). A presentation module uses a semantic role or shared component token, not a new raw appearance value.

## Language authority

Visible labels and help text follow [GUI product language](../gui-language/spec.md). The product-surface direction and discoverability boundaries follow [GUI product design](../gui-product-design/spec.md). Physical and scanner terms remain owned by [`CONTEXT.md`](../../CONTEXT.md) and the [film-chain specification](../film-chain/spec.md).

## Authority

The presentation layer in `crates/spektrafilm-gui` owns the token set and surface hierarchy. It presents the control vocabulary owned by [GUI product language](../gui-language/spec.md); it does not redefine visible labels or help text. [GUI workflow](../gui-workflow/spec.md#stage-and-surface-ownership) owns stage and module ownership.

[Complete control access](../gui-workflow/spec.md#complete-control-access) owns the editable inventory and reset behavior. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns editor bounds and value rejection.

## Visual hierarchy

The central viewer MUST remain the dominant surface. The collapsible preset browser occupies the left side. The continuous inspector occupies the right side.

Stage headings MUST be stronger than module headings, and module headings MUST be stronger than row labels and secondary help.

The header toolbar MUST present Open, Undo, Redo, Settings, and Export without competing with the retained image. Preview remains automatic; Scan and Save MUST remain distinct and reachable under [stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership). Settings and Export MUST read as separate dialogs, not additional inspector stages.

The toolbar MUST use one horizontal action row with read-only source and material context. Compact icon actions MUST retain their canonical accessible names. Routine render completion MUST NOT duplicate the viewer status identity.

Live rendering feedback follows [viewer presentation](../gui-workflow/spec.md#viewer-presentation).

The inspector MUST use one vertically scrolling control list. Long lists MUST NOT require horizontal scrolling to reach a value, reset action, or help. Labels and help MAY wrap without hiding the value or its editor.

Collapse controls MUST show their expanded state without color alone. Stage and module expansion MUST remain independent under [complete control access](../gui-workflow/spec.md#complete-control-access).

Labels and current values MUST have greater visual emphasis than reset, management, and technical-help actions. Compact secondary actions MUST retain visible keyboard focus and an accessible name that identifies their target. Essential actions MUST NOT be discoverable only by hover.

Numeric entries and sliders MUST remain available under [Parameter rows](#parameter-rows); reducing visual weight MUST NOT remove an editing method. Canonical paths MUST remain secondary detail rather than replace product labels. Inactive values MUST stay legible.

These changes MUST use the existing semantic tokens rather than introduce a second theme.

## Native layout baseline

The shared native tokens MUST use this baseline in logical points. Settings, Export, and the inspector MUST share row alignment and control sizing; a surface MUST NOT squeeze a value or selector to make room for its label or action.

| Role | Baseline |
| --- | --- |
| Body, label, and button text | 13 |
| Small text and units | 11 |
| Title text | 13.5 |
| Stage heading text | 15 |
| Control height | 28 |
| Inspector width | 344 |
| Selector maximum width | 160 |
| Action height and reserved action area | 64 and 80 |
| Header height | 64 |
| Viewer status and footer height | 34 and 50 |
| Expanded module inset | 12 |
| Dialog header height and region inset | 56 and 16 |

Long labels MUST wrap without obscuring their values. Fixed and inapplicable Export rows MUST use the same readable row structure as editable options under [dialog interaction](../gui-workflow/spec.md#dialog-interaction). Responsive preset geometry follows the [preset browser boundary](../presets/design.md#boundaries).

Expanded modules MUST use a quiet bounded surface. Collapsed modules MUST remain visually lighter than their expanded controls. Workflow and Camera MUST precede source-detail modules in Input and capture.

## Parameter rows

- The inspector presents an editable parameter as a parameter row in its owning module.
- A parameter row presents a label, a value, and one editing owner for that value.
- A row presents its label before its value. It MAY wrap at narrow widths while preserving that reading order.
- A label MUST NOT contain a unit, a state marker, or any other value-dependent text.
- A label that names an enable flag names the effect the flag turns on, not the flag's state.
- A label that two different parameters share MUST distinguish them. A qualifier that names the owning concept satisfies this rule.
- A module title alone does not, because rows share one label column across the inspector.
- A value presents the parameter's declared decimals with tabular figures. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns those decimals.
- Rows in the inspector align to shared label and value columns when width permits. Wrapped rows preserve the same label, value, and action order.
- A numeric parameter row presents a slider in addition to its numeric entry. Both edit the same value through the same editing owner and show the same accepted value.
- A slider is logarithmic when its declared range spans more than three decades, or when its declared minimum is zero and its declared maximum reaches one thousand or more. Every other slider is linear.
- A logarithmic slider anchors its smallest positive value at the parameter's declared step, so its positive floor matches the editor's own granularity.
- A logarithmic slider with a zero minimum keeps zero reachable at the track's leftmost stop.
- Slider bounds, step, and entry decimals come from the same declaration under [numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges). Presentation MUST NOT add a different slider-only bound or rounding rule.
- Units MUST remain readable beside values without becoming part of the label or editable numeric text.
- Numeric rows MUST place the unit before the right-aligned value.
- A unitless row MUST retain the same value-column alignment.
- The slider MUST span the module's usable row width.
- A tuple parameter row presents one editor per component, each with its component identifier.
- A choice parameter row presents a selector as its editor. The row label is the selector's only label.
- A boolean parameter row presents a switch as its editor. A checkbox belongs to list selection only.

## Module headers and resets

A module header MUST present its collapse control, title, modified state, and the module's factory reset action. Rows, sliders, and dialog options MUST NOT present their own reset actions; Undo reverses an individual adjustment. Reset actions MUST be keyboard reachable and MUST identify their target and factory baseline in accessible help.

Reset scope follows [complete control access](../gui-workflow/spec.md#complete-control-access), and reset transactions follow [edit history](../gui-workflow/spec.md#edit-history). A module reset MUST identify the module's owned controls, not imply a reset of the whole serialized preset section.

A row whose value differs from its projected factory value MUST present a non-color modified marker. A module header MUST present the aggregate modified state of its owned rows. Markers MUST NOT change the row's accessible label.

The [presentation design](design.md#factory-projection-and-modified-state) owns the comparison and projection boundary.

## Markers

- A control that carries an experimental marker presents that marker beside its label.
- An experimental marker MUST NOT be part of the label text.

## Stage navigation and control locators

The inspector MUST present Input and capture, Film, Print, Scan, and Output as stage headings in one continuous list. The stage jump strip MUST look like navigation to those headings, not a mutually exclusive page selector. The interface MUST NOT present MAIN, FILM, PRINT, ADVANCED, or CONFIG tabs.

The search field MUST remain visible above the inspector list. Search behavior follows [control search](../gui-workflow/spec.md#control-search). A locator MUST show the control label, owning module, and destination stage or dialog.

It MUST expose the canonical path as secondary technical detail without replacing the product label.

A locator MUST look and act like a navigation result, not a second parameter editor. Results for Settings or Export MUST identify the destination dialog. The destination MUST visibly identify the real row when reached.

Locator results and destination cues MUST remain readable and keyboard accessible without relying on color.

## Viewer footer

- The footer groups its actions by target. One action belongs to one group.
- Its groups are layer selection, view zoom, and reveal or crossfade presentation.
- The footer presents the current zoom as read-only text, apart from the actions.
- The footer MUST keep Original and Modified together and Paper back separate. Compact Paper back and transition actions MAY use named icons to preserve nonoverlapping zoom and reset actions in a reopened narrow workspace.

## Preset browser

- A preset entry presents its film stock and its print stock.
- A stock presents the same visual identifier everywhere it appears.
- The browser MUST have a labeled, keyboard-operable collapse and reopen control. Collapsing it MUST NOT hide the only indication of how to reopen it.
- Selection, hover, and keyboard focus MUST remain visually distinct. A selected preset MUST have a non-color cue.
- Preset names and material identifiers MUST remain readable in long lists. Truncated names MUST remain available through accessible text and help.
- The browser MUST keep the current-look summary independent of search and filters. Without an applied preset, it MUST identify current adjustments without inventing an applied look.
- Reference credit MUST remain in a fixed footer outside the scrolling cards.

## States

- Every control presents a hover, focus, disabled, and invalid state.
- A disabled or inactive control keeps its current value readable and presents the reason for its unavailability. Inactive values MUST NOT appear as empty or cleared fields.
- A state MUST NOT be presented by color alone.
- Every control is operable by pointer and by keyboard.
- Text, values, units, and inactive reasons MUST remain legible against their semantic surfaces. Muted roles MUST NOT make retained values unreadable.
- Focus MUST be visible and MUST NOT be clipped by scrolling containers. Keyboard navigation MUST reach collapse controls, resets, search results, and each editable row in a predictable visual order.
- A row's accessible name MUST identify its product label. Its value, unit, state, and inactive reason MUST remain available without hover.
