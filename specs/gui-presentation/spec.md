# GUI presentation
## SpektraFilm design language

This specification is the authoritative entry point for the SpektraFilm GUI design language. It owns the product's visual grammar, token roles, component presentation, state cues, and accessibility presentation. The [GUI workflow specification](../gui-workflow/spec.md) owns workflow behavior, stage and surface ownership, and the control inventory. The [GUI product design specification](../gui-product-design/spec.md) owns product direction, information hierarchy, and discoverability boundaries. The [GUI product language specification](../gui-language/spec.md) owns visible labels, help text, and status wording.

SpektraFilm defines its own palette, surface hierarchy, workflow layout, component names, and state meanings. Industry design systems may inform reusable methods such as semantic tokens, explicit interaction states, and perceivable feedback, but they are background research and are not normative references for this product.

The product visual grammar is:

```text
material stage  →  parameter workbench  →  retained image  →  delivery
stage heading  →      module/row       →      viewer       →  export
```

Every interactive presentation decision must identify the material or workflow stage, the value or state under inspection, the action owner, and what remains unchanged while work is pending or unavailable.

The interface is process-first, material-aware, layered, quiet, explicit, precise, reversible, and native. It presents film simulation as a material process, treats the viewer as an inspection surface, keeps the control panel as the parameter workbench, uses a restrained warm accent for selection and action, and explains route capability and retained values in product language.

## Semantic role vocabulary

The native token vocabulary belongs to SpektraFilm:

| Family | Roles | Intent |
| --- | --- | --- |
| Surfaces | `BG`, `SURFACE`, `SURFACE2`, `SURFACE3` | Separate the work surface, containers, sections, and nested interaction surfaces. |
| Structure | `BORDER`, `BORDER_ACTIVE` | Show containment and focus without using decoration as status. |
| Content | `FG`, `FG_SECONDARY`, `FG_MUTED` | Set readable content hierarchy for values, labels, descriptions, units, and hints. |
| Action | `ACCENT`, `ACCENT_DIM`, `CONTRAST_ON_ACCENT` | Identify selection, focus, action, and readable content on an accent fill. |
| Feedback | `ERROR`, `WARNING` | Distinguish invalid input from inactive capability, experimental behavior, or retained-value notices. |

The exact values, geometry, type sizes, and toolkit mapping remain in [`theme.rs`](../../crates/spektrafilm-gui/src/theme.rs). A presentation module uses a semantic role or shared component token, not a new raw appearance value.

## Language authority

Visible labels and help text follow [GUI product language](../gui-language/spec.md). The product-surface direction and discoverability boundaries follow [GUI product design](../gui-product-design/spec.md). Physical and scanner terms remain owned by [`CONTEXT.md`](../../CONTEXT.md) and the [film-chain specification](../film-chain/spec.md).

## Authority

