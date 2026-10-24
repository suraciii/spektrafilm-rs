# Feature profiles

## Authority

This document owns the feature profile concept, built-in catalog, profile scopes, and selection behavior. Feature specifications own execution and parameter semantics. [Presets](../presets/spec.md) owns reusable look and section documents.

## Contract

A **feature profile** is a named, built-in parameter set for one film-chain feature. It resolves only the leaves in its declared scope. It does not select a film profile, print profile, workflow, route, input, scan sensor, calibration state, GUI state, or complete look.

A **feature profile family** has one feature identifier, one profile scope, one validation rule, and one set of named entries. A family MAY expose `custom` as the state in which the user controls the scoped leaves directly. `custom` is not a built-in parameter set.

The application MUST resolve a feature profile before rendering. It MUST validate the family, name, scope values, and active parameter combination before changing controls. An invalid selection MUST leave controls, transient state, and render revision unchanged except for the error status.

Applying a feature profile MUST replace exactly the leaves in its profile scope. It MUST preserve every other control, including `active`, engine selection, film and print references, workflow, route, seed, and controls outside the profile scope. A profile MUST NOT enable or disable its feature as a side effect.

A profile selection is an edit to active controls. It MUST NOT rewrite a built-in look or section document. A later direct edit MAY produce a `custom` effective profile; it MUST NOT cause the application to reapply the previous named profile silently.

Feature profiles are built-in catalog entries. They are not a second user profile library. A user who wants a reusable custom combination MUST save the relevant controls as an existing section preset.

The GUI and CLI MUST resolve the same family identifiers, names, scope, validation, and values. A profile selected in either surface MUST produce the same resolved feature controls at the same precision boundary.

## Built-in families

The current catalog provides these families:

| Family identifier | Product label | Profile scope | Availability |
| --- | --- | --- | --- |
| `grain.procedural` | Procedural grain profile | The named Procedural grain profile values | When Procedural controls are available; selecting it does not change `film_render.grain.engine` |
| `halation` | Halation profile | `film_render.halation.halation_first_sigma_um` and `film_render.halation.halation_strength` | Whenever Halation controls are editable; inactive route conditions retain the values |

There is no Dye feature-profile family in this algorithm revision. A future Dye profile requires a separate parameter scope, calibration record, and acceptance contract.

There is no universal profile that maps one parameter set across Spektra, Procedural, Dye, and Fotufilm Film. Grain engines retain independent domains and controls under [Film grain](../film-grain/spec.md).

### Procedural grain profile family

The family retains the existing `v2_profile` identifiers and default. Names are serialized values, not display labels, and remain lower-case with no aliases.

| Name | Size | Amount | Shadows | Midtones | Highlights | Chroma | Film Resolution |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `8mm50` | 40 | 25 | 30 | 50 | 50 | 50 | 50 |
| `8mm250` | 48 | 50 | 25 | 45 | 65 | 65 | 70 |
| `8mm500` | 48 | 70 | 30 | 45 | 65 | 90 | 75 |
| `16mm50` | 25 | 25 | 30 | 55 | 65 | 65 | 65 |
| `16mm250` | 25 | 45 | 30 | 55 | 65 | 65 | 75 |
| `16mm500` | 25 | 70 | 30 | 45 | 65 | 75 | 80 |
| `35mm50` | 8 | 30 | 30 | 55 | 65 | 65 | 70 |
| `35mm250` | 12 | 35 | 30 | 55 | 65 | 65 | 75 |
| `35mm500` | 16 | 40 | 35 | 55 | 65 | 70 | 80 |
| `65mm50` | 1 | 10 | 30 | 45 | 55 | 65 | 90 |
| `65mm250` | 2 | 15 | 30 | 55 | 65 | 65 | 90 |
| `65mm500` | 3 | 25 | 25 | 55 | 65 | 80 | 100 |

Applying a named Procedural profile MUST set `v2_profile` to that name and clear optional profile overrides. It MUST preserve `v2_mode`, `v2_film_type`, `v2_resolution_type`, `v2_timer`, `active`, and `engine`.

Selecting `custom` preserves current resolved Procedural values and exposes them as explicit custom controls. It MUST NOT select an engine or change mode, film type, resolution implementation, or timer.

The catalog values are SpektraFilm values. They do not claim pixel parity with an external product. The algorithm and post-scan boundary remain those defined by [Procedural grain](../film-grain/v2/spec.md).

### Halation profile family

The Halation family exposes existing stock-class baseline pairs as named feature profiles. Names describe material-use and antihalation categories. They do not identify a film stock or claim measured stock calibration.

