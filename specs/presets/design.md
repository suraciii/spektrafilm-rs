# Preset design

- Vocabularies: a **preset** is a named parameter document — the user-facing one is a library document (`Built-in` compiled from the crate's `assets/presets`, `My Presets` read from `<config_dir>/presets`), while `data/presets/` keeps the upstream scope, the per-stock grain and coupler tables the runtime compiles in. **Section** remains the name of a GUI control group and of the path set a section menu captures; it is not a document kind. The terms *look preset*, *section preset*, *kind*, and *looks directory* must not appear in a document, an error, an API name, or a spec.
- *Calibration* stays reserved for the Convert device-correction matrix and its worker. Names that mirror the pinned upstream (`apply_grain_preset`, `HALATION_PRESETS`, `v2_profile` and its values) keep their upstream spelling for parity traceability.
- The preset document, strict validation, resolution, capture, the library directory policy, discovery, name conflict rejection, and atomic writes belong to `core`. The Rust document type is `presets::Preset`, named after its module like `profile::Profile`. Its payload type stores the sparse map from canonical parameter paths to values the document supplies, so one payload type serves a whole look, a section, and a single leaf.
- Built-in preset documents are repository files in `crates/spektrafilm-core/assets/presets`. The build must compile them in, so built-in listing and name resolution must not require a data directory, and packaged data directories must not carry copies.
- A document is valid when every path it carries belongs to the [parameter set](#parameter-set), its values pass the same validators as a sparse parameter source, and it carries at least one path. A document must not declare a scope and must not be required to be complete.
- The payload must follow the parameter-file leaf discipline: only supplied leaves are edited, unknown fields and duplicate keys fail, and omitted values must not be filled from defaults. Applying a document edits exactly its supplied paths and registers those values as explicit, so neither stock initialization nor database calibration rewrites them, and it must not clear explicit values outside its supplied paths.
- Writes, atomicity, rename, delete, duplicate, import, and export must treat every document through one code path. The resolver must reuse the parameter-file leaf-merge implementation for presets instead of adding a second merge.
- The library directory is `<config_dir>/presets`. The configuration directory policy is shared by the GUI and the CLI. `SPEKTRAFILM_CONFIG_DIR` must override it; there is no separate preset directory override. The library directory must not require a data directory.

## Parameter set

Every runtime parameter leaf carries exactly one class, and the class together with the capture attribute defines the one set every input surface offers. Both are properties of the leaf in the parameter metadata, next to its type, bounds, default, and unit, so one authoritative table serves the CLI, the GUI, the API, and preset validation.

A whole-look capture must write every leaf whose capture attribute is true, so a saved document determines its look by itself: a later change to a default, to a stock-derived seed, or to a fitted profile value must not change what the document reproduces. A sparse document stays legal for a hand-authored or built-in look, and Update rewrites exactly the paths a stored document supplies, so a short document stays short. The leaves with a false capture attribute are the delivery and environment values, which is why a preset never moves the caller's crop, resize, rotation, input declaration, or export settings.

GUI state and a render recipe are snapshots rather than parameter surfaces: they keep carrying the value of a `debug` leaf, and only the settable surfaces gate it.

| Class | Meaning | Settable by |
|---|---|---|
| `parameter` | A user control edits it | the GUI, the CLI, a preset, and the API |
| `debug` | A diagnostics, parity, or verification switch that no control edits | the CLI, only under the debug mode; never a preset |
| `internal` | No control edits it, it is derived, or the pipeline never reads it | nowhere |

`capture` is a second attribute of a `parameter` leaf: a whole-look capture writes the leaves with `capture=true`, which are the values a user tunes while simulating. A `capture=false` leaf is a source, geometry, delivery, viewer, or environment value that a document may still carry explicitly.

The parameter metadata must expose both attributes through `spektrafilm describe` and `spektrafilm describe --field`, so the settable set is discoverable without reading a preset by trial, and `describe --module` must cover every group.

The leaf table lives in [preset parameters](parameters.md). Counts from the current tree:

| Class | Leaves | of which `capture=true` |
|---|---|---|
| `parameter` | 198 | 182 |
| `debug` | 20 | 0 |

The twenty debug leaves are the measured neutral axes, the parity-only glare fields, the spectral grid and LUT and performance switches, the preview switch, the calibration-policy switch, the tap hooks, and the diagnostics switches. They keep a command-line path only under the debug mode, because a value the user cannot edit must not become an ordinary parameter; three taking-side style values instead need a control and stay `parameter`. [preset parameters](parameters.md) lists them with their evidence.

The table must be regenerated from the metadata rather than maintained by hand, so a new leaf cannot enter the runtime without a class, a control decision, and a capture attribute.

## Migration

Schema version 4 changes the document from a scoped payload with profile references into a sparse parameter payload. A reader must migrate the accepted versions explicitly and must apply the same vocabulary, profile, and value validation to the migrated result.

| Version 3 | Version 4 | Rule |
|---|---|---|
| `schema_version = 3` | `4` | rewritten on export; a migrated document must not keep the old version |
| `name`, `author`, `description` | unchanged | identity and metadata are not parameters |
| `film_profile.stock` | `film.stock` | the stock name becomes a vocabulary path |
| `print_profile.stock` | `print.stock` | the stock name becomes a vocabulary path |
| `film_profile.version`, `film_profile.sha256-<hex>`, `print_profile.version`, `print_profile.sha256-<hex>` | dropped | profile identity is not preset content; the resolved stock is |
| `parameters` (look groups, all required) | `parameters` (sparse) | paths are unchanged; a group the document omitted stays absent, and a whole look is no longer required to be complete |
| `parameters.camera` (atomic optional group) | `parameters.camera.*` | the three fields become ordinary sparse paths; supplying a subset is valid |
| section document (exactly one section) | unchanged content | its paths already belong to the vocabulary; the document simply carries them |
| `output_mix.amount` | unchanged | version 3 already carries it |
| `provenance` | `implementation_version` and `model_version` kept, `scanner` dropped | provenance is optional metadata; the scanner snapshot repeats the `scanner.*` paths |
| version 1 or 2 | version 4 | the existing version 1 and version 2 rules apply first, then this table |

A migrated document that carries a path outside the parameter set must be rejected with the offending path named, so a legacy document never gains a class it cannot have. A migrated section must preserve sparse omission and must not acquire leaves. Migration must not capture or replace caller `io` settings, and it must not invent a profile identity.

## Profile identity

A version 4 document carries no profile identity. The stock name is the identity a preset needs: `film.stock` and `print.stock` name the material, and the profile files that implement them are versioned data of the data directory. A byte or version pin would turn a later improvement of a profile into a failure of a document the user cannot repair from any control, and a built-in document would need its pins regenerated on every fitted-value change. A reader must therefore drop `version` and the `sha256-<hex>` content identity on migration and must reject either as an unknown field in a version 4 document.

`provenance` stays optional metadata. A capture writes the implementation and model versions; no reader requires them and no provenance field is a parameter or a validation gate. The scanner snapshot is removed because it repeats the `scanner.*` values the document already carries.

## Everyday flows

The flows a user actually runs must be short; every one below is two or three steps, and none of them needs a dialog that asks for a scope.

- Start a look: pick a browser row. The supplied film and print stocks, the carried look controls, and the workflow default follow, and a render is scheduled.
- Keep a section: adjust that section, open its menu (it reads `custom`), Save as, name it. The file lands in the library and the menu reads that name; the next image applies it in one click after the look.
- Change an applied look: edit any control the document supplies, then Update the user document, Save as under a new name, or Revert.
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
- A write must validate the whole document before it replaces anything. A write that supplies `film.stock` or `print.stock` must resolve those profiles. A write that supplies neither must not require profiles or an application workflow.
- A write must publish a complete temporary file atomically. A failed write must retain the preceding document and must not leave an empty document at a new name.
- Library mutations must serialize name checks with publication and removal across processes. A name has one owner across letter case and carriers. Creating or renaming to an existing name must fail without touching its document, including when writers compete for that name.
- A new document must default to TOML. A document whose explicit null leaves TOML cannot represent must be stored as JSON; rename and duplicate must preserve the source document's carrier.
- Rename must change the document name and the file name together. A failed rename must leave the preceding document reachable under its old name. A rename that only changes letter case must keep the document reachable on a case-insensitive filesystem.
- Import must not replace an existing name in `My Presets`. A conflict must be reported and must offer a renamed copy.
- Export must default its file name to the preset name with the carrier extension and must not write into the library directory unless the user picks it.
- Export into the library must use the document name as its file stem and must create a new name. It must not replace an existing library document; Update owns replacement.
- Deleting or updating one document must not modify another document or a built-in document.
- Update must rewrite exactly the paths the stored document supplies, with their active values. Save as must write every leaf whose capture attribute is true, or the paths of the capturing section.

## Boundaries

- The GUI state keeps its complete-state contract. The panel width, the expanded state, and the last applied group and name belong to the versioned Rust extension and must not enter the Python-compatible sections. They must not change the meaning of a saved preset document.
- Modified detection compares the active value of every path the applied document supplies with the value in that document, including `film.stock` and `print.stock`. It must not compare a path the document omits, GUI state, or a prepared runtime parameter tree. A document that supplies a subset must show as modified only for that subset.
- A whole-look capture must collect the leaves whose capture attribute is true through the existing control sections and must never collect an `IoParams` snapshot, output gamut settings, debug switches, or a neutral axis. A section capture collects that section's paths only.
- Built-in documents must stay read-only at run time. A user edit must produce a user document or no change. The `My Presets` group header must show the resolved library directory path, must offer to reveal it, and must offer to refresh discovery, so a document placed while the GUI runs becomes visible without a restart. Only user documents have a path.
- Renaming the applied user preset must follow the marker to the new name. Deleting it must leave the controls untouched and must show the remembered name as missing.
- Restoring GUI state must restore the stored controls and must not re-apply the remembered preset document. The applied marker is recomputed against the restored controls.
- Live thumbnails, favorites, search, sorting options, collections, per-scope preset memory, inheritance, remote distribution, and scanning directories other than the library directory are outside this boundary.
