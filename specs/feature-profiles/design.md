# Feature profile design

## Boundary

Feature profiles are a selection layer over existing runtime parameter groups. They are not a new runtime stage, a new serialized parameter group, or a replacement for look and section presets.

The feature profile resolver owns family identifiers, built-in catalog entries, scope extraction, exact value application, `custom` matching, and selector validation. The existing parameter model owns field types, numeric ranges, serialization, and execution semantics. The preset resolver owns preset documents and profile references. The CLI parser owns option syntax but must call the shared feature profile resolver.

The first implementation should reuse the existing Procedural profile table and `HALATION_PRESETS` values. The Dye family owns a one-row catalog whose `reference` values are the existing model defaults; it must not create speculative alternatives. It must not duplicate any authoritative table in GUI or CLI code. The authoritative product catalog is this specification; code tables must be checked against it.

## Options considered

### Extend look presets with a feature selector

This would reuse the existing library and save custom combinations. It would also make a feature selection carry profile identity into a document that currently stores resolved leaves, expand preset migration, and blur the boundary between a stock profile, a look, and a feature. It is rejected for version 1.

### Add a generic arbitrary feature-profile registry

This would make future families cheap to register, but it would require a generic scope language, heterogeneous value payloads, per-family applicability, and new persistence rules before a second family exists. It would duplicate the existing typed parameter model. It is rejected for version 1.

### Add typed, built-in family resolvers over existing groups

This keeps each family explicit, preserves current fields and algorithms, and allows the CLI and GUI to share one small selection contract. It is the selected design. A later family may be added only with its own scope, catalog, applicability, and acceptance evidence.

## Domain model

A resolved feature profile is:

```text
FeatureProfileSelection {
    family: FeatureProfileFamily,
    name: FeatureProfileName,
    source: FeatureProfileSource,
}
```

`FeatureProfileFamily` has the stable values `grain.procedural`, `grain.dye`, and `halation`. `FeatureProfileName` is an exact family-local identifier. `FeatureProfileSource` identifies GUI selection or one-based CLI option position for reporting; it does not enter runtime serialization.

The resolver returns a typed candidate, not a serialized overlay:

```text
FeatureProfileValues =
    Procedural(ProceduralProfileValues)
  | Dye(DyeProfileValues)
  | Halation(HalationProfileValues)
```

The candidate carries only the declared scope:

```text
ProceduralProfileValues {
    v2_profile,
    v2_size,
    v2_amount,
    v2_shadows,
    v2_midtones,
    v2_highlights,
    v2_chroma,
    v2_resolution_factor,
}

DyeProfileValues {
    v3_dye_support_um,
    v3_event_area_um2,
    channel_correlation,
}

HalationProfileValues {
    halation_first_sigma_um,
    halation_strength,
}
```

`custom` is not represented as a catalog row with defaults. It means that the caller keeps or explicitly exposes the current scoped values. For Procedural, the existing `select_custom_grain_v2` operation materializes the resolved named values into the nullable custom fields. For Dye and Halation, `custom` is derived by exact comparison and leaves the scoped values unchanged.

## Resolution invariants

1. A family resolver accepts only its own names. `35mm250` is valid for `grain.procedural`, `reference` is valid for `grain.dye`, and neither is valid for `halation`.
2. Every catalog entry resolves to finite values accepted by the existing `RuntimeParams::validate` rules.
3. Application is scope-exact. It cannot write `active`, engine, mode, film type, route, workflow, seed, or another family.
4. Application is transactional. Resolve and validate a candidate copy before committing it to active state.
5. Reapplying the same named profile is idempotent for the declared scope and has no effect on unrelated controls.
6. `custom` never invents values. It preserves the current resolved values and only changes the existing Procedural profile marker and overrides required by its current compatibility contract.
7. Feature profile values are explicit runtime values after application. Look capture and section capture serialize those values through existing carriers.
8. No runtime path may infer a feature profile name from a film stock, look name, or serialized resolved arrays except for the existing derived `custom` display state.

## Family details

### Procedural

The existing `spektrafilm_model::grain::v2::PROFILES` table is the single code owner for the twelve standard rows. The feature resolver must use `profile_index` and `GrainParams::select_custom_grain_v2` or equivalent existing methods rather than copying numeric literals into callers.

Applying a named row must preserve the existing `v2_profile` behavior: the named profile supplies its standard values, while optional overrides are cleared when the action means exact selection. The model's `resolved_grain_v2` remains the only conversion from serialized parameters to dispatch parameters. The feature resolver must not alter `v2_mode`, `v2_film_type`, `v2_resolution_type`, `v2_timer`, seed, `engine`, or `active`.

The procedural family is intentionally narrower than all grain parameters. `cluster_size`, `rotation`, colored/clustered behavior, timer, and mode are algorithm controls and are not part of the standard format/speed profile rows. A future family for those controls would need separate evidence.

### Dye

The Dye family owns only the three appearance controls that the Dye model itself consumes directly: support sigma, base event area, and channel correlation. It must not own stock-resolved `rms_granularity`, `particle_scale_sublayers`, `density_min`, or `uniformity`; those values are derived from the selected film and its chemistry. It must not own `camera.film_format_mm`, because film format is a shared camera control and the Dye contract requires one global source.

