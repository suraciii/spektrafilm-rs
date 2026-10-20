# GUI presentation
## SpektraFilm design language

This specification is the authoritative entry point for the SpektraFilm GUI design language. It owns the product's visual grammar, token roles, component presentation, state cues, and accessibility presentation. The [GUI workflow specification](../gui-workflow/spec.md) owns workflow behavior, page ownership, and the control inventory. The [GUI product design specification](../gui-product-design/spec.md) owns product direction, information hierarchy, and discoverability boundaries. The [GUI product language specification](../gui-language/spec.md) owns visible labels, help text, and status wording.

SpektraFilm defines its own palette, surface hierarchy, workflow layout, component names, and state meanings. Industry design systems may inform reusable methods such as semantic tokens, explicit interaction states, and perceivable feedback, but they are background research and are not normative references for this product.

The product visual grammar is:

```text
material stage  →  parameter workbench  →  retained image  →  delivery
     page       →      section/row       →      viewer       →  export
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

The presentation layer in `crates/spektrafilm-gui` owns the token set and surface hierarchy. It presents the control vocabulary owned by [GUI product language](../gui-language/spec.md); it does not redefine visible labels or help text. [GUI workflow](../gui-workflow/spec.md) owns page and section ownership, the control inventory, and editing behavior. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns editor bounds and value rejection.

## Parameter rows

- The control panel presents an editable parameter as a parameter row.
- A parameter row presents a label, a value, and one editor for that value.
- A row presents its label before its value, on one line.
- A label must not contain a unit, a state marker, or any other value-dependent text.
- A label that names an enable flag names the effect the flag turns on, not the flag's state.
- A label that two different parameters share must distinguish them. A qualifier that names the owning concept satisfies this rule. A section title alone does not, because rows share one label column across a whole page.
- A value presents the parameter's declared decimals with tabular figures. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns those decimals.
- Every row on a page aligns to one label column and one value column.
- A numeric parameter row presents a slider in addition to its value. The value remains the row's editor.
- A slider is logarithmic when its declared range spans more than three decades, or when its declared minimum is zero and its declared maximum reaches one thousand or more. Every other slider is linear.
- A logarithmic slider anchors its smallest positive value at the parameter's declared step, so its positive floor matches the editor's own granularity.
- A tuple parameter row presents one editor per component, each with its component identifier.
- A choice parameter row presents a selector as its editor. The row label is the selector's only label.
- A boolean parameter row presents a switch as its editor. A checkbox belongs to list selection only.

## Section headers

- A section presents its reset action on its header, and nowhere else. The [GUI presentation design](design.md) owns the presentation reset scope and factory projection; the [GUI workflow specification](../gui-workflow/spec.md) owns which sections exist and how editing behaves.
- A parameter row whose value differs from the value that its section reset restores is marked as modified. The marker must provide a non-color cue and must not change the row's accessible label. Numeric, tuple, optional, extra, and boolean rows use the same declared section-reset scope.
- A section header presents the modified state of its rows. Row-level markers use exact-bit comparison against the factory projection, including leaf-specific Camera and Enlarger reset scopes.

## Markers

- A control that carries an experimental marker presents that marker beside its label.
- An experimental marker must not be part of the label text.

## Page navigation

- The control panel presents the page set as one segmented control.
- The panel must not present a second navigation surface for the same pages.

## Viewer footer

- The footer groups its actions by target. One action belongs to one group.
- The footer presents the current zoom as read-only text, apart from the actions.

## Preset browser

- A preset entry presents its film stock and its print stock.
- A stock presents the same visual identifier everywhere it appears.

## States

- Every control presents a hover, focus, disabled, and invalid state.
- A disabled control keeps its value readable.
- A state must not be presented by color alone.
- Every control is operable by pointer and by keyboard.