| Name | Display label | `halation_first_sigma_um` | `halation_strength` |
| --- | --- | --- | --- |
| `still_strong` | Still / strong antihalation | `[65, 65, 65]` | `[0.015, 0.005, 0]` |
| `still_weak` | Still / weak antihalation | `[65, 65, 65]` | `[0.08, 0.02, 0]` |
| `still_no` | Still / no antihalation | `[65, 65, 65]` | `[0.30, 0.10, 0.015]` |
| `cine_strong` | Cine / strong antihalation | `[50, 50, 50]` | `[0.015, 0.005, 0]` |
| `cine_weak` | Cine / weak antihalation | `[50, 50, 50]` | `[0.08, 0.02, 0]` |
| `cine_no` | Cine / no antihalation | `[50, 50, 50]` | `[0.30, 0.10, 0.015]` |

Applying a named Halation profile MUST replace only the two arrays in the table. It MUST preserve `active`, scatter controls, source compression, boost and protection controls, bounce controls, and renormalization. A user-selected profile takes precedence over later stock-default reapplication for its explicit leaves.

The catalog exposes existing Halation parameter arrays without claiming a separate measured-width or channel-strength model. The film-chain contract owns source response semantics, and the catalog does not claim measured point-spread functions.

`custom` is the effective name when the two scoped arrays do not equal a built-in entry. It preserves the current arrays and does not reset other controls.

## User surfaces

### GUI

Each applicable feature section MUST provide one profile selector. The selector MUST list built-in entries in stable order and `custom`. It MUST NOT repeat numeric controls owned by the same section.

The selector MUST show `custom` when current scoped values do not equal a built-in entry. It MUST NOT store a second persisted identity for Halation. Procedural retains its existing `v2_profile` wire field.

Selecting an entry MUST update only its profile scope and mark the feature section as edited. The selector MUST explain inactive route or engine conditions without clearing stored values.

### CLI

`process` and `process --dry-run` MUST accept the repeatable option:

```text
--feature-profile FAMILY=NAME
```

`FAMILY` MUST be `grain.procedural` or `halation`. `NAME` MUST be a built-in name from this document or `custom`. Malformed assignments, unknown families, and unknown names MUST fail.

The option MUST NOT fall back to a parameter path or look preset name.

Feature profile options apply in command-line order before `--set` sources. A later selection for the same family replaces the earlier selection. `--set` sources then apply in their existing order and MAY override the profile's scoped values.

The dry-run report MUST include each requested profile with family, name, source, scope, and effective status. CLI and GUI MUST use the same identifiers and names.

Feature profile selection MUST NOT be accepted inside a preset document, sparse parameter file, or `parameters` wrapper. Those carriers contain resolved runtime leaves only.

## Composition and persistence

Resolution MUST use this order:

1. Resolve film and print references and any look or section preset sources.
2. Apply feature profile selections in their appearance order.
3. Apply sparse `--set` sources in their appearance order.
4. Apply explicit workflow, scan-output, and input choices.
5. Validate composed runtime controls and apply ordinary preview and backend constraints.

A GUI feature-profile action has the same position as step 2 relative to an already applied look and later direct edits. Applying a look after a feature profile MAY replace look-owned values.

Feature-profile selection is not serialized as a new top-level or nested field. A saved look or section contains resolved runtime leaves under its existing schema. A document MUST NOT require a catalog entry because it stores leaves rather than profile names.

## Acceptance

1. Every built-in family and name in this document resolves to the exact scope values in its table. Dye is not part of this catalog.
2. An unknown family or name fails before render publication and leaves active controls unchanged.
3. Applying a profile changes only its declared scope. Tests MUST retain sentinels in out-of-scope controls, including `active`, route, workflow, engine, and seed.
4. A direct Halation scope edit reports `custom` and preserves existing Procedural profile behavior.
5. CLI and GUI selection resolve identical values. A profile run and equivalent direct-leaf `--set` run produce identical output for the same executable, backend, seed, and input.
6. Profile selection does not create a second preset library or add profile references or external-product fields to carriers.
7. Existing look presets, section presets, sparse parameter files, GUI state, Procedural profile values, Halation stock defaults, CPU execution, GPU execution, and inactive-value behavior remain compatible when no feature profile is selected.
8. Catalog changes require an explicit product revision and MUST NOT silently reinterpret stored resolved values.

## Reference basis

The Procedural profile relationships are informed by public research notes. SpektraFilm MUST keep its own parameter names, execution contracts, attribution, and acceptance evidence. Research observations do not authorize copying proprietary assets or claiming parity.
