# GUI presentation design

- Presentation must not change parameter meaning, accepted values, editor bounds, validation, section ownership, or persisted state.
- Region and control ownership follow the [GUI workflow design](../gui-workflow/design.md). Presentation assigns each region its surface level and moves no control between regions.
- One token set owns spacing, corner radius, type roles, and color roles. A module must not declare its own spacing, radius, font size, or literal color.
- A color token names a role — a surface level, a text role, or a state role. A module references the role, never an appearance.
- The viewer region presents the image and carries no control chrome. The control panel uses the lowest-emphasis surfaces and text roles, so the image stays the strongest element in the window.
- One control kind has one implementation. A section composes the vocabulary; a raw toolkit widget may appear only inside that implementation.
- The editor range declaration is the only source of value decimals and slider bounds.
- Hover, focus, disabled, and invalid have one treatment per control kind, declared once.
- The modified mark is derived from the value and the section reset target. It must not be persisted or serialized.
