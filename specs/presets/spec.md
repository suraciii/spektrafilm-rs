# Presets

## Contract

A **preset** is a named, reusable set of film and print profile selections and visual simulation controls. Reusable combinations of controls that are already preset-owned belong in the existing look or section preset. A preset is not a complete GUI state, an input image, a viewer state, an export configuration, a LUT, a calibration profile, or a workflow selection. A preset is either **built-in** or **user-owned**.

A preset document is a **look preset** or a **section preset**. A look preset carries both profile references and every required look-owned group. Its optional camera group follows the [file format rules](#file-formats). A section preset carries exactly one section of the look and no profile reference; a section is a named set of look-owned parameter paths, defined by the [preset design](design.md). The kind is derived from the carried scope, never declared by a field, and a document that carries neither a whole look nor exactly one section must be rejected.

Built-in look presets must enable the selected film profile's Hanatos2025 adaptation surface. The surface coefficients belong to the film profile. The user may turn the surface off after applying the preset.

A preset **name** is its identity: the document `name`, the document file stem, and the selector used by the GUI and the CLI. A name must be unique inside its group and may repeat across groups. The groups are `Built-in` and `My Presets`. The [preset design](design.md) owns the name rules, file locations, discovery, conflicts, and writes.

The GUI provides a Presets browser for look presets in a collapsible panel on the left of the window, and one preset menu per section for section presets. The browser shows the two groups. Each row shows the preset name and its film and print references. The active row is marked. A modified marker appears while the active look controls differ from the applied document. The inspector must not repeat the browser list. User presets must not be implemented by reusing complete `GuiState`.

A section preset must appear only in the menu of its own section, never in the browser. Every section of the section table must carry one menu at its named primary module anchor in the [preset design](design.md#sections). A section that spans modules or stages must not gain duplicate menus or split payloads. Its menu must identify the full carried scope, not just the controls under its anchor. Save as lives in that menu: an empty list must offer to save the section's current controls and must state that a preset file can also be placed in the library. A section menu must report the issues of the documents it would carry. A section menu must show the name of the section preset whose controls match the section and `custom` otherwise. That match must be derived from the section's controls and must not be stored as separate state.

Browser rows and section menus must be ordered by name, case-insensitively, with a deterministic tie-break, so one library always presents one order. User-selectable sorting is outside this contract.

### Browser search

The browser must offer a transient search field. It must filter look presets by a case-insensitive substring match against the preset name or either referenced film or print stock's name or identifier. An empty query must show all look presets. Search must not change discovery, exact name resolution, preset identity, group membership, or the deterministic order within each group.

Filtering must preserve the `Built-in` and `My Presets` groups and their identities. The applied group and name, modified state, and missing-document state must remain visible as context even when the applied row does not match. Search must not apply a preset or clear that context. Library faults must remain visible and must not be mistaken for unmatched presets.

The browser must distinguish a library with no look presets from a search with no matches. The no-match state must offer to clear the query. The empty-library state must retain the save guidance. The query must not persist across restarts or enter GUI-state files or preset documents. This browser search is separate from [control search](../gui-workflow/spec.md#control-search).

### Preset discovery aids

The browser must offer Favorites and Recently used filters within the existing library. Both must retain group identity, deterministic row ordering, and the applied or missing context under [Browser search](#browser-search). Recent membership must record successful look application, not browsing or failed application. It must contain the latest ten distinct applied look identities.

Favorites and recent membership must persist as application browsing preferences, separately from presets and photographic state. Toggling a favorite or changing a filter must not apply a look or render a photograph. Application-owned rename must carry these references to the new name; missing entries must not bind to another group with the same name.

Built-in looks must offer thumbnails rendered from one fixed reference photograph. Each thumbnail must identify that reference and must not imply that it previews the active photograph. It must represent its preset and material versions; a missing or outdated image must show unavailable rather than a misleading substitute. User looks may have no reference image and must remain fully usable.

Reference images must retain visible preset and material names. Opening the browser must not start a render of the current photograph for every preset. Reference-image rights must permit distribution with the application.

## User library

A user preset is a valid preset document in the preset library directory. Placing such a document in that directory makes it available to the GUI browser and to preset name resolution. The browser and the CLI must use the same discovery and name rules.

The GUI must support these library operations for look presets:

- **Save as**: capture the active look controls as a new look preset under a name the user edits.
- **Update**: replace the stored document of the active user preset with the active look controls.
- **Revert**: restore the look controls of the applied document and clear the modified marker.
- **Rename**: rename the document file and its name together, and preserve its look values.
- **Duplicate**: copy a built-in or user preset into `My Presets` under a new name.
- **Delete**: remove a user preset document after confirmation.
- **Import**: copy an external document into `My Presets` as a new preset. An existing name in `My Presets` must not be replaced; a conflicting import must offer a renamed copy.
- **Export**: write the selected document to a chosen path.
- **Show file**: reveal the document path of a user preset. A built-in preset has no path, so this action must be unavailable for it.

A section menu must offer the same operations for the section presets it lists: **Save as** captures that section's controls, and Rename, Duplicate, Delete, Export, and Show file act on that section's documents. Both kinds must share one discovery, write, and validation implementation.

The remembered applied document is always a look preset; a section application is an edit and never becomes the applied preset. The GUI writes look presets through Save as and Update, and a section preset is authored as a file or through a section menu until an arbitrary scope-capture action exists.

A name that exists in both groups must remain reachable in both. Editing look controls after an application must mark the active preset modified and must not rewrite any document. Built-in presets must remain immutable.

The browser must list, with its path, every document that cannot be read, parsed, or accepted. An issue must not hide the remaining library. An empty library must state how to save the current look.

The browser must remember the applied preset group and name across restarts. A remembered preset whose document is absent must be shown as missing. The modified state must not persist across restarts. Restoring GUI state must restore its stored controls and must not re-apply the remembered preset document.

Ordinary startup must restore the browser metadata independently of the configured startup controls. An explicitly loaded GUI state must supply its own browser metadata and must not inherit the remembered selection. Remembering a browser selection must not save current controls as the startup default.

Renaming or deleting the applied user preset must update the marker: a rename follows the new name, and a delete leaves the look controls untouched and shows the remembered name as missing.

Browser rows must show the preset name, the film reference, and the print reference. [Preset discovery aids](#preset-discovery-aids) owns reference-image requirements and browsing filters. Live current-photograph thumbnails, user-selectable sorting, collections, per-scope preset memory, and arbitrary scope capture are outside this contract. Multi-photograph application follows [Photograph work](../gui-product-design/photo-work/spec.md#copy-adjustments).

### Apply

- Selecting a preset validates the document, or the section it carries, before changing active controls.
- A look preset identifies both film and print profiles. The resolver validates film=`support=film, stage=filming` and print=`support=paper, stage=printing`.
- A preset does not store or directly apply a workflow. A look preset selects the corresponding workflow default: negative film selects `input > film > print > scan`, and positive film selects `input > film > scan`. The user may then choose another supported workflow manually. A section preset carries no film reference and must not change the workflow.
- Applying a look preset replaces all carried look-owned controls together, including the controls of every carried section, so a section preset must be applied after a look change to take effect. An omitted camera group preserves the active virtual film exposure and metering controls. Application clears transient Scan-for-print state, invalidates dependent render state, and schedules automatic Preview.
- Applying a look preset normalizes the scanner output: when the resolved film is positive or the selected workflow route is not a film-scan route, the output becomes the direct-scan mode. A route the caller already declared as `input > film > scan` or `input > convert-film > scan` is preserved, so a document that selects the negative conversion keeps that selection on those routes. `core` owns that rule once for the GUI and the CLI.
- Applying a look preset does not change recorded explicit-leaf ownership. Only a section preset or a sparse parameter source marks its own leaves explicit, and those marks survive a later look application.
- Applying a section preset replaces exactly the controls of its section, and those values count as explicit. Every other control retains its value, including explicit values from an earlier preset or parameter source, so the result does not re-derive them from stock defaults. It clears transient Scan-for-print state, invalidates dependent render state, and schedules automatic Preview.
- Applying a section preset that would introduce a scanner-output, workflow-route, scan-film, film-profile, or grain-engine combination the pipeline rejects must fail without changing any control. A state that already fails that check may still receive an unrelated section edit.
- Applying a section preset must not replace the remembered applied preset. It is an edit, so an applied look preset becomes modified.
- Each successful look or section preset application must create one undo transaction under [Edit history](../gui-workflow/spec.md#edit-history), including explicit-leaf provenance and dependent normalization. A failed application must not create a history entry. Undo and Redo must not reverse preset-file or library writes.
- Applying a preset preserves loaded image and metadata, RAW/source settings, source exposure correction, input color interpretation, crop, resize, rotation, viewer/display state, Save settings, and Export settings.
- Built-in presets are immutable. Editing controls after an apply never rewrites the built-in definition.

### Workflow defaulting

Changing a film profile selects the corresponding workflow default while the workflow remains user-editable:

```mermaid
flowchart LR
    F["Selected film profile"] --> T{"film_type"}
    T -->|"negative"| N["Default: input > film > print > scan"]
    T -->|"positive"| P["Default: input > film > scan"]
    N --> W["Workflow selector"]
    P --> W
    U["User workflow choice"] --> W
    W --> R["Resolved RuntimeParams"]
```

A profile change must not apply hidden stock-specific look values. The workflow selector remains editable until the next film profile change. The positive path does not execute print stages; its explicit print profile remains part of the portable pair.

A serialized `route` or `workflow` field is unknown and rejected.

### Look ownership

A look preset owns film and print selection plus explicit film, print, enlarger, diffusion, halation, coupler, grain, base, chemistry, scanner, CMYK ink style, and output-mix controls. The scanner controls it owns are `scanner.scan_output`, `scanner.conversion_style`, `scanner.lens_blur`, and `scanner.unsharp_mask`. Viewing glare ownership is limited to effective `print_render.glare`; the non-effective `film_render.glare` group is not preset-owned. Neutral enlarger filter anchors are calibration baselines, not look controls, and are not preset-owned. Existing material controls and `output_mix` follow [Output mix and material controls](../output-mix/spec.md). A section preset owns exactly the controls of its section. Virtual film exposure compensation and metering belong to the look through the optional `parameters.camera` group. Other camera controls remain outside this group; camera diffusion remains a separate look-owned group. The assumed scene light belongs to the look through `camera.capture_illuminant`, which is an optional camera leaf under the [capture illuminant contract](../image-input/spec.md#capture-illuminant): an omitted leaf keeps the selected film profile's `reference_illuminant`, an explicit catalog name or `BB<kelvin>` token overrides it for the camera-side conversion into film exposure only, and a generic look must not hard-code one stock's tungsten or daylight reference. A look also owns look-relevant processing settings: hanatos2025 adaptation window/surface switches, spectral Gaussian blur, CAT16, and the print-exposure EV tracking choice (`LookSettingsParams`, a subset of `SettingsParams`).
### Surface parity

Every leaf under `parameters` that a preset carries is a user-settable parameter. A carried parameter leaf **must** have both of these surfaces:

- the GUI has one editable control or one named action that writes that exact canonical runtime value; an inactive condition may disable the surface, but must retain the value and explain the condition;
- the CLI accepts the same value through the canonical `--set` path and exposes that path through `describe --field` or `describe --module` with the shared type, validator, units, and conditions.

The GUI label and CLI syntax need not be identical, but they must edit the same runtime leaf with the same semantics. A value that can only be populated by preset capture, profile defaults, database calibration, migration, or internal state is not a preset-owned user parameter. It must either gain both surfaces or be removed from preset capture and validation. Provenance, profile references, and other preset metadata are outside this leaf-parity rule.

This rule applies equally to whole-look payloads and section payloads. Capturing, importing, migrating, or exporting a preset must not create a hidden parameter surface. The section table may contain only sections whose carried leaves satisfy this rule and have a visible GUI owner.

### Implementation rules

- `core` owns the typed look model, strict schema validation, profile compatibility, profile-driven workflow defaulting, capture, candidate resolution, the library directory policy, discovery, name conflict rejection, and atomic document writes.
- `gui` owns browser presentation, group and row state, library actions, active and modified state, transient state, and user-editable workflow controls.
- `GuiState` remains complete GUI-state persistence and is not a look payload.
- `core` owns strict parsing, profile resolution, stock-baseline construction, explicit look-value overlay, workflow-default derivation, and `RuntimeParams` validation. Neutral print-filter anchors (`enlarger.c_filter_neutral`, `enlarger.m_filter_neutral`, and `enlarger.y_filter_neutral`) are calibration baselines supplied by the per-paper/illuminant/film database, not preset-owned leaves. They have no ordinary GUI/CLI look surface; explicit calibration state remains in runtime preparation and GUI-state compatibility and must not be confused with a reusable look. The non-effective `film_render.glare` group, derived `film_render.grain.monochrome`, no-op `film_render.grain.micro_sublayers`, and `enlarger.lens_blur` (zeroed by digest and applied by no upstream stage) likewise remain outside preset capture and section ownership. A look protects a neutral print-filter axis only when a legacy or explicit runtime source has authored that axis; an axis the look does not carry keeps the [per-paper database](../film-chain/spec.md) baseline instead of one paper's pack being reused everywhere. The authored-versus-baseline state persists in the parameter document so replaying a resolved document reproduces its render bit-identically; documents without the flags treat every axis as baseline. The exposure normalization refines baseline axes from the database's f64 values while the published parameter surface carries their f32 views (authored axes stay exact); a render that consumes only the published f32 views must agree within 1e-8 absolute on the f64 reference values (measured 6.4e-10 in the report/render acceptance fixture).
- Existing `--params` invocations, RenderRecipe, LUT behavior, CPU f64 semantics, WGPU f32 semantics, Grain V2 algorithm, and raw profile assets must retain their `0.3.4` results with `output_mix.amount = 1`, except for the explicit [output lightness retirement](../output-mix/spec.md#output-boundary-and-retirement) and [optical-order cutover](../film-chain/spec.md#optical-order-cutover). The cutover does not change stored blur units, grain selectors, or print-glare ownership. The [CLI parameter language](../cli/spec.md) owns command syntax and composition rules.

The portable canonical snake-case schema contains:

- `schema_version`, `name`, optional `author` and `description`;
- `film_profile` and `print_profile`, each with stock, optional profile version, and optional `sha256-<lowercase-hex>` content identity; both are required in a look preset and absent from a section preset;

- `parameters`, the typed look-owned parameter object: every required group and the optional camera group in a look preset, the paths of its section in a section preset;
- `provenance`, the SpektraFilm implementation and model versions.

It must not contain `route`, `workflow`, image pixels/paths, RAW data, LUT pixels, viewer rasters, GUI paths, credentials, random seeds, or Dehancer-specific fields. Unknown fields are rejected. The resolved candidate is returned without mutating active GUI state; GUI commits only after successful resolution.

### File formats

TOML must be the default preset export format. Readers must accept `.toml` and `.json` files as carriers of the same typed schema. The file extension must select the parser. A reader must not try another parser after a parse failure. Both carriers must apply the same version, ownership, profile, type, and value validation.

The preset schema version must be 6. The [lab scan cutover](../film-chain/lab-scan/spec.md#point-wise-consumers-and-migration) moves acquisition light from scanner lamp keys into the Convert owner in complete legacy documents, normalizes the input style alias, and validates historical scanner provenance before re-deriving it. Version 6 documents must reject scanner lamp keys. The previous schema version was 5 because the negative conversion controls and the enlarger print exposure mode rename their paths, and the terminal `cmyk` group replaces the retired magazine spelling. A version 1 through version 4 document is accepted only through an explicit migration. Version 1 migration must discard the old `id`. Migration of a version 1 or version 2 whole look must supply `output_mix.amount = 1` and retain existing material controls and M/Y shifts; the neutral enlarger filter anchors are calibration baselines rather than look values, so a legacy document that carries them is rejected rather than migrated, exactly like any other not-preset-owned field. Version 3 to version 5 migration must map the existing scanner and enlarger legacy paths to their canonical controls, then normalize the retired CMYK group and route only for an approved complete document. The migration must preserve rendered pixels and must not keep a legacy path beside its canonical path. An older section must preserve sparse omission and must not acquire new leaves. Resolving, exporting, or updating a migrated whole look must not capture or replace caller `io` settings. Exporters must write version 6. Legacy renamed paths and the retired CMYK group in a version 5 document must fail unknown-field validation. The unshipped `film_render.response`, `print_render.response`, `enlarger.c_filter_shift`, `reproduction`, and `io.finished_rgb_output_processing` fields must fail normal unknown-field validation in every accepted version, including draft legacy documents. The unshipped `output response` section must be rejected. None has a compatibility alias, default, or migration.

A look preset must contain every required look-owned group. An exporter must write every non-optional editable look control so a newly captured preset does not depend on implicit defaults. Readers must retain the existing omitted-field and JSON null decoding rules of the typed model, subject to the camera group rule below. Readers must materialize other omitted controls before resolution or re-export. TOML absence must represent an unset optional control; an exporter must omit unset optional controls in TOML. The two carriers must not introduce a separate schema version or field vocabulary. A future change to accepted omitted-field behavior must use an explicit schema migration rather than silently rejecting existing documents.

`parameters.camera` is an additive optional group in schema version 3. When present, it must contain exactly three required fields: `exposure_compensation_ev` (f32), `auto_exposure` (boolean), and `auto_exposure_method` (string). Readers must reject a partial group, a null group, and unknown fields within it. Look presets in any accepted schema version that omit the group must retain its absence during migration, reading, and re-export. Applying such a document must preserve the active values of all three controls. Look matching and the modified marker must ignore these controls when the applied document omits the group. A new look capture must emit the complete group with the three active values. A version 1 or version 2 document that carries the group must be rejected as a schema violation rather than migrated, because the field has no legacy vocabulary. `auto_exposure_method` must satisfy the same vocabulary as a sparse parameter source, and that validation must apply before resolution so a whole look cannot admit a value the metering dispatcher does not implement.

`camera.capture_illuminant` is a fourth optional leaf inside that group, governed by the [capture illuminant contract](../image-input/spec.md#capture-illuminant). TOML absence must mean the leaf is unset, an exporter must omit it when unset, and an unset leaf must resolve to the selected film profile's `reference_illuminant` rather than to a stored default. A JSON `null` clears it to the same unset state. It must satisfy the shared illuminant vocabulary before resolution, so a whole look cannot admit a light source the spectral service does not provide. The three required fields above remain the required set; this leaf is optional and its unset state is meaningful. A new look capture must therefore emit the leaf's active state alongside the three active values — the explicit illuminant name or `BB<kelvin>` token when the control holds one, and an explicit unset value when it does not — so re-applying the capture restores the captured state instead of silently inheriting. Look matching and the modified marker must compare the leaf's presence and value exactly as they do for the three required fields.

A section preset must carry exactly the parameter paths of its section and must not carry a profile reference. Its payload must follow the leaf-merge rules of a parameter-file source in the [CLI parameter language](../cli/spec.md), including the rejection of unknown fields, duplicate keys, and omitted values acquired by default-filled deserialization, and an exporter must write exactly what it carries rather than filling omitted controls. Both kinds use the same schema version, the same name rules, and the same profile and value validation for what they carry.

Export and reload must preserve the represented finite f32 and f64 values. Export must not reduce numeric precision for shorter text. Readers must accept TOML comments. Re-export need not preserve comments or source formatting.

Readers must reject non-finite numeric controls and numeric values that overflow their declared f32 or f64 type. TOML `nan`, `inf`, date/time values, and numbers that become non-finite during typed conversion must not reach the resolver. Readers must not convert unsupported carrier types to strings. This rule must apply before runtime validation so JSON and TOML cannot admit different numeric domains.

Preset files must not support includes, inheritance, expressions, environment interpolation, or references to GUI state files.

### GUI state compatibility

GUI state must retain its JSON contract. Loading a GUI state must not interpret it as a preset. Importing a preset must not interpret it as a complete GUI state.

Saving state after applying or editing a preset must store the actual controls and the calibration policy needed to restore them. Restoring that state must preserve explicit neutral filters through runtime preparation. The saved state must not depend on the preset file or built-in definition remaining available or unchanged. Any new Rust-owned state metadata must use the existing versioned Rust extension and must preserve existing Python state normalization.

Complete GUI states, render recipes, and presets that carry a versioned complete runtime document must migrate the retired `magazine_print_color` group and route only at their own legacy boundary. A current preset schema 4 document must reject the retired group; the preset schema bump required for canonical `cmyk` must be recorded separately from the parameter-document bump. Successful migration must preserve omitted section leaves and must not materialize `cmyk` into a section preset that did not carry the feature. Their versioned legacy complete-document migration must apply the [output lightness retirement rule](../output-mix/spec.md#output-boundary-and-retirement): allow an absent or null `io.output_gamut_compress.lightness_compression`, but reject every explicit numeric triplet even when the old gamut algorithm ignored it. The error must identify that exact path, state that no automatic replacement exists, and require explicit removal. It must not suggest a deleted response group. Other caller `io` settings must remain intact. Current complete documents, sparse sources, and inline assignments must reject the retired leaf even when null. New writers must never emit it. The unshipped response and Cyan-shift fields must fail normal unknown-field validation rather than gain defaults or migration support. A migration failure must preserve the source document, active state, and existing output.

Capturing a preset from the GUI must use the active look controls. Applying it must preserve the excluded context defined in [Look ownership](#look-ownership). A captured section must carry an unset optional control as an explicit null, so re-applying the capture restores the captured unset value; a capture that carries explicit nulls uses the JSON carrier.

### Resolution flow

The preset and active render context are separate inputs to the same runtime:

```mermaid
flowchart LR
    P["Preset"] --> V["Strict validation"]
    C["Active render context"] --> R["Core resolver"]
    V --> R
    R --> B["Profile baseline"]
    B --> L["Explicit look values"]
    L --> W["Profile-driven workflow default"]
    W --> Q["RuntimeParams validation"]
    Q --> O["Preview / Scan / Save / Export"]
```

The resolver must preserve image/source, geometry, viewer, display, Save, Export,
debug, precision, backend, and random-seed context. The GUI commits the returned
candidate only after resolution succeeds.

### Transaction and stale results

Applying a preset validates and resolves first, commits its carried controls together, preserves excluded context, clears transient Scan-for-print, increments a parameter revision, invalidates caches, and marks the render dirty. A look application also commits film and print selections and the profile-derived workflow default under [Apply](#apply). A section application must preserve profile selections and the workflow. Render workers carry the parameter revision and stale results cannot replace a newer preset result.

The GUI must commit the application as the single edit-history transaction defined under [Apply](#apply). Parameter revisions must continue to reject stale render results after Undo or Redo.

A failed application must preserve active controls, the applied marker, transient Scan-for-print state, and the parameter revision. Only the error status may change.

CLI preset selection must use the same core resolver and name rules as GUI preset application. The [CLI parameter language](../cli/spec.md) defines selection, sparse parameter files, and command compatibility.

The version 5 scanner cutover must not reopen older retired-field migrations. A version 5 document that carries the retired CMYK spelling or legacy conversion fields must still fail. A sparse scanner section must normalize `neutral` to `standard` without acquiring omitted leaves. The readout cutover intentionally changes negative scan appearance; it does not promise historical pixels for the migrated built-in style.
