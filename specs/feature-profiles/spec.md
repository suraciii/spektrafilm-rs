# Feature profiles

## Authority

This document owns the feature profile concept, the built-in catalog, profile scopes, and selection behavior. Feature specifications own the execution and parameter semantics. [Presets](../presets/spec.md) owns reusable look and section documents.

## Contract

A **feature profile** is a named, built-in parameter set for one film-chain feature. It resolves only the leaves in its declared profile scope. It does not select a film profile, print profile, workflow, route, input, scanner receiver, calibration state, GUI state, or complete look.

A **feature profile family** has one feature identifier, one profile scope, one validation rule, and one set of named entries. A family may expose `custom` as the state in which the user controls the scoped leaves directly. `custom` is not a built-in parameter set.

The application must resolve a feature profile before rendering. It must validate the family, name, scope values, and active parameter combination before changing controls. An invalid selection must leave controls, the applied look marker, transient state, and render revision unchanged except for the error status.

Applying a feature profile must replace exactly the leaves in its profile scope. It must preserve every other control, including `active`, engine selection, film and print references, workflow, route, seed, and controls outside the profile scope. A profile must not enable or disable its feature as a side effect.

A profile selection is an edit to the active controls. It must not rewrite a built-in look or section document. A later direct edit may produce a `custom` effective profile; it must not cause the application to reapply the previous named profile silently.

Feature profiles are built-in catalog entries in version 1. They are not a second user profile library. A user who wants a reusable custom combination must save the relevant controls as an existing section preset. A section preset remains the portable source of user-authored feature values.

The GUI and CLI must resolve the same family identifiers, names, scope, validation, and values. A profile selected in either surface must produce the same resolved feature controls at the same precision boundary.

## Built-in families

Version 1 provides these families:

| Family identifier | Product label | Profile scope | Availability |
| --- | --- | --- | --- |
| `grain.procedural` | Procedural grain profile | The named Procedural grain profile values | When the Procedural grain controls are available; selecting it does not change `film_render.grain.engine` |
| `grain.dye` | Dye grain profile | `film_render.grain.v3_dye_support_um`, `film_render.grain.v3_event_area_um2`, and `film_render.grain.channel_correlation` | When the Dye grain controls are available; selecting it does not change `film_render.grain.engine` |
| `halation` | Halation profile | `film_render.halation.halation_first_sigma_um` and `film_render.halation.halation_strength` | Whenever Halation controls are editable; inactive route conditions retain the values |

There is no universal profile that maps one parameter set across Spektra, Procedural, Dye, and Fotufilm Film. The four grain engines retain their independent domains and controls under [Film grain](../film-grain/spec.md). Procedural and Dye have separate feature profile families; additional engine-specific families require their own parameter scope and acceptance evidence.

### Procedural grain profile family

The family retains the existing `v2_profile` identifiers and the existing default. The names are serialized values, not display labels, and must remain lower-case with no aliases.

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

`35mm250` is the default. Size is in the existing Procedural profile scale domain. The other columns are percentages in the existing `0..=100` domains. Film Resolution retains its existing reversed meaning: `100` preserves the most source detail and `0` applies the strongest resolution limit.

Applying a named Procedural profile must set `v2_profile` to that name and clear the optional profile overrides `v2_size`, `v2_amount`, `v2_shadows`, `v2_midtones`, `v2_highlights`, `v2_chroma`, and `v2_resolution_factor`. It must preserve `v2_mode`, `v2_film_type`, `v2_resolution_type`, `v2_timer`, `active`, and `engine`. This produces the exact table entry without changing the existing algorithm or wire names.

Selecting `custom` must preserve the currently resolved Procedural values and expose them as explicit custom controls. It must not select an engine or change the mode, film type, resolution implementation, or timer. The existing `custom` and `v2_profile` compatibility rules remain authoritative for serialized documents and direct parameter edits.

The catalog shape and values are retained from SpektraFilm's existing Dehancer-derived Procedural model. They are SpektraFilm built-in values, not imported Dehancer files and not a claim of pixel parity with Dehancer. The algorithm and post-scan boundary remain those defined by [Procedural grain](../film-grain/v2/spec.md).

### Dye grain profile family

The Dye family provides named appearance parameter sets for the self-developed film-coordinate dye-field engine. It does not select `film_render.grain.engine`; the caller must select `dye` through the existing engine control. The family does not replace stock-resolved `rms_granularity`, `particle_scale_sublayers`, `density_min`, or `uniformity`, because those values belong to film chemistry and the existing stock granularity resolution.

The catalog contains six built-in profiles in stable name order:

| Name | Display label | `v3_dye_support_um` | `v3_event_area_um2` | `channel_correlation` |
| --- | --- | ---: | ---: | ---: |
| `reference` | Reference | 3.5 | 1.28 | 0.0 |
| `fine` | Fine | 1.0 | 1.28 | 0.0 |
| `coarse` | Coarse | 8.0 | 1.28 | 0.0 |
| `subtle` | Subtle | 3.5 | 0.32 | 0.0 |
| `strong` | Strong | 3.5 | 5.12 | 0.0 |
| `correlated` | Correlated | 3.5 | 1.28 | 0.45 |

