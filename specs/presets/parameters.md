# Preset parameters

A preset is a sparse parameter document. The GUI, CLI, presets, and API use one settable parameter set. A leaf belongs to that set or it is not settable through those surfaces. The CLI accepts a `debug` leaf only under `--debug`. A preset must not carry a `debug` leaf.

## Classes and counts

A `parameter` leaf belongs to the shared set and may appear in a preset. The evidence inventory contains 198 `parameter` leaves.

A `debug` leaf is a diagnostics, parity, or verification switch. The CLI accepts it only under `--debug`, and a preset must not carry it. The evidence inventory contains 20 `debug` leaves.

An `internal` leaf is module-owned and is not settable through these surfaces. The current evidence inventory contains zero `internal` leaves.

The GUI audit records 179 adjustable canonical leaves. Film and print profile selectors add the `film.stock` and `print.stock` leaves, each with GUI evidence. The audit records 37 leaves with no editable GUI binding. The established 20 `debug` leaves account for 20 of those no-binding leaves. The remaining 17 no-binding leaves belong to the `parameter` class. The 198 `parameter` leaves therefore comprise 179 adjustable canonical leaves, two profile-selection leaves, and 17 leaves without an editable GUI binding. Together with 20 `debug` leaves, the inventory contains 218 leaves.

## Capture

`capture` is a second attribute of a `parameter` leaf. A whole-look capture writes every leaf with `capture=true`. These leaves are values a user tunes while simulating. A `capture=false` leaf describes a source, geometry, delivery, viewer, or environment value. A document may carry a `capture=false` leaf explicitly.

## Leaves without an editable GUI control

The GUI audit records 37 leaves with no editable GUI binding. The 20 `debug` leaves remain in the `debug` class. The PR #161 evidence appendix records those leaves and the other leaves with GUI bindings. This section records the 17 non-debug leaves that need a control or class decision.

### Must add a control

- `camera.filter_uv` — GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:35-38`; the Camera panel bindings are at `crates/spektrafilm-gui/src/controls.rs:154-160`. A control must be added.
- `camera.filter_ir` — GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:36-38`; the Camera panel bindings are at `crates/spektrafilm-gui/src/controls.rs:154-160`. A control must be added.
- `camera.lens_blur_um` — GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:31-38`; the Camera panel bindings are at `crates/spektrafilm-gui/src/controls.rs:154-160`, while the visible blur control edits `scanner.lens_blur` at `crates/spektrafilm-gui/src/controls.rs:265`. A control must be added for this camera leaf.

### Owner decision pending

Each leaf below has no GUI binding. The owner must decide whether to add a control or move the leaf out of the `parameter` class. The evidence supports one option but does not settle the decision.

- `enlarger.illuminant` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no editable choice; the combo offers only `TH-KG3`. Evidence: `crates/spektrafilm-core/src/params/mod.rs:69-94`; `crates/spektrafilm-gui/src/controls.rs:525-528`. The evidence supports moving it out of the parameter class because the combo cannot change its value.
- `enlarger.normalize_print_exposure` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:75-77`; Enlarger bindings at `crates/spektrafilm-gui/src/controls.rs:163-173` do not bind this leaf. The evidence supports moving it out of the parameter class because no GUI control edits it.
- `enlarger.lens_blur` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:87-91`; the Enlarger Diffusion group is selected at `crates/spektrafilm-gui/src/controls.rs:518-522` and is separate from this direct lens-blur leaf. The evidence supports adding a control for the direct enlarger lens setting.
- `film_render.density_curve_gamma` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:407-410`; film GUI sections are mapped at `crates/spektrafilm-gui/src/ui.rs:636-680` without this binding. The evidence supports adding a control for this film-render setting.
- `film_render.grain.micro_sublayers` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/grain.rs:152-155`; `crates/spektrafilm-core/src/params/sources/effects.rs:154-164` identifies it as a preserved upstream no-op. The evidence supports moving it out of the parameter class because it has no effect.
- `film_render.grain.sublayers_active` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/grain.rs:157-158`; Grain bindings at `crates/spektrafilm-gui/src/controls.rs:391-479` do not expose it. The evidence supports moving it out of the parameter class because the GUI does not expose it.
- `film_render.grain.particle_area_um2` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/grain.rs:161-170`; the visible Dye `Event area` binds `v3_event_area_um2` at `crates/spektrafilm-gui/src/controls.rs:461-467`, not this leaf. The evidence supports moving it out of the parameter class because the GUI exposes a different leaf.
- `film_render.grain.particle_scale` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/grain.rs:165-174`; the GUI exposes `particle_scale_sublayers` at `crates/spektrafilm-gui/src/controls.rs:464-472`, not this leaf. The evidence supports moving it out of the parameter class because the GUI exposes a different leaf.
- `film_render.grain.n_sub_layers` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/grain.rs:179-181`; Grain bindings at `crates/spektrafilm-gui/src/controls.rs:391-479` do not bind this leaf. The evidence supports moving it out of the parameter class because no GUI control edits it.
- `print_render.density_curve_gamma` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:445-450`; PRINT bindings at `crates/spektrafilm-gui/src/ui.rs:636-680` do not bind this leaf. The evidence supports adding a control for this print-render setting.
- `io.output_cctf_encoding` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:551-558`; the saving-CCTF checkbox at `crates/spektrafilm-gui/src/controls.rs:531-535` is GUI `simulation` state, not this runtime leaf. The evidence supports moving it out of the parameter class because the GUI uses a different state field.
- `io.input_gamut_compress.algorithm` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no editable choice; the combo contains only `xy`. Evidence: `crates/spektrafilm-core/src/params/mod.rs:600-610`; `crates/spektrafilm-gui/src/controls.rs:321-325`. The evidence supports moving it out of the parameter class because the combo cannot change its value.
- `io.input_gamut_compress.boundary` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:600-608`; the GUI binds active, knee, and hull detail at `crates/spektrafilm-gui/src/controls.rs:321-325`, not boundary. The evidence supports moving it out of the parameter class because no GUI control edits it.
- `settings.use_cat16` — owner decision pending: add a control or move it out of the parameter class. GUI binding: no binding. Evidence: `crates/spektrafilm-core/src/params/mod.rs:712-719`; neighboring settings bindings at `crates/spektrafilm-gui/src/controls.rs:384-388` do not bind this leaf. The evidence supports moving it out of the parameter class because no GUI control edits it.

The PR #161 evidence appendix and these 17 decision entries account for all 218 leaves exactly once.