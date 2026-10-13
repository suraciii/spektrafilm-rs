# Preset design

- Vocabularies: a **preset** is a named parameter set — the user-facing one is a library document (`Built-in` compiled from the crate's `assets/presets`, `My Presets` read from `<config_dir>/presets`), while `data/presets/` keeps the upstream scope, the per-stock grain and coupler tables the runtime compiles in. A **look** is the content a preset owns: the look controls and their values. A **look preset** is the whole-look kind and a **section preset** the one-section kind; the terms *look document* and a `looks` directory must not appear.
- *Calibration* stays reserved for the Convert device-correction matrix and its worker. Names that mirror the pinned upstream (`apply_grain_preset`, `HALATION_PRESETS`, `v2_profile` and its values) keep their upstream spelling for parity traceability.
- The preset document, strict validation, resolution, capture, the library directory policy, discovery, name conflict rejection, and atomic writes belong to `core`. The Rust document type is `presets::Preset`, named after its module like `profile::Profile`; `LookParameters` remains the look payload. Its `settings` group (`LookSettingsParams`) carries only the look-relevant processing settings (hanatos2025 adaptation window/surface, spectral Gaussian blur, CAT16) — not the wavelength grid, LUT wiring, preview limits, or performance toggles.
- Built-in preset documents are repository files in `crates/spektrafilm-core/assets/presets`. The build must compile them in, so built-in listing and name resolution must not require a data directory, and packaged data directories must not carry copies.
- A document is a **look preset** (both profile references and every required look-owned group, with the optional camera group defined by the [preset file format rules](spec.md#file-formats)) or a **section preset** (exactly one section, no profile reference). The kind is derived from the carried scope, never declared by a field, and a built-in document may be either kind. A document that carries neither a whole look nor exactly one section must be rejected.
- A section preset must pass the same value validators as a look preset for the paths it carries, and its payload must follow the parameter-file leaf discipline: only supplied leaves are edited, unknown fields and duplicate keys fail, and omitted values must not be filled from defaults. Applying it edits exactly its section and registers those values as explicit, so neither stock initialization nor database calibration rewrites them, and it must not clear explicit values outside its section.
- Writes, atomicity, rename, delete, duplicate, import, and export must treat both kinds through one code path. The resolver must reuse the parameter-file leaf-merge implementation for section presets instead of adding a second merge.
- The library directory is `<config_dir>/presets`. The configuration directory policy is shared by the GUI and the CLI. `SPEKTRAFILM_CONFIG_DIR` must override it; there is no separate preset directory override. The library directory must not require a data directory.
- Enlarger filtration is two fields, not one. A **neutral filter** (`c/m/y_filter_neutral`) is the CC baseline a dichroic axis sits at; a **filter shift** (`m/y_filter_shift`) is a CC offset from that baseline. The effective CC value is `neutral + shift`. The two words must not be used interchangeably in a document, a control label, or a diagnostic.
- The neutral baseline belongs to the print profile and is built in. The database `data/filters/neutral_print_filters.json` is keyed by (print stock, illuminant, film stock) and each entry is the filtration that prints a neutral ramp neutral on that paper, so a value fitted on one paper is not portable to another. The look payload carries the shifts only; the neutral fields are resolved per paper. This keeps one authority per fact: the paper owns its baseline, the look owns its offsets.
- Because the neutral fields are not look payload, `LookParameters` must not serialize them and `neutral_print_filters_protected` must not be set by preset application. The database lookup then supplies the baseline. Explicit user edits through a sparse parameter source still set protection for the axis they name, and `settings.neutral_print_filters_from_database` keeps the user's on/off choice.
- A neutral value that reaches the `enlarger` group is a schema rejection, not a silent drop, so no document can carry a second paper's baseline into a new one. Migration removes a carried value.

### Print filtration

A neutral filter value is built in, never preset content. The database `data/filters/neutral_print_filters.json` is keyed by (print stock, illuminant, film stock) under the `TH-KG3` illuminant, and a paper it does not describe leaves the axes at the runtime default `C0 M65 Y55`.

- A look carries the M and Y shifts and nothing else in the neutral set. `cinestill_800t_cs41_approx` once carried its fitted `C0 M23.48 Y27.97` inside `enlarger` and marked the axes protected, because the database had no entry for that film then. That fit is the look's own, solved on Kodak Portra Endura, and it traveled to every paper.
- A paper's own entry supersedes any fitted value, because it is that paper's measurement. On Kodak Portra Endura the entry for that film is `C0 M29.78 Y34.93`; on Fujifilm Crystal Archive Type II the same film needs `C0 M43.63 Y53.21`, so a carried Endura fit prints with a red cast there.
- Resolution order per axis is: an explicit parameter source, then the database entry, then the runtime default. Each step must be reported so a caller can see which one produced the effective value.
- No preset carries the three neutral axes. A look excludes them from `enlarger` and the `enlarger` section does not own them, so a parameter source remains the only way a user overrides a paper's baseline.
- The control surface already matches this split: the enlarger section exposes the M and Y shifts and no neutral control, so a preset cannot change a control the user cannot change.


## Sections

A section is a named set of look-owned parameter paths. This table is authoritative for routing, and a section preset carries exactly its section's paths.

A section must be a group of editable controls that a user can see and that can change the render. A parity-only, never-read, derived, or control-less parameter must not own a section: `film_render.glare` stays a document field for upstream parity but is not a section, because the pipeline never reads it and no control surface edits it. Print viewing glare remains in `print` through `print_render`; its [readout boundary](../film-chain/spec.md#scan-optics-and-print-viewing-glare) must not be inferred from that storage location.

- `film`: `film_render.density_curve_gamma`, `film_render.development_time`, `film_render.chemistry`
- `grain`: `film_render.grain`
- `halation`: `film_render.halation`
- `couplers`: `film_render.dir_couplers`
- `base`: `film_render.base`
- `convert`: `film_render.convert`
- `camera`: `camera.exposure_compensation_ev`, `camera.auto_exposure`, `camera.auto_exposure_method`
- `camera filter`: `camera_diffusion_filter`
- `enlarger filter`: `enlarger.diffusion_filter`
- `enlarger`: `enlarger` without `enlarger.diffusion_filter` and without the three neutral filter baselines, which the [print filtration contract](spec.md#print-filtration) resolves per paper
- `scanner`: `scanner`
- `print`: `print_render`
- `output mix`: `output_mix`

- The browser lists look presets only. A section preset must not appear in it and must not be applicable from it.
- A section preset loads from the menu of its own section. The menu must show the preset whose carried controls equal the section's current controls, or `custom`. The comparison must use the carried editable controls and never derived or prepared values, and equal matches must resolve deterministically by name order.
- Every section in this table carries its menu, because Save as lives there. A section that spans several visible headers (for example `print`, which covers print chemistry, print base, print glare, and the print density-curve morph) carries one menu on its primary header, so a menu belongs to a section and never to a visible label. A menu whose list is empty must offer Save as for the section's current controls and must state that a preset file can also be placed in the library. The menu must not repeat a control that already exists inside the section.
- The section menu owns load, Save as, Rename, Duplicate, Delete, Export, and Show file for its own documents; the browser keeps the same operations for look presets. Both must call the shared library implementation.
- A section preset must not change a profile reference or the workflow.

## Everyday flows

The flows a user actually runs must be short; every one below is two or three steps, and none of them needs a dialog that asks for a scope.

- Start a look: pick a browser row. Profiles, carried look controls, and the workflow default follow, and a render is scheduled.
- Keep a section: adjust that section, open its menu (it reads `custom`), Save as, name it. The file lands in the library and the menu reads that name; the next image applies it in one click after the look.
- Change an applied look: edit any look control, then Update the user document, Save as under a name, or Revert.
- Change the paper of an applied look: pick a new print profile. The neutral baseline re-derives from the new paper's database entry and the M/Y shifts stay. No paper's neutral value is copied onto another paper, and the look does not read modified because its own controls did not change.
- See why a paper changed: the dry-run report names the source of each baseline — `set`, `database`, or `default` — so the effective filtration is visible before a render.
- Move a preset between machines: Export either kind, drop the file into `<config_dir>/presets`, refresh. The same name then resolves for the GUI and the CLI.
- Reopen the app: controls come from the restored state, and the browser re-marks the remembered group and name; a missing document shows as missing.
- Script a run: `preset list` and `preset show` describe the same names and groups the browser shows, and `--preset` resolves them with the same rules.

## Names and groups

- A preset name is its identity. The document `name`, the file stem, and the selector must be the same string.
- The built-in documents form the `Built-in` group. The library directory forms the `My Presets` group.
- A name must be unique inside a group. Two built-in documents must not share a name, and two library documents must not share a name. The same name in both groups is valid.
- Name comparison must be case-insensitive, because some file systems resolve names without case.
- Browser rows and section menus must sort by name, case-insensitively, with a deterministic tie-break. A file-system scan order must never reach the UI, and one library must present one order.
- A name must be 1 to 64 characters. It must not contain `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`, or control characters. It must not end with a period or with a carrier extension. It must not be a Windows reserved device name. These rules must hold on every platform, so a library can move between hosts.
- A library document must be named `<name>.toml` or `<name>.json`. A document whose `name` differs from its file stem must be a fault of that document.

## Discovery

- Discovery must not recurse. Only `.toml` and `.json` names must be documents; other entries must be ignored.
- An unreadable, unparsable, or invalid document must be a fault of that document. A per-document fault must not fail the scan or hide other entries.
- An unusable `My Presets` scope — a path that is not a readable directory — must register one fault entry for that scope and must retain every compiled `Built-in` entry. An explicit `built-in:` selector must resolve from the compiled documents without scanning `My Presets`.
- Name resolution must be exact. Without a prefix it must try `My Presets`, then `Built-in`, and then fail. The optional `user:` and `built-in:` prefixes must select one group.
- A bare name that resolves in `My Presets` while a built-in document carries the same name must report the resolved group. A shadowed built-in preset must stay reachable through its prefix.
- An unknown name must not trigger a filesystem search or a name-similarity match.

## Writes

- A missing library directory is an empty library, not a fault. Discovery must not create it; the first write must create it.
- A write must validate the whole document before it replaces anything. A look write must resolve its profile references. A section write must not require profiles or an application workflow.
- A write must publish a complete temporary file atomically. A failed write must retain the preceding document and must not leave an empty document at a new name.
- Library mutations must serialize name checks with publication and removal across processes. A name has one owner across letter case and carriers. Creating or renaming to an existing name must fail without touching its document, including when writers compete for that name.
- A new document must default to TOML. A document whose explicit null leaves TOML cannot represent must be stored as JSON; rename and duplicate must preserve the source document's carrier.
- Rename must change the document name and the file name together. A failed rename must leave the preceding document reachable under its old name. A rename that only changes letter case must keep the document reachable on a case-insensitive filesystem.
- Import must not replace an existing name in `My Presets`. A conflict must be reported and must offer a renamed copy.
- Export must default its file name to the preset name with the carrier extension and must not write into the library directory unless the user picks it.
- Export into the library must use the document name as its file stem and must create a new name. It must not replace an existing library document; Update owns replacement.
- Deleting or updating one document must not modify another document or a built-in document.

## Boundaries

- The GUI state keeps its complete-state contract. The panel width, the expanded state, and the last applied group and name belong to the versioned Rust extension and must not enter the Python-compatible sections. They must not change the meaning of a saved preset document.
- Modified detection compares the captured look controls and the film and print references with the applied document, and never the resolved neutral baselines or GUI state. A resolved baseline is derived state, not look state, so it cannot make an applied preset read modified. A print-profile change therefore moves the resolved baseline while the applied preset still matches on its own controls. It must use `LookParameters::matches` to respect an omitted legacy camera group under the [preset file format rules](spec.md#file-formats). It must not compare GUI state or a prepared runtime parameter tree.
- Look capture must collect the M/Y shifts through the `enlarger` section and never a resolved neutral baseline.
- Look capture must collect existing material controls through their existing sections and output mix through its own section. It must never collect an `IoParams` snapshot or output gamut settings. Schema migration must follow the [preset file-format contract](spec.md#file-formats): older whole looks gain full output mix, while older sections retain sparse omission. Capture, apply, and migrated export must not expand `io` ownership. Preparation must preserve existing material capability, scan-inverse, and calibration semantics under [Output mix and material controls](../output-mix/spec.md). Unshipped response groups and the Cyan-shift field are unknown fields, including in draft version 3 documents; neither capture nor migration may materialize them.
- Built-in documents must stay read-only at run time. A user edit must produce a user document or no change. The `My Presets` group header must show the resolved library directory path, must offer to reveal it, and must offer to refresh discovery, so a document placed while the GUI runs becomes visible without a restart. Only user documents have a path.
- Renaming the applied user preset must follow the marker to the new name. Deleting it must leave the look controls untouched and must show the remembered name as missing.
- Restoring GUI state must restore the stored controls and must not re-apply the remembered preset document. The applied marker is recomputed against the restored controls.
- Live thumbnails, favorites, search, sorting options, collections, per-scope preset memory, inheritance, remote distribution, and scanning directories other than the library directory are outside this boundary.