`reference` uses the existing Dye parameter defaults. Those defaults were validated on the `kodak_portra_800` reference condition, but the profile is not a stock selector and does not infer or change the active film profile. The other five rows keep the `reference` default for the two controls they do not vary. The catalog has one size axis (`reference`, `fine`, `coarse`) and one strength axis (`reference`, `subtle`, `strong`), plus the one accepted correlation point. This shape is a catalog layout choice; the values come from the local Dye knob measurements, not from any other product's profile table.

The three controls have distinct meanings in the Dye model: support controls the spatial operator, event area controls statistical strength, and channel correlation controls the shared count component. A built-in row must provide evidence for the complete combination and must not be introduced by copying values from Procedural grain, Spektra grain, or a film stock's unrelated granularity controls.

#### Measurement condition and evidence caveats

The support/event-area measurements for `reference`, `fine`, `coarse`, `subtle`, and `strong` use one fixed CPU scan condition: the release CLI on the CPU backend, film `kodak_ultramax_400`, route `input > film > scan`, scan output `negative_to_positive`, seed `20261013`, `camera.film_format_mm = 8.96` (1400 px at 6.4 µm), stock granularity `rms_granularity = [8, 8, 8]`, a flat mid-gray input, and one metrics window. The `correlated` row uses the accepted `channel_correlation` ceiling and was exercised separately in the CPU/GPU Dye acceptance matrix. The reference row's own validation remains the `kodak_portra_800` reference condition defined by the Dye contract.

A measured outcome is not a general parameter claim. At the support/event-area measurement condition, event area moved the 48-micrometer high-pass standard deviation by the square-root law (`subtle` 0.503×, `strong` 1.964×) with the support unchanged, and support moved both the correlation length and the amplitude (`fine` 2.157×, `coarse` 0.402×). The separate correlated-row evidence exercises the accepted 0.45 setting; it does not turn that setting into stock calibration. Three rows carry explicit caveats:

- `fine` has an apparent scale that the output pitch limits. Its 6.4 µm correlation length is one output pixel of that measurement, so the measurement establishes the sampling, not a film-space correlation length. Its field-cell pitch is also the finest in the catalog.
- `coarse` is softer than the measured real film at that condition, not an improvement over it.
- `correlated` sits at the accepted evidence ceiling of `channel_correlation`. The configured value is an upper bound on the realized correlation, and the appearance contract continues to forbid imitating scanner chroma processing.

The rows are appearance selections under these declared evidence conditions. They are not validated for the `kodak_portra_800` reference condition, are not stock calibrations, and do not claim Portra, CineStill, or Dehancer equivalence. The [Dye grain contract](../film-grain/v3/spec.md) remains responsible for film-coordinate formation, conditional Poisson counts, three-sublayer allocation, stock granularity, film-format scaling, the supported route, and the meaning and evidence limits of `channel_correlation`.

Applying a named Dye profile must replace only the three profile-scope leaves in its row. It must preserve `active`, `engine`, all other grain controls, the resolved stock values, `camera.film_format_mm`, route, workflow, and seed. It must not change `rms_granularity`, `particle_scale_sublayers`, `density_min`, `uniformity`, or any film profile reference.

`custom` is the effective name when the three scoped values do not exactly match any catalog row. Selecting `custom` preserves the current values. Matching uses exact declared f32/f64 values; no tolerance is used for profile identity.

