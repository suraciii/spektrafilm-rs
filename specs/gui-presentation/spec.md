# GUI presentation
## Language authority

Visible labels and help text follow [GUI product language](../gui-language/spec.md). The product-surface direction and discoverability boundaries follow [GUI product design](../gui-product-design/spec.md). Physical and scanner terms remain owned by [`CONTEXT.md`](../../CONTEXT.md) and the [film-chain specification](../film-chain/spec.md).

## Authority

The presentation layer in `crates/spektrafilm-gui` owns the token set, the control vocabulary, and the surface hierarchy. [GUI workflow](../gui-workflow/spec.md) owns page and section ownership, the control inventory, and editing behavior. [Numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges) owns editor bounds and value rejection.

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

- A section header presents a disclosure control and the section title.
- A section presents its reset action on its header, and nowhere else. [GUI workflow](../gui-workflow/spec.md#section-reset) owns which sections carry a reset and what it restores.
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
