# GUI presentation design

## Design language boundary

This design is the implementation boundary for the SpektraFilm presentation language. It translates the semantic roles in the [GUI presentation specification](spec.md) into one native token set and one implementation per control kind.

Presentation must preserve the vocabulary from [GUI product language](../gui-language/spec.md), the hierarchy from [GUI product design](../gui-product-design/spec.md), and [stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership). It must not create a second language, token set, navigation model, or state model.

## Surface and component boundaries

Presentation must not change parameter meaning, accepted values, editor bounds, validation, canonical ownership, or persisted state. It assigns surface levels to the workflow's regions without moving controls between owners.

One token set must own spacing, corner radius, type roles, and color roles. A module must not declare its own spacing, radius, font size, or literal color. Color tokens must name semantic roles rather than appearances. The existing `BG`, `SURFACE`, `SURFACE2`, `SURFACE3`, content, action, structure, and feedback roles remain the vocabulary defined by the [presentation specification](spec.md#semantic-role-vocabulary).

The viewer image must have the strongest visual emphasis. Viewer controls must remain outside the image surface. The preset browser and inspector must use restrained surfaces and text roles. Stage headings must divide the continuous inspector without creating page chrome. Settings and Export must use the same component vocabulary as the inspector without becoming alternate editors for inspector-owned values.

One control kind must have one implementation. A module composes shared controls; a raw toolkit widget may appear only inside the shared implementation. Hover, focus, disabled, and invalid must have one treatment per control kind. Accessible names and state descriptions must come from the same product vocabulary as visible text.

## Continuous inspector

The inspector must use a single vertical scroll owner for its control list. Stage and module headings must remain in the same reading order as their rows. Long lists must retain readable labels, numeric entries, reset actions, and focus indicators without horizontal scrolling. Row wrapping must preserve label-before-value reading order. Help and inactive reasons must not cover the editor or its reset action.

The stage strip must navigate to headings in that list rather than replace the list with mutually exclusive pages. Expansion, navigation, and search behavior follow [complete control access](../gui-workflow/spec.md#complete-control-access) and [control search](../gui-workflow/spec.md#control-search). Presentation must not add a second tab or page state.

Cross-surface search results must use locator rows distinct from editable parameter rows. Each locator must present the product label first, then its module and stage or dialog destination. The canonical path must remain secondary technical detail. Reaching a result must expose a visible, non-color destination cue at the real editing owner. A locator must not contain a slider, switch, numeric entry, or other duplicate editor. In-place inspector matches retain their real rows under the workflow search contract.

The preset browser must scroll independently from the inspector. Its collapse and reopen control must remain reachable when the browser is collapsed. Preset selection and keyboard focus must use distinct cues without changing the stock identifier used elsewhere.

## Numeric editing presentation

The editor-range declaration must be the only source of value decimals, bounds, and step. Slider and text entry must share the canonical value, validator, accepted range, and edit transaction. Slider scale follows the exact threshold rules in [parameter rows](spec.md#parameter-rows); presentation must not introduce a second range source.

A logarithmic slider must use the declared step as its smallest positive value. A declared zero minimum must remain reachable as the track's leftmost stop. Invalid or incomplete text must remain visibly distinct from the accepted value under [numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges). The slider must not represent an unaccepted text draft as a committed parameter change.

## Factory projection and modified state

The row reset target must be the canonical control's projected factory value. The module reset target must be the factory projection of only the visible controls that the module owns under [complete control access](../gui-workflow/spec.md#complete-control-access). A route-inactive owned row remains part of that scope. Collapse and search visibility must not redefine ownership or reset scope.

The projection must not use the selected preset, loaded state, startup default, or whole serialized preset section as its baseline. Runtime-only editable controls, including the grain selector and engine-specific controls, must retain factory targets even when no persisted section carries them. The canonical ownership mapping must be the source of module scope; the numeric range table must not define scope because it excludes nonnumeric controls.

The modified marker must be derived rather than persisted or serialized. Numeric comparisons must use exact bits against the factory projection. Tuple components, optional values, extra controls, choices, and booleans must participate in the same row and module scope. A module's marker must aggregate only its owned rows. A marker must use a non-color cue and must not change the row's accessible label.

Preset-section actions must remain separate from row and module reset actions. A section menu at its primary module anchor must state its full payload scope when that scope crosses visible module boundaries. Presentation must not change the preset schema or imply that visual module boundaries change stored section payloads.

Reset and edit transactions must follow [edit history](../gui-workflow/spec.md#edit-history). Presentation must not infer explicit provenance from the modified marker or implement a second undo model.