The initial catalog contains one typed row, `reference`, with the existing validated values `3.5`, `1.28`, and `0.0`. The resolver must preserve the existing field boundaries: `v3_dye_support_um` is `f32`, while `v3_event_area_um2` and `channel_correlation` are `f64`. This is deliberately not a stock selector. Repository presets provide the same two default values for the reference condition, but no measured alternative combination exists for these three feature leaves. The resolver must not invent `fine`, `strong`, or correlated rows from unrelated Procedural or Spektra controls.

The three leaves retain their model meanings: support controls the spatial operator, event area controls statistical strength, and channel correlation controls the shared count component. A future Dye row requires evidence for the complete combination and must remain within the existing validator range. No row changes the engine, route, film format, or stock resolution.

The Dye profile scope is deliberately narrower than the full Dye control surface. `rms_granularity`, `particle_scale_sublayers`, `density_min`, and `uniformity` remain editable where their existing GUI and CLI surfaces expose them, but their values belong to stock resolution and are not standard feature-profile catalog entries. A user-authored combination of those controls must use the existing grain section preset.

Exact matching compares the three declared values without tolerance. A direct edit to any one of them makes the effective profile `custom` unless all three values equal the `reference` row. The resolver must not normalize, clamp, or retune a value to make it match.

### Halation

The existing six `HALATION_PRESETS` entries are the single baseline source for the table. If that private table cannot be called from the resolver without widening unrelated visibility, move the table to one crate-private typed owner and let both stock-default digestion and feature selection call it. Do not create a second six-row literal table.

The Halation profile scope excludes `active` and every scatter, boost, source-compression, bounce, and renormalization control. This prevents a profile selection from changing whether the effect runs or from presenting an appearance baseline as a complete physical/material calibration.

Exact matching compares all three components of both arrays in their declared f64 representation. Any finite custom edit makes the displayed profile `custom` unless it exactly equals one built-in row. No tolerance is allowed for selection identity.

Stock defaults remain a separate initialization source. Their precedence must be explicit in the runtime composition design: stock initialization may seed values before user edits, while an explicit feature profile or section/preset leaf must survive later default preparation. A profile selection must not be re-derived from the current film metadata after the user applies it.

## CLI composition

The parser should collect typed `FeatureProfileSelection` values while preserving option position. The shared process resolver applies the last selection for each family in option order, then applies existing `--set` layers. It must record the source for dry-run reporting without putting the selection into `RuntimeParams` or any carrier.

The option uses one assignment token because the family and name are one choice:

```text
--feature-profile grain.procedural=35mm250
--feature-profile grain.dye=reference
--feature-profile halation=cine_weak
```

The parser must reject whitespace-dependent or ambiguous forms. It must not accept `grain=...`, dotted runtime paths, file names, or a preset selector in this option.

Equivalent direct leaves are deliberately allowed after selection. This makes the profile option a convenience and discoverable action, not a second parameter language. The final dry-run report must show the profile request and the winning leaf sources separately.

## GUI composition

The selector is a projection of the current parameter values plus an action. It must not add a second persisted profile identity for Halation. Procedural's existing `v2_profile` remains persisted because it is already a public runtime selector and compatibility field.

The GUI control writes through the same typed resolver used by the CLI. It must validate the candidate before updating `GuiState`, invalidating render state, and scheduling preview. A disabled selector must retain its current value and display the applicability reason.

## Persistence and compatibility

No feature-profile selection is serialized as a new top-level or nested field in version 1. A saved look or section contains the resulting runtime leaves under its existing schema. This makes a document portable even when the built-in catalog changes.

The existing Procedural `v2_profile` string remains accepted with the same twelve names and `custom`; no alias is introduced. Existing Dye documents continue to carry the three leaves only, and existing Halation documents continue to carry arrays only. An old document must not fail because a named Dye or Halation profile is absent from a future catalog revision, since those documents do not carry feature-profile names.

Catalog revisions must be versioned by the owning model/product revision when values change. A changed standard row must not silently reinterpret an existing serialized `v2_profile` selection in a way that changes old renders without a declared compatibility decision. The initial implementation therefore keeps the existing table values unchanged.

## Verification design

The implementation must add focused tests for:

- family-local name validation and exact catalog counts;
- every Procedural row resolving to the existing `GrainV2Params` values;
- the single Dye `reference` row resolving to `3.5`, `1.28`, and `0.0`, with the normal validator accepting it;
- every Halation row resolving to the exact f64 arrays;
- scope isolation with sentinels for active, engine, mode, film type, route, workflow, seed, stock granularity, film format, and unrelated controls;
- `custom` matching and non-matching behavior;
- CLI option composition, repeated-family precedence, malformed values, dry-run reporting, and direct-leaf override;
- round-trip through existing preset and parameter carriers without a feature-profile field.

Behavioral verification must exercise a real CLI dry run and one real render for each family. It must compare each profile invocation with the equivalent direct-leaf configuration under the same executable, backend, seed, and input. Existing tests for no-option behavior, look application, stock defaults, inactive effects, and CPU/GPU execution must continue to pass.

No Dehancer binary, proprietary LUT, shader source, or extracted asset belongs in this repository. The external research notes remain evidence only; the implementation must use the existing SpektraFilm model and its own attribution policy.
