# GUI presentation design

## Design language boundary

This design is the implementation boundary for the SpektraFilm presentation language. It translates the semantic roles in the [GUI presentation specification](spec.md) into one native token set and one implementation per control kind.

Presentation must preserve the product vocabulary from [GUI product language](../gui-language/spec.md), the product direction from [GUI product design](../gui-product-design/spec.md), and the workflow ownership from [GUI workflow design](../gui-workflow/design.md). It must not create a second language, token set, page model, or state model.
- Presentation must not change parameter meaning, accepted values, editor bounds, validation, section ownership, or persisted state.
- Region and control ownership follow the [GUI workflow design](../gui-workflow/design.md). Presentation assigns each region its surface level and moves no control between regions.
- One token set owns spacing, corner radius, type roles, and color roles. A module must not declare its own spacing, radius, font size, or literal color.
- A color token names a role — a surface level, a text role, or a state role. A module references the role, never an appearance.
- The viewer region presents the image and carries no control chrome. The control panel uses the lowest-emphasis surfaces and text roles, so the image stays the strongest element in the window.
- One control kind has one implementation. A section composes the vocabulary; a raw toolkit widget may appear only inside that implementation.
- The editor range declaration is the only source of value decimals and slider bounds. Slider scale is a presentation decision derived from that declaration; it introduces no second range source.
- A logarithmic slider uses the declared step as its smallest positive value, and a zero minimum stays reachable as the track's leftmost stop.
- Hover, focus, disabled, and invalid have one treatment per control kind, declared once.
- The modified mark is derived from the value and the section reset target. It must not be persisted or serialized.
- The section reset target is the projected factory state, not the persisted section payload. Parameters that only the runtime model carries — a grain engine and its engine-specific group among them — keep a factory value and therefore a reset target, even though no persisted section holds them.
- One section-to-parameter table maps each section to the parameters it presents. The table is the only source of a section's reset scope and of the rows its modified mark covers. A section's parameter set is not derived from the numeric range table, which holds numeric parameters only.