The existing accepted range for `channel_correlation` remains `[0, 0.45]`. The `reference` and `correlated` rows hold the two ends of that range. The [Dye grain contract](../film-grain/v3/spec.md#channel-correlation) remains authoritative for the meaning and evidence limits of this control.

### Halation profile family

The Halation family exposes the existing stock-class baseline pairs as named feature profiles. The names describe material-use and antihalation categories. They do not identify a film stock and do not claim measured stock calibration.

| Name | Display label | `halation_first_sigma_um` | `halation_strength` |
| --- | --- | --- | --- |
| `still_strong` | Still / strong antihalation | `[65, 65, 65]` | `[0.015, 0.005, 0]` |
| `still_weak` | Still / weak antihalation | `[65, 65, 65]` | `[0.08, 0.02, 0]` |
| `still_no` | Still / no antihalation | `[65, 65, 65]` | `[0.30, 0.10, 0.015]` |
| `cine_strong` | Cine / strong antihalation | `[50, 50, 50]` | `[0.015, 0.005, 0]` |
| `cine_weak` | Cine / weak antihalation | `[50, 50, 50]` | `[0.08, 0.02, 0]` |
| `cine_no` | Cine / no antihalation | `[50, 50, 50]` | `[0.30, 0.10, 0.015]` |

Applying a named Halation profile must replace only the two arrays in the table. It must preserve `active`, scatter controls, source compression, boost and protection controls, bounce controls, and renormalization. The existing `HALATION_PRESETS` stock-default behavior may select the same values from film metadata, but a user-selected feature profile takes precedence over later stock-default reapplication for its explicit leaves.

The `halation` family does not claim that these six entries are measured point-spread functions. Halation width, channel strength, exposure geometry, and film-format scaling remain the semantics of the existing [film halation contract](../film-chain/spec.md#film-halation-source-response).

`custom` is the effective name when the two scoped arrays do not exactly match a built-in entry. It preserves the current arrays and does not reset any other Halation control.

## User surfaces

### GUI

Each applicable feature section must provide one profile selector. The selector must list the built-in entries in stable name order and `custom`. It must not repeat the numeric controls owned by the same section.

The selector must show `custom` when the current scoped values do not equal a built-in entry. The selector must not store a second identity field for Halation. Its displayed name is derived from the current scoped values. Procedural grain retains the existing `v2_profile` wire field and its compatibility behavior.

Selecting an entry must update only its profile scope and mark the feature section as edited. The selector must explain an inactive route or engine condition without clearing the stored values. The GUI must not silently switch `film_render.grain.engine` when a Procedural profile is selected.

### CLI

`process` and `process --dry-run` must accept the repeatable option:

```text
--feature-profile FAMILY=NAME
```

`FAMILY` must be `grain.procedural`, `grain.dye`, or `halation`. `NAME` must be a built-in name from this document or `custom`. The option must be rejected for malformed assignments, unknown families, and unknown names. It must not fall back to a parameter path or a look preset name.

Feature profile options apply in command-line appearance order before `--set` sources. A later profile selection for the same family replaces the earlier selection. `--set` sources then apply in their existing appearance order and may override the profile's scoped values. A direct `--set` edit must not change the meaning of another family or select a workflow.

The dry-run report must include each requested feature profile with its family, name, source, scope, and effective status. A profile that is stored but inactive must be reported as inactive with the same condition used by field discovery. CLI and GUI must use the same canonical family identifiers and profile names.

Feature profile selection must not be accepted inside a preset document, a sparse parameter file, or a `parameters` wrapper. Those carriers contain resolved runtime leaves only. Existing preset and parameter-file unknown-field rules remain unchanged.

## Composition and persistence

Resolution must use this order:

1. Resolve film and print references and any look or section preset sources.
2. Apply feature profile selections in their appearance order.
3. Apply sparse `--set` sources in their appearance order.
4. Apply explicit workflow, scan-output, and input choices.
5. Validate the composed runtime controls and apply ordinary preview and backend constraints.

A GUI feature-profile action has the same position as step 2 relative to an already applied look and later direct edits. Applying a look after a feature profile may replace the look-owned values, as existing look application requires.

Look and section preset capture must write the resolved parameter leaves. A selected Dye profile must therefore round-trip as its three existing leaves, a selected Halation profile as its two arrays, and a selected Procedural profile with its existing `v2_profile` value and optional-field rules. Loading an old document must not require a feature profile catalog entry.

Feature profile values must remain valid when the selected profile or route makes the feature inactive. Inactive values remain stored and become effective if the caller later selects a compatible route or engine. A feature profile must never alter route eligibility by itself.

## Compatibility and acceptance

1. Every built-in family and name in this document resolves to the exact scope values in its table. The Dye family has exactly six built-in names — `reference`, `fine`, `coarse`, `subtle`, `strong`, and `correlated` — each resolving to its exact declared triple; `35mm250` remains the Procedural default.
2. An unknown family or name fails before render publication and leaves active controls unchanged.
3. Applying a profile changes only its declared scope. A test must retain a sentinel value in every out-of-scope control, including `active`, route, workflow, engine, and seed.
4. A direct scope edit reports `custom` for Dye and Halation and preserves the existing Procedural `v2_profile` and optional override semantics.
5. CLI and GUI selection resolve identical values. A feature-profile run and an equivalent direct-leaf `--set` run produce identical output for the same executable, backend, seed, and input.
6. Feature profile selections do not create a second preset library, do not add profile references to look documents, and do not add Dehancer-specific fields to any carrier.
7. Existing look presets, section presets, sparse parameter files, GUI state, `v2_profile` values, Halation stock defaults, CPU execution, GPU execution, and inactive-value behavior remain compatible when no feature profile is selected.
8. Standard catalog changes require an explicit model or product revision and must not reinterpret stored resolved Halation values or old Procedural profile identifiers. A new Dye row must not reinterpret a stored document: the document keeps its three resolved leaves, and only the derived `custom` display state can change when those leaves newly match a built-in row.

## Reference basis

The profile-family shape and the Procedural grain parameter relationships are informed by the public research index and reverse-engineering notes in [`suraciii/dehancer-re`](https://github.com/suraciii/dehancer-re). SpektraFilm must keep its own parameter names, execution contracts, attribution, and acceptance evidence. Research observations do not authorize copying proprietary assets or claiming Dehancer parity.

The profile catalog is intentionally smaller than the research repository's complete film-profile and look-preset inventory. Film profiles and look presets remain separate concepts.
