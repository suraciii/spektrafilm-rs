# Preset design

- Vocabularies: a **preset** is a named parameter set — the user-facing one is a library document (`Built-in` compiled from the crate's `assets/presets`, `My Presets` read from `<config_dir>/presets`). `data/presets/` keeps the upstream scope, the per-stock grain and coupler tables the runtime compiles in.
- A **look** is the content a preset owns: the look controls and their values. A **look preset** is the whole-look kind and a **section preset** the one-section kind; the terms *look document* and a `looks` directory MUST NOT appear.
- The per-stock grain and coupler tables in `data/presets/` are **not** upstream data. Pinned Python 0.3.4 carries no such database; it applies hardcoded polarity overrides in `runtime/params_builder.py` (`is_negative` / `is_positive` branches).
- The Rust tables replace those overrides with a stock → cine → defaults lookup and their own optimized values. Any parity claim that touches `film_render.dir_couplers` or `film_render.grain` therefore compares structure and magnitude, not sample equality; a numerical diff against pinned Python is expected and is not a defect.
- *Calibration* stays reserved for the Convert device-correction matrix and its worker. Names that mirror the pinned upstream (`apply_grain_preset`, `HALATION_PRESETS`, `procedural_profile` and its values) keep their upstream spelling for parity traceability.
- The preset document, strict validation, resolution, capture, the library directory policy, discovery, name conflict rejection, and atomic writes belong to `core`.
- The Rust document type is `presets::Preset`, named after its module like `profile::Profile`; `LookParameters` remains the look payload. Its `settings` group (`LookSettingsParams`) carries only the look-relevant processing settings (hanatos2025 adaptation window/surface, spectral Gaussian blur, CAT16) — not the wavelength grid, LUT wiring, preview limits, or performance toggles.
- Built-in preset documents are repository files in `crates/spektrafilm-core/assets/presets`. The build MUST compile them in, so built-in listing and name resolution MUST NOT require a data directory, and packaged data directories MUST NOT carry copies.
- A document is a **look preset** (both profile references and every required look-owned group, with the optional camera group defined by the [preset file format rules](spec.md#file-formats)) or a **section preset** (exactly one section, no profile reference).
- The kind is derived from the carried scope, never declared by a field, and a built-in document MAY be either kind. A document that carries neither a whole look nor exactly one section MUST be rejected.
- A section preset MUST pass the same value validators as a look preset for the paths it carries. Its payload MUST follow the parameter-file leaf discipline: only supplied leaves are edited, unknown fields and duplicate keys fail, and omitted values MUST NOT be filled from defaults.
- Applying it edits exactly its section and registers those values as explicit, so neither stock initialization nor database calibration rewrites them, and it MUST NOT clear explicit values outside its section.
- GUI application MUST commit one transaction under [Edit history](../gui-workflow/spec.md#edit-history). The transaction MUST retain explicit-leaf provenance and dependent normalized values, not only the carried payload; Undo and Redo MUST NOT reverse library writes.
- Writes, atomicity, rename, delete, duplicate, import, and export MUST treat both kinds through one code path. The resolver MUST reuse the parameter-file leaf-merge implementation for section presets instead of adding a second merge.
- The library directory is `<config_dir>/presets`. The configuration directory policy is shared by the GUI and the CLI.
- `SPEKTRAFILM_CONFIG_DIR` MUST override it; there is no separate preset directory override. The library directory MUST NOT require a data directory.
- Reusable combinations of controls that are already preset-owned belong in the existing look or section preset. They use the preset library, ownership table, strict validators, and shared resolver; there is no second cross-domain configuration document or selector.

## Material defaults

Core MUST own one implementation of [material defaults](spec.md#material-defaults) shared by GUI material selection and CLI preparation. It MUST retain the existing stock → cine → class/polarity lookup where applicable, including partial stock entries, before using schema fallback.

The implementation MUST use existing explicit-leaf metadata, not comparison with a default value. It MUST preserve the metadata while refreshing unowned leaves; a vector is one leaf.

Stock initialization MUST distinguish fresh material defaults from legacy overrides at the shared lookup boundary. The existing batch digest and legacy baselines retain missing-stock grain values, partial-array remainders, and unmatched halation values. Fresh initialization resolves class/schema defaults instead.

The process resolver MUST select this policy from baseline origin, not from the presence of later presets, feature profiles, or `--set` sources. Both policies MUST share the digest ordering and lookup helpers.

This behavior MUST NOT add a wire field or format version. Reset ownership and complete-state restoration follow [GUI workflow](../gui-workflow/spec.md#section-reset).

## Sections

A section is a named set of look-owned parameter paths. This table is authoritative for routing, and a section preset carries exactly its section's paths.

- A section MUST be a group of editable controls that a user can see and that can change the render. A parity-only, never-read, derived, or control-less parameter MUST NOT own a section.
- `film_render.glare` is not preset-owned because the pipeline never reads it and no control surface edits it. `film_render.grain.monochrome` is derived from the film profile (`broadcast_monochrome_layout`) and `film_render.grain.micro_sublayers` is a no-op (`sources::effects` classifies it `NoOp`).
- `enlarger.lens_blur` is zeroed by digest and applied by no upstream stage; the neutral enlarger filter anchors are database-calibration state rather than section-owned look controls. Print viewing glare remains in `print` through `print_render`; its [readout boundary](../film-chain/spec.md#scan-optics-and-print-viewing-glare) MUST NOT be inferred from that storage location.

- `film`: `film_render.density_curve_gamma`, `film_render.development_time`, `film_render.chemistry`. Primary module anchor: Film / Chemistry.
- `grain`: `film_render.grain`. Primary module anchor: Film / Grain.
- `halation`: `film_render.halation`. Primary module anchor: Film / Halation.
- `couplers`: `film_render.dir_couplers`. Primary module anchor: Film / Couplers.
- `base`: `film_render.base`. Primary module anchor: Film / Film base.
- `convert`: `film_render.convert`. Primary module anchor: Input and capture / Convert.
- `camera`: `camera.exposure_compensation_ev`, `camera.auto_exposure`, `camera.auto_exposure_method`, `camera.capture_illuminant`. Primary module anchor: Input and capture / Camera.
- `camera filter`: `camera_diffusion_filter`. Primary module anchor: Input and capture / Camera diffusion.
- `enlarger filter`: `enlarger.diffusion_filter`. Primary module anchor: Print / Enlarger diffusion.
- `enlarger`: `enlarger` without `enlarger.diffusion_filter`. Primary module anchor: Print / Enlarger.
- `scanner`: `scanner.scan_output`, `scanner.conversion_style`, `scanner.lens_blur`, `scanner.unsharp_mask`. Primary module anchor: Scan / Scanner.
- `cmyk ink style`: `cmyk`. Primary module anchor: Output / CMYK ink style.
- `print`: `print_render`. Primary module anchor: Print / Chemistry.
- `output mix`: `output_mix`. Primary module anchor: Output / Simulation blend.

The `convert` section includes the imported-capture acquisition light in `film_render.convert.scan_illuminant`. The fixed product readout illuminant is not preset-owned. Retired scanner lamp keys and the input-only `neutral` style alias follow the [lab scan migration contract](../film-chain/lab-scan/spec.md#point-wise-consumers-and-migration).

- The browser lists look presets only. A section preset MUST NOT appear in it, and the GUI MUST NOT offer a section menu, section capture, section management, or section import.
- A section payload is captured and applied by `core`, the CLI, and scoped photograph copying under [Photograph work](../gui-product-design/photo-work/spec.md#copy-adjustments). Stored section documents keep their payloads and file locations.
- The `enlarger` section includes the Print / Preflash controls; the `print` section includes print chemistry, print base, print density-curve controls, and Scan / Print viewing glare. Those placements MUST NOT split sections or change schemas or canonical payload scopes.
- A section preset MUST NOT change a profile reference or the workflow.
- Visible stage and module ownership follows [Stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership), not serialized group names. Module resets follow [Section reset](../gui-workflow/spec.md#section-reset); a section's wider payload MUST NOT expand a visible module's reset scope.

## Everyday flows

The flows a user actually runs MUST be short; every one below is two or three steps, and none of them needs a dialog that asks for a scope.

- Start a look: pick a browser row. Profiles, carried look controls, and the workflow default follow, and a render is scheduled.
- Change an applied look: edit any look control, then Update the user document, Save as under a new name, or Revert.
- Move a preset between machines: Export the document, drop the file into `<config_dir>/presets`, refresh. The same name then resolves for the GUI and the CLI.
- Reopen the app: controls come from the restored state, and the browser re-marks the remembered group and name; a missing document shows as missing.
- Script a run: `preset list` and `preset show` describe the same names and groups the browser shows, and `--preset` resolves them with the same rules.

## Names and groups

- A preset name is its identity. The document `name`, the file stem, and the selector MUST be the same string.
- The built-in documents form the `Built-in` group. The library directory forms the `My Presets` group.
- A name MUST be unique inside a group. Two built-in documents MUST NOT share a name, and two library documents MUST NOT share a name; the same name in both groups is valid.
- Name comparison MUST be case-insensitive, because some file systems resolve names without case.
- Browser rows MUST sort by name, case-insensitively, with a deterministic tie-break. A file-system scan order MUST never reach the UI, and one library MUST present one order.
- Browser search MUST implement the [browser search contract](spec.md#browser-search) as a transient presentation filter over discovered look presets. It MUST match a case-insensitive substring of the preset name or referenced film or print stock names or identifiers.
- It MUST retain group/name identities, deterministic ordering, library faults, and applied/modified/missing context. It MUST NOT change exact name resolution or write to the library, GUI-state files, or preset documents.
- A filtered browser with no matches MUST offer query clearing, not claim that the library is empty. An empty query MUST restore the unfiltered list and its existing order.
- A name MUST be 1 to 64 characters. It MUST NOT contain `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`, or control characters.
- It MUST NOT end with a period or with a carrier extension; it MUST NOT be a Windows reserved device name. These rules MUST hold on every platform, so a library can move between hosts.
- A library document MUST be named `<name>.toml` or `<name>.json`. A document whose `name` differs from its file stem MUST be a fault of that document.

## Discovery

- Discovery MUST NOT recurse. Only `.toml` and `.json` names MUST be documents; other entries MUST be ignored.
- An unreadable, unparsable, or invalid document MUST be a fault of that document. A per-document fault MUST NOT fail the scan or hide other entries.
- An unusable `My Presets` scope — a path that is not a readable directory — MUST register one fault entry for that scope and MUST retain every compiled `Built-in` entry. An explicit `built-in:` selector MUST resolve from the compiled documents without scanning `My Presets`.
- Name resolution MUST be exact; without a prefix it MUST try `My Presets`, then `Built-in`, and then fail. The optional `user:` and `built-in:` prefixes MUST select one group.
- A bare name that resolves in `My Presets` while a built-in document carries the same name MUST report the resolved group. A shadowed built-in preset MUST stay reachable through its prefix.
- An unknown name MUST NOT trigger a filesystem search or a name-similarity match.

## Writes

- A missing library directory is an empty library, not a fault. Discovery MUST NOT create it; the first write MUST create it.
- A write MUST validate the whole document before it replaces anything. A look write MUST resolve its profile references; a section write MUST NOT require profiles or an application workflow.
- A write MUST publish a complete temporary file atomically. A failed write MUST retain the preceding document and MUST NOT leave an empty document at a new name.
- Library mutations MUST serialize name checks with publication and removal across processes; a name has one owner across letter case and carriers. Creating or renaming to an existing name MUST fail without touching its document, including when writers compete for that name.
- A new document MUST default to TOML. A document whose explicit null leaves TOML cannot represent MUST be stored as JSON; rename and duplicate MUST preserve the source document's carrier.
- Rename MUST change the document name and the file name together; a failed rename MUST leave the preceding document reachable under its old name. A rename that only changes letter case MUST keep the document reachable on a case-insensitive filesystem.
- Import MUST NOT replace an existing name in `My Presets`. A conflict MUST be reported and MUST offer a renamed copy.
- Export MUST default its file name to the preset name with the carrier extension and MUST NOT write into the library directory unless the user picks it.
- Export into the library MUST use the document name as its file stem and MUST create a new name. It MUST NOT replace an existing library document; Update owns replacement.
- Deleting or updating one document MUST NOT modify another document or a built-in document.

## Boundaries

- The GUI state keeps its complete-state contract; the panel width, the expanded preference, and the last applied group and name belong to the versioned Rust extension and MUST NOT enter the Python-compatible sections. They MUST NOT change the meaning of a saved preset document.
- At a window width of 1250 logical points or less, the browser MUST automatically collapse to a 48-point rail.
- A manual reopen at that width MUST remain session-local and MUST NOT rewrite the serialized expanded or width preference.
- Returning to a wider window MUST restore the stored expanded and width preferences.
- Modified detection compares the active look controls and the film and print references with the applied document. It MUST use `LookParameters::matches` to respect an omitted legacy camera group under the [preset file format rules](spec.md#file-formats); it MUST NOT compare GUI state or a prepared runtime parameter tree.
- Look capture MUST collect existing material controls through their existing sections and output mix through its own section; it MUST never collect an `IoParams` snapshot or output gamut settings. Schema migration MUST follow the [preset file-format contract](spec.md#file-formats): older whole looks gain full output mix, while older sections retain sparse omission.
- Capture, apply, and migrated export MUST NOT expand `io` ownership; preparation MUST preserve existing material capability, scan-inverse, and calibration semantics under [Output mix and material controls](../output-mix/spec.md). Unshipped response groups and the Cyan-shift field are unknown fields, including in draft version 3 documents; neither capture nor migration MAY materialize them.
- Built-in documents MUST stay read-only at run time. A user edit MUST produce a user document or no change.
- The `My Presets` group header MUST show the resolved library directory path, MUST offer to reveal it, and MUST offer to refresh discovery, so a document placed while the GUI runs becomes visible without a restart. Only user documents have a path.
- Renaming the applied user preset MUST follow the marker to the new name. Deleting it MUST leave the look controls untouched and MUST show the remembered name as missing.
- Restoring GUI state MUST restore the stored controls and MUST NOT re-apply the remembered preset document. The applied marker is recomputed against the restored controls.
- Reference thumbnails and browsing preferences follow [Preset discovery aids](spec.md#preset-discovery-aids); they MUST NOT change preset discovery, identity, resolution, or document schemas. Live current-photograph thumbnails, user-selectable sorting, collections, per-scope preset memory, inheritance, remote distribution, and scanning directories other than the library directory remain outside this boundary.
