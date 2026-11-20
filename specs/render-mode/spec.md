# Render modes

## Authority

| Capability | Owns |
| --- | --- |
| This capability | Film, LUT, and Off modes; mode selection and switching; stage activation; the canonical `workflow.mode` value; inactive-value retention. |
| [Simulation methods](../simulation-method/spec.md) | Spectral and RGB method language and method mapping. |
| [Halation models](../halation/spec.md) | Film Halation and RGB Halation model distinction. |
| [LUT transform](../lut-transform/spec.md) | LUT declaration, validation, evaluation order, and the independent Diffusion, RGB Light Leak, RGB Halation, and Grain effects. |
| [LUT workflow](../lut-workflow/spec.md) | LUT file export and import, LUT presets, and the LUT-stage actions. |
| [Presets](../presets/spec.md) | Preset kinds, library behavior, and mode selection through preset application. |
| [GUI workflow](../gui-workflow/spec.md) | Simulation-method selector placement, stage ownership, and inspector behavior. |
| [CLI](../cli/spec.md) | Command syntax and canonical parameter discovery. |

## Contract

A **render mode** is Film, LUT, or Off. Exactly one mode is active.

Film executes the spectral film chain. LUT executes the external LUT transform and its effects. Off executes neither film nor LUT; it runs the Effects stage and output color management.

The active mode determines the simulation method under [Simulation methods](../simulation-method/spec.md).

The global selector presents the simulation method and RGB workflow.

- [Simulation methods](../simulation-method/spec.md) owns the mapping.
- [GUI workflow](../gui-workflow/spec.md#stage-and-surface-ownership) owns placement.
- [GUI language](../gui-language/spec.md#simulation-method-and-halation-labels) owns labels.

The canonical mode values remain `film`, `lut`, and `off`; the selector MUST NOT add a fourth mode.

Visible surfaces MUST present the active modules, not a mode named Off.
The canonical `off` value is an internal dispatch value.

The selector is a global control above the stage list, not a stage and not part of the stage jump strip.

The header MAY identify the active simulation method and active material, LUT, or Effects context as read-only information. It MUST NOT provide a second method editor.

## Activation

| Operation or stage | Film | LUT | Off |
| --- | --- | --- | --- |
| Input color interpretation, crop, rotation, upscale | active | active | active |
| Spectral camera, conversion, gamut, and reconstruction controls | active | inactive | inactive |
| Film route selector | active | inactive | inactive |
| Film, Print, and Scan | active | inactive | inactive |
| Scan Tone & Color | active | inactive | inactive |
| LUT declaration | inactive | active | inactive |
| LUT Grade | inactive | active | active (independent) |
| Effects: Diffusion, RGB Light Leak, RGB Halation, Grain | inactive | active | active |
| Output color space, encoding, and gamut processing | active | active | active |
| Output mix and CMYK ink style | active | inactive | inactive |

Inactive means stored values are retained but MUST NOT be read, prepared, or executed. An inactive stage keeps its heading, stored values, and applicability reason.

Effects are independent. Disabling one effect MUST be an exact bypass.

Off with every effect disabled and LUT Grade inactive passes the input to output color management without a film or LUT operation. Off is not monochrome.

Film material effects remain in Film and Print. Effects MUST NOT be duplicated into Off or LUT as film-density controls.

## Selection and switching

The GUI presents these transitions as Spectral, RGB LUT, and RGB Effects. The table names the canonical mode.

| Action | Result |
| --- | --- |
| Select Film | The active mode becomes Film. Stored LUT, Grade, and Effects values are preserved. |
| Select Off | The active mode becomes Off. Stored film, LUT, Grade, and Effects values are preserved. |
| Select LUT with a usable LUT declaration | The active mode becomes LUT. Stored film, Grade, and Effects values are preserved. |
| Select LUT without a usable declaration | Import or replacement is requested. Cancelling or failing leaves the preceding mode unchanged. |

A destination mode MUST validate its active groups before the mode becomes active.

A manual method switch is one adjustment-history transaction. It MUST clear the applied-preset identity and MUST NOT prepare stages inactive in the destination mode.

Entering Film or Off MUST NOT read a LUT file. Entering LUT or Off MUST NOT read spectral material.

A method switch preserves every stored group. It MUST NOT reset an inactive film, LUT, Grade, or Effects value.

Applying a preset selects the mode its kind requires in the same transaction. A preset document MUST NOT carry a separate mode field.

A failed method switch preserves the preceding mode, controls, applied identity, and retained image.

## Parameter language

The canonical path is `workflow.mode` with values `film`, `lut`, and `off`. An absent value resolves to `film`.

The render mode has three optional runtime groups:

- `lut`: the LUT declaration and strength;
- `effects`: Diffusion, RGB Light Leak, RGB Halation, and Grain;
- `grade`: the LUT Grade.

The former `lut_look` group MUST NOT appear in a new document. A one-time migration splits it into `lut`, `effects`, and `grade`.

| Mode | Active groups | Required validation | Inactive groups |
| --- | --- | --- | --- |
| Film | film chain, Scan Tone & Color, Output mix, CMYK | film route and profiles | `lut`, `effects`, `grade` |
| LUT | `lut`, `effects`, `grade` | LUT declaration and active effects or Grade | film chain, Output mix, CMYK |
| Off | `effects`, optional `grade`, output color management | active effects or Grade | film chain, `lut`, Output mix, CMYK |

Inactive groups are retained and ignored. An inactive LUT declaration MUST NOT be read or validated. An inactive invalid film, LUT, or effect value MUST NOT be read or validated for the active render.

An effect or LUT Grade group with `active=false` is inactive inside an active root. Its values are retained and MUST NOT be read or validated for the active render.

`effects` absent means every effect is off. `grade` absent means the LUT Grade is neutral.

`workflow.route` remains the spectral route and is active only in Film. `output_mix` and `cmyk` remain stored but inactive in LUT and Off.

## Persistence

Runtime parameters, GUI state, and render recipes MUST retain the active mode and every stored group. A complete preset kind selects its mode and MUST NOT store a mode field.

A Film preset selects Film; a LUT preset selects LUT; an Effects preset selects Off. A section preset remains Film-only and does not switch modes.

GUI state version 8 and recipe schema `spektrafilm-rs-params-8` own the split. Versions 1 through 7 that carry `lut_look` MUST migrate it once. The migration maps its LUT declaration to `lut`, nested effects to `effects`, and `grade_active` plus grade values to `grade.active` and `grade`.

A new writer MUST use `lut`, `effects`, and `grade` instead of `lut_look`; it MUST NOT emit `lut_look` or a compatibility alias.

## Failure

Active-mode validation MUST complete before output publication.

- LUT mode without a LUT declaration MUST fail.
- LUT mode with a missing file, changed hash, malformed carrier, or invalid declaration MUST fail.
- Off mode MUST NOT fail because a stored LUT declaration is missing or invalid.
- Film mode MUST NOT fail because a stored LUT declaration is missing or invalid.
- An invalid active effect MUST fail in LUT and Off.

A rejected mode or preset MUST leave the preceding accepted mode, controls, identity, and output unchanged.

## Acceptance

1. Off renders the active effects without a film or LUT operation; an active LUT Grade renders independently; a missing stored LUT file does not block it; output color management still applies.
2. Switching Film, LUT, and Off preserves stored groups, reads no inactive declaration, and creates one undo transaction.
3. LUT requires a valid declaration; Off does not; active effects and inactive groups follow the validation table.
