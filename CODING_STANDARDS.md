# Coding review standards

This file contains reviewer-only judgment rules. Read it when a change touches a data structure, serialized document, default, enum wire value, or module boundary.

## Data structures and wire models

A review is complete only when it names the authoritative owner, the affected wire behavior, the consumers that depend on it, and the evidence that compatibility is preserved or intentionally changed.

- Identify the domain owner before reviewing an extraction or redesign. Keep one typed model authoritative. An adapter may translate to a wire model, but it must not create a second competing parameter vocabulary.
- Follow the owning capability specification and design document before evaluating fields, defaults, enum names, optionality, or nested boundaries. Product behavior belongs in `spec.md`; ownership and invariants belong in `design.md`.
- Treat Serde attributes as wire behavior. Review `deny_unknown_fields`, `default`, `skip`, renames, custom deserializers, omitted fields, JSON `null`, and version migrations together with their consumers.
- Trace every structural change across runtime parameters, CLI sources, presets, GUI state, recipes, telemetry reports, LUT bundles, and generated artifacts when those surfaces consume the type. A compile pass does not prove wire compatibility.
- Prefer a narrow domain module over a line-count split. A child module must own a coherent data boundary and must not duplicate validation, defaults, or serialization rules already owned elsewhere.
- Preserve unknown-field rejection, omitted-field behavior, nullability, numeric precision, enum spelling, and schema-version policy unless the governing specification changes in the same patch.
- Use existing round-trip, rejection, migration, and real-consumer tests. Add a permanent test only for a consumer-visible boundary or an uncertain compatibility edge; do not test field copies or serializer wiring in isolation.

## Navigation pointers

Use these pointers to reach the authoritative source for the affected model:

- Runtime parameters and sparse sources: `crates/spektrafilm-core/src/params/mod.rs`, `crates/spektrafilm-core/src/params/`, `specs/cli-parameters/spec.md`, and `specs/cli-parameters/design.md`.
- Presets and GUI state: `crates/spektrafilm-core/src/presets.rs`, `crates/spektrafilm-core/src/presets/`, `crates/spektrafilm-gui/src/state.rs`, `specs/presets/`, and `specs/gui-workflow/`.
- Telemetry reports and observations: `crates/spektrafilm-core/src/telemetry/`, `crates/spektrafilm-gpu/src/telemetry.rs`, `specs/telemetry/spec.md`, and `specs/telemetry/metrics.md`.
- LUT bundle wire data: `crates/spektrafilm-core/src/lut_baker/bundle.rs`, `crates/spektrafilm-core/src/lut_delivery.rs`, and `specs/lut-delivery/`.
- CLI workflow structures: `crates/spektrafilm-cli/src/`, `specs/cli-workflow/spec.md`, and `specs/cli-parameters/spec.md`.

## Verification evidence

Run the relevant commands from `Justfile`. Use `just test-fast` for deterministic model and state checks before a full gate. If a local `just ci` run reaches the shell timeout during the long workspace tests, run `just test` and `just test-f64` separately and report each result; a partial gate is not a full pass.