The presentation layer in `crates/spektrafilm-gui` owns the token set and surface hierarchy. It presents the control vocabulary owned by [GUI product language](../gui-language/spec.md); it does not redefine visible labels or help text. [GUI workflow](../gui-workflow/spec.md#stage-and-surface-ownership) owns stage and module ownership. [Complete control access](../gui-workflow/spec.md#complete-control-access) owns the editable inventory and reset behavior. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns editor bounds and value rejection.

## Visual hierarchy

The central viewer must remain the dominant surface. The collapsible preset browser occupies the left side. The continuous inspector occupies the right side. Stage headings must be stronger than module headings, and module headings must be stronger than row labels and secondary help.

The header toolbar must present Open, Undo, Redo, Settings, and Export without competing with the retained image. Preview remains automatic; Scan and Save must remain distinct and reachable under [stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership). Settings and Export must read as separate dialogs, not additional inspector stages. Live rendering feedback follows [viewer presentation](../gui-workflow/spec.md#viewer-presentation).

The inspector must use one vertically scrolling control list. Long lists must not require horizontal scrolling to reach a value, reset action, or help. Labels and help may wrap without hiding the value or its editor. Collapse controls must show their expanded state without color alone. Stage and module expansion must remain independent under [complete control access](../gui-workflow/spec.md#complete-control-access).

Labels and current values must have greater visual emphasis than reset, management, and technical-help actions. Compact secondary actions must retain visible keyboard focus and an accessible name that identifies their target. Essential actions must not be discoverable only by hover.

Numeric entries and sliders must remain available under [Parameter rows](#parameter-rows); reducing visual weight must not remove an editing method. Canonical paths must remain secondary detail rather than replace product labels. Inactive values must stay legible. These changes must use the existing semantic tokens rather than introduce a second theme.

## Parameter rows

- The inspector presents an editable parameter as a parameter row in its owning module.
- A parameter row presents a label, a value, and one editing owner for that value.
- A row presents its label before its value. It may wrap at narrow widths while preserving that reading order.
- A label must not contain a unit, a state marker, or any other value-dependent text.
- A label that names an enable flag names the effect the flag turns on, not the flag's state.
- A label that two different parameters share must distinguish them. A qualifier that names the owning concept satisfies this rule. A module title alone does not, because rows share one label column across the inspector.
- A value presents the parameter's declared decimals with tabular figures. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns those decimals.
- Rows in the inspector align to shared label and value columns when width permits. Wrapped rows preserve the same label, value, and action order.
- A numeric parameter row presents a slider in addition to its numeric entry. Both edit the same value through the same editing owner and show the same accepted value.
- A slider is logarithmic when its declared range spans more than three decades, or when its declared minimum is zero and its declared maximum reaches one thousand or more. Every other slider is linear.
- A logarithmic slider anchors its smallest positive value at the parameter's declared step, so its positive floor matches the editor's own granularity.
- A logarithmic slider with a zero minimum keeps zero reachable at the track's leftmost stop.
- Slider bounds, step, and entry decimals come from the same declaration under [numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges). Presentation must not add a different slider-only bound or rounding rule.
- Units must remain readable beside values without becoming part of the label or editable numeric text.
- A tuple parameter row presents one editor per component, each with its component identifier.
- A choice parameter row presents a selector as its editor. The row label is the selector's only label.
- A boolean parameter row presents a switch as its editor. A checkbox belongs to list selection only.

## Module headers and resets

A module header must present its collapse control, title, modified state, and factory reset action. A parameter row must also expose a factory reset for that row. Reset actions must be keyboard reachable and must identify their target and factory baseline in accessible help.

Reset scope follows [complete control access](../gui-workflow/spec.md#complete-control-access), and reset transactions follow [edit history](../gui-workflow/spec.md#edit-history). A module reset must identify the module's owned controls, not imply a reset of the whole serialized preset section. A wider section-preset menu must identify its full scope at its primary module anchor.

A row whose value differs from its factory reset target must present a non-color modified marker. A module header must present the aggregate modified state of its owned rows. Markers must not change the row's accessible label. The [presentation design](design.md#factory-projection-and-modified-state) owns the comparison and projection boundary.

## Markers

- A control that carries an experimental marker presents that marker beside its label.
- An experimental marker must not be part of the label text.

## Stage navigation and control locators

The inspector must present Input and capture, Film, Print, Scan, and Output as stage headings in one continuous list. The stage jump strip must look like navigation to those headings, not a mutually exclusive page selector. The interface must not present MAIN, FILM, PRINT, ADVANCED, or CONFIG tabs.

The search field must remain visible above the inspector list. Search behavior follows [control search](../gui-workflow/spec.md#control-search). A locator must show the control label, owning module, and destination stage or dialog. It must expose the canonical path as secondary technical detail without replacing the product label.

A locator must look and act like a navigation result, not a second parameter editor. Results for Settings or Export must identify the destination dialog. The destination must visibly identify the real row when reached. Locator results and destination cues must remain readable and keyboard accessible without relying on color.

## Viewer footer

- The footer groups its actions by target. One action belongs to one group.
- The footer presents the current zoom as read-only text, apart from the actions.

## Preset browser

- A preset entry presents its film stock and its print stock.
- A stock presents the same visual identifier everywhere it appears.
- The browser must have a labeled, keyboard-operable collapse and reopen control. Collapsing it must not hide the only indication of how to reopen it.
- Selection, hover, and keyboard focus must remain visually distinct. A selected preset must have a non-color cue.
- Preset names and material identifiers must remain readable in long lists. Truncated names must remain available through accessible text and help.

## States

- Every control presents a hover, focus, disabled, and invalid state.
- A disabled or inactive control keeps its current value readable and presents the reason for its unavailability. Inactive values must not appear as empty or cleared fields.
- A state must not be presented by color alone.
- Every control is operable by pointer and by keyboard.
- Text, values, units, and inactive reasons must remain legible against their semantic surfaces. Muted roles must not make retained values unreadable.
- Focus must be visible and must not be clipped by scrolling containers. Keyboard navigation must reach collapse controls, resets, search results, and each editable row in a predictable visual order.
- A row's accessible name must identify its product label. Its value, unit, state, inactive reason, and reset target must remain available without hover.
