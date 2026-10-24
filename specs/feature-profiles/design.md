# Feature profile design

## Boundary

Feature profiles are a selection layer over existing runtime parameter groups. They are not a runtime stage, a serialized parameter group, or a replacement for look and section presets.

The feature profile resolver owns family identifiers, built-in catalog entries, scope extraction, exact value application, `custom` matching, and selector validation. The existing parameter model owns field types, numeric ranges, serialization, and execution semantics. The preset resolver owns preset documents and profile references.

The CLI parser owns option syntax but MUST call the shared resolver.

The current catalog covers Procedural grain and Halation. Dye has no feature-profile catalog in this algorithm revision. A future Dye appearance profile requires its own scope, parameter record, calibration evidence, and product specification.

## Options considered

### Extend look presets with a feature selector

This would reuse the existing library and save custom combinations. It would also make a feature selection carry profile identity into a document that currently stores resolved leaves, expand migration, and blur the boundary between a stock profile, a look, and a feature. It is rejected.

### Add a generic arbitrary feature-profile registry

This would make future families cheap to register, but it would require a generic scope language, heterogeneous value payloads, per-family applicability, and new persistence rules. It would duplicate the existing typed parameter model. It is rejected.

### Add typed, built-in family resolvers over existing groups

This keeps each family explicit and allows the CLI and GUI to share one small selection contract. It is the selected design. A later family MAY be added only with its own scope, catalog, applicability, and acceptance evidence.

## Domain model

A resolved feature profile is:

```text
FeatureProfileSelection {
    family: FeatureProfileFamily,
    name: FeatureProfileName,
    source: FeatureProfileSource,
}
```

`FeatureProfileFamily` has the stable values `grain.procedural` and `halation`. `FeatureProfileName` is an exact family-local identifier. `FeatureProfileSource` identifies GUI selection or one-based CLI option position for reporting; it does not enter runtime serialization.

The resolver returns a typed candidate, not a serialized overlay:

```text
FeatureProfileValues =
    Procedural(ProceduralProfileValues)
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

HalationProfileValues {
    halation_first_sigma_um,
    halation_strength,
}
```

`custom` is not a catalog row with defaults. It means that the caller keeps or explicitly exposes the current scoped values. Procedural uses its existing custom operation.

Halation derives `custom` by exact comparison and leaves its scoped values unchanged.

## Resolution invariants

1. A family resolver accepts only its own names. `35mm250` is valid for `grain.procedural`, and `still_strong` is valid for `halation`.
2. Every catalog entry resolves to finite values accepted by the existing runtime validation rules.
3. Application is scope-exact. It cannot write `active`, engine, mode, film type, route, workflow, seed, or another family.
4. Application is transactional. Resolve and validate a candidate copy before committing it to active state.
5. Reapplying the same named profile is idempotent for its scope and has no effect on unrelated controls.
6. `custom` never invents values. It preserves the current scoped values.
7. Feature profile values are explicit runtime values after application. Look capture and section capture serialize those values through existing carriers.
8. No runtime path MAY infer a feature profile name from a film stock, look name, or serialized values except for a family-defined `custom` projection.

## Family details

### Procedural

The existing `spektrafilm_model::grain::v2::PROFILES` table is the single code owner for the standard rows. The feature resolver MUST use that owner rather than copying numeric literals into callers.

Applying a named row MUST preserve the existing `v2_profile` behavior: the named profile supplies its standard values and optional overrides are cleared when exact selection is requested. The resolver MUST NOT alter `v2_mode`, `v2_film_type`, `v2_resolution_type`, `v2_timer`, seed, `engine`, or `active`.

The family is narrower than all Procedural controls. Cluster, color, timer, and mode controls are algorithm controls, not standard format/speed profile leaves.

### Halation

The existing `HALATION_PROFILES` entries are the single baseline source for the table. If the table cannot be called from the resolver without widening unrelated visibility, the implementation MUST move it to one crate-private typed owner. A second literal table MUST NOT be created.

The Halation profile scope excludes `active` and scatter, boost, source-compression, bounce, and renormalization controls. A profile selection MUST NOT change whether the effect runs or present an appearance baseline as a complete material calibration.

Selection identity MUST NOT permit a tolerance.

Stock defaults remain a separate initialization source. A user-selected profile MUST NOT be re-derived from film metadata after application.

## CLI composition

The parser MUST collect typed `FeatureProfileSelection` values while preserving option position. The shared resolver applies the last selection for each family in option order, then applies existing `--set` layers. The selection source is for reporting and does not enter runtime serialization.

The option uses one assignment token:

```text
--feature-profile grain.procedural=35mm250
--feature-profile halation=cine_weak
```

The parser MUST reject whitespace-dependent or ambiguous forms. It MUST NOT accept a generic grain assignment, dotted runtime paths, file names, or a preset selector in this option.

Direct leaves remain allowed after selection. The dry-run report MUST show the profile request and the winning leaf sources separately.

## GUI composition

The selector is a projection of current parameter values plus an action. It MUST NOT add a second persisted identity for Halation. Procedural's existing `v2_profile` remains persisted because it is already a public runtime selector.

The GUI control writes through the same typed resolver as the CLI. It MUST validate the candidate before updating state, invalidating render state, and scheduling preview. A disabled selector MUST retain its value and display the applicability reason.

## Persistence

Feature-profile selection is not serialized as a new top-level or nested field. A saved look or section contains resolved runtime leaves under its existing schema. A document MUST NOT require a catalog entry because it stores leaves rather than feature-profile names.

The existing Procedural `v2_profile` wire behavior remains authoritative for that engine. Halation documents continue to carry their resolved arrays. Catalog changes require an explicit product revision and MUST NOT silently reinterpret stored resolved values.

## Verification design

The implementation MUST add focused tests for:

- family-local name validation and catalog counts;
- every Procedural row resolving to the existing dispatch values;
- every Halation row resolving to its exact arrays;
- scope isolation with sentinels for active, engine, mode, film type, route, workflow, seed, film format, and unrelated controls;
- `custom` matching and non-matching behavior for Halation, with no tolerance;
- CLI option composition, repeated-family precedence, malformed values, dry-run reporting, and direct-leaf override;
- round-trip through existing preset and parameter carriers without a feature-profile field.

Behavioral verification MUST exercise a real CLI dry run and one real render for each family. It MUST compare profile selection with equivalent direct-leaf configuration under the same executable, backend, seed, and input.

No external binary, proprietary LUT, shader source, or extracted asset belongs in this repository. Research observations do not authorize copying proprietary assets or claiming parity.
