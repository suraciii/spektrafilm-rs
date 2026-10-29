# GUI presentation design

## Design language boundary

This design is the implementation boundary for the SpektraFilm presentation language. It translates the semantic roles in the [GUI presentation specification](spec.md) into one native token set and one implementation per control kind.

Presentation MUST preserve the vocabulary from [GUI product language](../gui-language/spec.md), the hierarchy from [GUI product design](../gui-product-design/spec.md), and [stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership). It MUST NOT create a second language, token set, navigation model, or state model. The quiet photo-first target in the [presentation specification](spec.md#spektrafilm-design-language) constrains this translation.

## Surface and component boundaries

Presentation MUST NOT change parameter meaning, accepted values, editor bounds, validation, canonical ownership, or persisted state. It assigns surface levels to the workflow's regions without moving controls between owners.

One token set MUST own spacing, corner radius, type roles, and color roles. A module MUST NOT declare its own spacing, radius, font size, or literal color. Color tokens MUST name semantic roles rather than appearances.

The existing `BG`, `SURFACE`, `SURFACE2`, `SURFACE3`, content, action, structure, and feedback roles remain the vocabulary defined by the [presentation specification](spec.md#semantic-role-vocabulary).

The viewer image MUST have the strongest visual emphasis. Viewer controls MUST remain outside the image surface. The preset browser and inspector MUST use restrained surfaces and text roles.

Stage headings MUST divide the continuous inspector without creating page chrome. Settings and Export MUST use the same component vocabulary as the inspector without becoming alternate editors for inspector-owned values.

One control kind MUST have one implementation. A module composes shared controls; a raw toolkit widget MAY appear only inside the shared implementation. Hover, focus, disabled, and invalid MUST have one treatment per control kind.

Accessible names and state descriptions MUST come from the same product vocabulary as visible text.

## Continuous inspector

The inspector MUST use a single vertical scroll owner for its control list. Stage and module headings MUST remain in the same reading order as their rows. Long lists MUST retain readable labels, numeric entries, reset actions, and focus indicators without horizontal scrolling.

Row wrapping MUST preserve label-before-value reading order. Help and inactive reasons MUST NOT cover the editor or the module reset action.

The stage strip MUST navigate to headings in that list rather than replace the list with mutually exclusive pages. Expansion, navigation, and search behavior follow [complete control access](../gui-workflow/spec.md#complete-control-access) and [control search](../gui-workflow/spec.md#control-search). Presentation MUST NOT add a second tab or page state.

Cross-surface search results MUST use locator rows distinct from editable parameter rows. Each locator MUST present the product label first, then its module and stage or dialog destination. The canonical path MUST remain secondary technical detail.

Reaching a result MUST expose a visible, non-color destination cue at the real editing owner. A locator MUST NOT contain a slider, switch, numeric entry, or other duplicate editor. In-place inspector matches retain their real rows under the workflow search contract.

The preset browser MUST scroll independently from the inspector. Its collapse and reopen control MUST remain reachable when the browser is collapsed. Preset selection and keyboard focus MUST use distinct cues without changing the stock identifier used elsewhere.

## Numeric editing presentation

The editor-range declaration MUST be the only source of value decimals, bounds, and step. Slider and text entry MUST share the canonical value, validator, accepted range, and edit transaction. Slider scale follows the exact threshold rules in [parameter rows](spec.md#parameter-rows); presentation MUST NOT introduce a second range source.

A logarithmic slider MUST use the declared step as its smallest positive value. A declared zero minimum MUST remain reachable as the track's leftmost stop. Invalid or incomplete text MUST remain visibly distinct from the accepted value under [numeric editor ranges](../gui-workflow/spec.md#numeric-editor-ranges).

The slider MUST NOT represent an unaccepted text draft as a committed parameter change.

## Factory projection and modified state

The module reset target MUST use the [reset baseline](../gui-workflow/spec.md#section-reset) for only the editable controls that the module owns. A route-inactive owned row remains part of that scope.

Collapse and search visibility MUST NOT redefine ownership or reset scope.

The projection MUST NOT use the selected preset, loaded state, startup default, or whole serialized preset section as its baseline. Runtime-only editable controls, including the grain selector and engine-specific controls, remain in scope even when no persisted section carries them.

The canonical ownership mapping MUST be the source of module scope; the numeric range table MUST NOT define scope because it excludes nonnumeric controls.

The modified marker MUST be derived rather than persisted or serialized. Numeric comparisons MUST use exact bits against the reset baseline. Tuple components, optional values, extra controls, choices, and booleans MUST participate in the same row and module scope.

A module's marker MUST aggregate only its owned rows. A marker MUST use a non-color cue and MUST NOT change the row's accessible label.

Preset-library actions MUST remain separate from module reset actions. Presentation MUST NOT change the preset schema or imply that visual module boundaries change stored section payloads.

Reset and edit transactions MUST follow [edit history](../gui-workflow/spec.md#edit-history). Presentation MUST NOT infer explicit provenance from the modified marker or implement a second undo model.
