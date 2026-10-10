# Preset design

- Vocabularies: a **preset** is a named parameter set — the user-facing one is a library document (`Built-in` compiled from the crate's `assets/presets`, `My Presets` read from `<config_dir>/presets`), while `data/presets/` keeps the upstream scope, the per-stock grain and coupler tables the runtime compiles in. A **look** is the content a preset owns: the look controls and their values. A **look preset** is the whole-look kind and a **section preset** the one-section kind; the terms *look document* and a `looks` directory must not appear.
- *Calibration* stays reserved for the Convert device-correction matrix and its worker. Names that mirror the pinned upstream (`apply_grain_preset`, `HALATION_PRESETS`, `v2_profile` and its values) keep their upstream spelling for parity traceability.
- The preset document, strict validation, resolution, capture, the library directory policy, discovery, name conflict rejection, and atomic writes belong to `core`. The Rust document type is `presets::Preset`, named after its module like `profile::Profile`; `LookParameters` remains the look payload.
- Built-in preset documents are repository files in `crates/spektrafilm-core/assets/presets`. The build must compile them in, so built-in listing and name resolution must not require a data directory, and packaged data directories must not carry copies.
- A document is a **look preset** (both profile references and every look-owned group) or a **section preset** (exactly one section, no profile reference). The kind is derived from the carried scope, never declared by a field, and a built-in document may be either kind. A document that carries neither a whole look nor exactly one section must be rejected.
- A section preset must pass the same value validators as a look preset for the paths it carries, and its payload must follow the parameter-file leaf discipline: only supplied leaves are edited, unknown fields and duplicate keys fail, and omitted values must not be filled from defaults. Applying it edits exactly its section and registers those values as explicit, so neither stock initialization nor database calibration rewrites them, and it must not clear explicit values outside its section.
- Writes, atomicity, rename, delete, duplicate, import, and export must treat both kinds through one code path. The resolver must reuse the parameter-file leaf-merge implementation for section presets instead of adding a second merge.
- The library directory is `<config_dir>/presets`. The configuration directory policy is shared by the GUI and the CLI. `SPEKTRAFILM_CONFIG_DIR` must override it; there is no separate preset directory override. The library directory must not require a data directory.

## Sections

A section is a named set of look-owned parameter paths. This table is authoritative for routing, and a section preset carries exactly its section's paths.

- `film`: `film_render.density_curve_gamma`, `film_render.development_time`, `film_render.chemistry`
- `grain`: `film_render.grain`
- `halation`: `film_render.halation`
- `couplers`: `film_render.dir_couplers`
- `glare`: `film_render.glare`
- `base`: `film_render.base`
- `convert`: `film_render.convert`
- `camera filter`: `camera_diffusion_filter`
- `enlarger filter`: `enlarger.diffusion_filter`
- `enlarger`: `enlarger` without `enlarger.diffusion_filter`
- `scanner`: `scanner`
- `print`: `print_render`

- The browser lists look presets only. A section preset must not appear in it and must not be applicable from it.
- A section preset loads from the menu of its own section. The menu must show the preset whose carried controls equal the section's current controls, or `custom`. The comparison must use the carried editable controls and never derived or prepared values, and equal matches must resolve deterministically by name order.
- A section menu must not appear while the library carries no preset for that section, and the menu must not repeat a control that already exists inside the section.
- The section menu owns load, Save as, Rename, Duplicate, Delete, Export, and Show file for its own documents; the browser keeps the same operations for look presets. Both must call the shared library implementation.
- A section preset must not change a profile reference or the workflow.

## Names and groups

- A preset name is its identity. The document `name`, the file stem, and the selector must be the same string.
- The built-in documents form the `Built-in` group. The library directory forms the `My Presets` group.
- A name must be unique inside a group. Two built-in documents must not share a name, and two library documents must not share a name. The same name in both groups is valid.
- Name comparison must be case-insensitive, because some file systems resolve names without case.
- A name must be 1 to 64 characters. It must not contain `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`, or control characters. It must not end with a period or with a carrier extension. It must not be a Windows reserved device name. These rules must hold on every platform, so a library can move between hosts.
- A library document must be named `<name>.toml` or `<name>.json`. A document whose `name` differs from its file stem must be a fault of that document.

## Discovery

- Discovery must not recurse. Only `.toml` and `.json` names must be documents; other entries must be ignored.
- An unreadable, unparsable, or invalid document must be a fault of that document. A per-document fault must not fail the scan or hide other entries.
- Name resolution must be exact. Without a prefix it must try `My Presets`, then `Built-in`, and then fail. The optional `user:` and `built-in:` prefixes must select one group.
- A bare name that resolves in `My Presets` while a built-in document carries the same name must report the resolved group. A shadowed built-in preset must stay reachable through its prefix.
- An unknown name must not trigger a filesystem search or a name-similarity match.

## Writes

- A missing library directory is an empty library, not a fault. Discovery must not create it; the first write must create it.
- A write must validate the whole document and resolve it before it replaces anything.
- A write must be atomic: write a temporary file in the library directory, then rename it. A failed write must retain the preceding document.
- Rename must change the document name and the file name together. A failed rename must leave the preceding document reachable under its old name.
- Import must not replace an existing name in `My Presets`. A conflict must be reported and must offer a renamed copy.
- Deleting or updating one document must not modify another document or a built-in document.

## Boundaries

- The GUI state keeps its complete-state contract. The panel width, the expanded state, and the last applied group and name belong to the versioned Rust extension and must not enter the Python-compatible sections. They must not change the meaning of a saved preset document.
- Modified detection compares the captured look controls and the film and print references with the applied document. It must not compare GUI state or a prepared runtime parameter tree.
- Built-in documents must stay read-only at run time. A user edit must produce a user document or no change. The `My Presets` group header must offer to reveal the library directory, because only user documents have a path.
- Live thumbnails, favorites, search, sorting options, collections, per-scope preset memory, inheritance, remote distribution, and scanning directories other than the library directory are outside this boundary.
