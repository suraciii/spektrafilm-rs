# Preset design

The [preset specification](spec.md) owns the approved browser behavior. This document owns the existing library model, isolated candidate rendering, state separation, and native cutover; it does not define a second parameter schema or renderer.

- Vocabularies: a **preset** is a named parameter set — the user-facing one is a library document (`Built-in` compiled from the crate's `assets/presets`, `My Presets` read from `<config_dir>/presets`). `data/presets/` keeps the upstream scope, the per-stock grain and coupler tables the runtime compiles in.
- A **look** is the content a film look preset owns: the look controls and their values. A **film look preset** is the whole-look film kind and a **section preset** the one-section film kind; the terms *look document* and a `looks` directory MUST NOT appear.
- The LUT preset is the third whole-look kind. It uses preset schema 8, carries an external LUT file path and SHA-256 plus the LUT declaration, and carries no film or print profile references.
- LUT preset library, browser, and thumbnail behavior follow the [LUT workflow](../lut-workflow/spec.md#lut-presets). It MUST NOT be resolved as a film look or a section.
- The per-stock grain and coupler tables in `data/presets/` are **not** upstream data. Pinned Python 0.3.4 carries no such database; it applies hardcoded polarity overrides in `runtime/params_builder.py` (`is_negative` / `is_positive` branches).
- The Rust tables replace those overrides with a stock → cine → defaults lookup and their own optimized values. Any parity claim that touches `film_render.dir_couplers` or `film_render.grain` therefore compares structure and magnitude, not sample equality; a numerical diff against pinned Python is expected and is not a defect.
- *Calibration* stays reserved for the Convert device-correction matrix and its worker. Names that mirror the pinned upstream (`apply_grain_preset`, `HALATION_PRESETS`, `procedural_profile` and its values) keep their upstream spelling for parity traceability.
- The preset document, strict validation, resolution, capture, the library directory policy, discovery, name conflict rejection, and atomic writes belong to `core`.
- The Rust document type is `presets::Preset`, named after its module like `profile::Profile`. `LookParameters` remains the film look payload; a LUT preset carries its own LUT declaration, effects, and grade payload.
- A film look preset's `settings` group (`LookSettingsParams`) carries only the look-relevant processing settings (hanatos2025 adaptation window/surface, spectral Gaussian blur, CAT16). It does not carry the wavelength grid, LUT wiring, preview limits, or performance toggles.
- Built-in preset documents are repository files in `crates/spektrafilm-core/assets/presets`. The build MUST compile them in, so built-in listing and name resolution MUST NOT require a data directory, and packaged data directories MUST NOT carry copies.
- Built-in LUT presets are outside this contract; built-in documents remain film look or section presets.
- A document is a **film look preset** with both profile references and every required look-owned group. Its optional camera group follows the [preset file format rules](spec.md#file-formats).
- A document is also a **LUT preset** with an external LUT file path and hash, declaration, effects, and optional grade, or a **section preset** with exactly one film section. Neither carries profile references.
- The kind is derived from the carried scope, never declared by a field, and a built-in document MAY be a film look or a section. A document that carries neither a whole film look, a whole LUT look, nor exactly one section MUST be rejected.
- A section preset MUST pass the same value validators as a film look preset for the paths it carries. Its payload MUST follow the parameter-file leaf discipline: only supplied leaves are edited, unknown fields and duplicate keys fail, and omitted values MUST NOT be filled from defaults.
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

## Explicit preparation ownership

The runtime parameter owner MUST provide operations to record validated explicit leaf paths, release paths in a supplied scope, and compare explicit ownership without allocating. Scope membership MUST match a complete path segment, not an arbitrary string prefix. GUI edits, scoped copying, feature profiles, section presets, and module resets MUST use these operations rather than modify a serialized preparation snapshot.

Explicit-path ownership and neutral-filter-axis protection MUST remain separate operations. The parameter owner MUST implement the canonical path-to-filter-axis mapping once. Source resolvers MUST retain their existing decisions about which operation applies; an internal value patch or a legacy complete parameter document MUST NOT acquire explicit ownership merely because it assigns a value.

Material preparation MUST retain explicitly owned controls through the parameter-preparation boundary. It MUST NOT depend on preset document code to capture or restore those controls. `ParameterEdits` MUST remain the shared value-patching implementation and MUST NOT mark ownership implicitly.

`RenderPreparationState` remains the persistence and restoration carrier. Recording, releasing, and comparing explicit paths MUST NOT normalize unrelated neutral-filter flags, including the distinction between absent flags and present all-false flags. Existing carrier fields, versions, migration rules, and numerical precision MUST remain unchanged.

Acceptance MUST cover an authored value equal to a material default, scoped release with unrelated ownership retained, and legacy baseline preparation without newly inferred ownership. GUI metadata-only reset and history acceptance follow [Section reset](../gui-workflow/spec.md#section-reset).

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
- `scanner`: `scanner.scan_output`, `scanner.white_reference`, `scanner.conversion_style`, `scanner.experimental_setup`, `scanner.operator`, `scanner.lens_blur`, `scanner.unsharp_mask`. Primary module anchor: Scan / Scanner.
- `cmyk ink style`: `cmyk`. Primary module anchor: Output / CMYK ink style.
- `print`: `print_render`. Primary module anchor: Print / Chemistry.
- `output mix`: `output_mix`. Primary module anchor: Output / Simulation blend.
- `finishing`: the independent [tone and color controls](../finishing/spec.md#controls). Primary module anchor: Scan / Tone & Color.

The `convert` section includes the imported-capture acquisition light in `film_render.convert.scan_illuminant`. The fixed product readout illuminant is not preset-owned. Retired scanner lamp keys and the input-only `neutral` style alias follow the [lab scan migration contract](../film-chain/lab-scan/spec.md#point-wise-consumers-and-migration).

- The browser lists film look and LUT presets only, in one list, and MUST NOT separate them by workflow or offer a workflow-type filter. A section preset MUST NOT appear in it, and the GUI MUST NOT offer a section menu, section capture, section management, or section import.
- A section payload is captured and applied by `core`, the CLI, and scoped photograph copying under [Photograph work](../gui-product-design/photo-work/spec.md#copy-adjustments). Stored section documents keep their payloads and file locations.
- The `enlarger` section includes the Print / Preflash controls; the `print` section includes print chemistry, print base, print density-curve controls, and Scan / Print viewing glare. Those placements MUST NOT split sections or change schemas or canonical payload scopes.
- A section preset MUST NOT change a profile reference or the workflow.
- Visible stage and module ownership follows [Stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership), not serialized group names. Module resets follow [Section reset](../gui-workflow/spec.md#section-reset); a section's wider payload MUST NOT expand a visible module's reset scope.

Tone and color capture, application, sparse omission, and legacy complete-look migration follow [adjustment persistence](../finishing/spec.md#persistence-and-replay). The `finishing` section remains independent of `camera`, `scanner`, and output mix; its Scan placement MUST NOT broaden any section's payload.

## Everyday flows

The flows a user actually runs MUST be short; every one below is two or three steps, and none of them needs a dialog that asks for a scope.

- Start a look: pick a browser row. Profiles, carried look controls, and the workflow default follow, and a render is scheduled.
- Change an applied look: edit look controls, then Save preset under a new name or explicitly replace a user document. Reactivate a card to reapply its stored look; Undo restores the previous accepted adjustments.
- Move a preset between machines: Export the document, drop the file into `<config_dir>/presets`, refresh. The same name then resolves for the GUI and the CLI.
- Restore a photograph or explicit state: restore its controls and applied identity without rereading the preset. An ordinary launch uses startup controls and layout preferences without inheriting a global last-used look name.
- Script a run: `preset list` and `preset show` describe the same names and groups the browser shows, and `--preset` resolves them with the same rules.

## Names and groups

- A preset name is its identity. The document `name`, the file stem, and the selector MUST be the same string.
- The built-in documents form the `Built-in` group. The library directory forms the `My Presets` group.
- A name MUST be unique inside a group. Two built-in documents MUST NOT share a name, and two library documents MUST NOT share a name; the same name in both groups is valid.
- Name comparison MUST be case-insensitive, because some file systems resolve names without case.
- Browser rows MUST sort by name, case-insensitively, with a deterministic tie-break. A file-system scan order MUST never reach the UI, and one library MUST present one order.
- Browser search MUST implement the [browser search contract](spec.md#browser-search) as a transient presentation filter over discovered film look and LUT presets. It MUST match a case-insensitive substring of the preset name, a film look's referenced film or print stock names or identifiers, and a LUT preset's LUT file name, title, and path.
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
- Export into the library MUST use the document name as its file stem and MUST create a new name. It MUST NOT replace an existing library document; [named replacement](spec.md#saving-and-replacing) owns GUI look replacement.
- Deleting or updating one document MUST NOT modify another document or a built-in document.

GUI look replacement MUST load and validate the intended user target while holding the existing mutation lock. It MUST preserve the target's canonical identity and metadata under [Saving and replacing](spec.md#saving-and-replacing), without narrowing core support for section-document updates.

## Boundaries

- Explicit GUI-state and photograph carriers retain complete accepted controls and applied group/name identity under [browser state ownership](#browser-state-ownership). Global browser preferences retain layout only; neither carrier changes preset documents or Python-compatible state sections.
- At a window width of 1250 logical points or less, the browser MUST automatically collapse to a 48-point rail.
- A manual reopen at that width MUST remain session-local and MUST NOT rewrite the serialized expanded or width preference.
- Returning to a wider window MUST restore the stored expanded and width preferences.
- Modified detection compares the active preset kind's payload with the applied document. A film look MUST use `LookParameters::matches` and its film and print references to respect an omitted legacy camera group under the [preset file format rules](spec.md#file-formats).
- A LUT preset MUST compare its LUT file path and hash, LUT declaration, effects, and grade. Neither kind MAY compare GUI state or a prepared runtime parameter tree.
- Film look capture MUST collect existing material controls through their existing sections and output mix through its own section; it MUST never collect an `IoParams` snapshot or output gamut settings.
- LUT preset capture MUST collect the LUT declaration, effects, and grade and MUST NOT collect film or print controls. Schema migration MUST follow the [preset file-format contract](spec.md#file-formats): older whole looks gain full output mix, while older sections retain sparse omission.
- Capture, apply, and migrated export MUST NOT expand `io` ownership; preparation MUST preserve existing material capability, scan-inverse, and calibration semantics under [Output mix and material controls](../output-mix/spec.md). Unshipped response groups and the Cyan-shift field are unknown fields, including in draft version 3 documents; neither capture nor migration MAY materialize them.
- Built-in documents MUST stay read-only at run time. A user edit MUST produce a user document or no change.
- The library menu owns resolved-directory display, reveal, refresh, and import under [library actions](spec.md#browser-state-and-library-actions). Discovery remains explicit; no directory watcher or second index is introduced.
- Renaming the applied user preset MUST follow the marker to the new name. Deleting it MUST leave the look controls untouched and MUST show the remembered name as missing.
- Restoring GUI state MUST restore the stored controls and MUST NOT re-apply the remembered preset document. The applied marker is recomputed against the restored controls.
- [Current-photograph thumbnails](spec.md#current-photograph-thumbnails) MUST NOT change preset discovery, identity, resolution, or document schemas. User-selectable sorting, collections, per-scope preset memory, inheritance, remote distribution, and scanning directories other than the library directory remain outside this boundary.

## Thumbnail execution

### Candidate preparation

The GUI MUST prepare candidate requests from one immutable accepted photographic snapshot, not mutate `App` and undo the change afterward. It MUST share the source image through existing immutable ownership rather than copy the full photograph for every card.

Candidate preparation MUST branch by preset kind. A film look candidate MUST use core's production whole-look resolver and the same transient-state normalization as accepted look application.

Explicit-leaf provenance and omitted-camera preservation MUST remain part of that preparation. A LUT preset candidate MUST use the LUT workflow resolver and LUT file identity and MUST NOT prepare spectral materials. A default-filled parameter tree is not an equivalent context for either kind.

Every candidate MUST start from the accepted source and input preparation, not the current Modified image. A film look candidate's own metering, frame setup, material preparation, and look-owned values MUST follow the normal spectral pipeline.

A LUT candidate MUST resolve its LUT file and declaration without spectral preparation. One candidate's derived preparation MUST NOT seed another candidate.

Candidate requests MUST include the source's accepted interpretation and geometry, the chosen backend and native precision, and the unchanged stochastic seed. Candidate processing MUST NOT add exposure fitting, disable authored effects, or infer a missing look from display pixels.

Thumbnail rendering MUST reuse the existing render and display paths at a bounded requested preview size. The exact full-working-frame requirements of [negative scan preview](../film-chain/lab-scan/spec.md#exact-preview-and-prepared-baseline) MUST take precedence over cheap early downsampling.

The display transform, ICC policy, and output color interpretation MUST match the photograph viewer. Zoom, pan, analysis overlays, and other viewer decorations MUST NOT enter candidate pixels.

### Scheduling and result ownership

Thumbnail demand MUST describe the visible cards for the current context, not append one unbounded job per frame, scroll event, or search character. Demand replacement MUST discard obsolete queued work; a collapsed browser or no accepted source MUST request no new thumbnail renders.

The execution owner MUST allow at most one active candidate render and no more queued candidates than the visible demand set. It MUST NOT render unrelated offscreen cards speculatively or create a worker thread per card.

Main Preview, Scan, and independent Export MUST take priority. On foreground demand, thumbnail work MUST yield through the existing safe cancellation boundary, and no further candidate MUST start until foreground work permits it.

Thumbnail jobs MUST NOT occupy or supersede the main Preview/Scan latest-request slot. Their completion path MUST publish only thumbnail state and texture data, never a retained frame, main status, calibration result, history entry, or deliverable.

The implementation MUST reuse execution components rather than add a second simulation pipeline. Sharing a worker or backend resource MUST preserve independent foreground request identity and serialize access where existing caches or device resources require it.

### Identity, invalidation, and cache

A thumbnail result MUST carry its accepted-source generation, candidate identity, effective resolved context, and render/presentation dependency identity. The browser MUST verify those identities against current demand before publishing pixels.

| Dependency | Required identity or invalidation |
| --- | --- |
| Accepted input | Source acceptance generation and its actual decoded/prepared image; failed RAW drafts do not become accepted input. |
| Candidate | Group/name plus validated document contents, not name or list position alone. |
| Effective context | Resolved controls, preparation provenance, preserved input/geometry, effective route, scan mode, and seed. |
| Material and model data | Dependencies used by the production resolver and preparation; a library refresh alone is not proof that unchanged names mean unchanged data. |
| Execution | Backend/device identity, native precision, and requested thumbnail size. |
| Presentation | Output color interpretation and display-transform/ICC dependency identity. |

The main parameter revision alone MUST NOT serve as the cache key: applying another whole look can leave a candidate's effective context unchanged. Conservative invalidation is valid; equality or version checks MUST NOT reuse a result after a dependency changed.

Source replacement MUST clear old-source textures and invalidate pending completions before new-source demand can publish. Edits to context a candidate preserves MUST invalidate that candidate, including virtual camera values when a legacy document omits its camera group.

Changes to display presentation MAY re-present a still-valid candidate raster instead of rerunning simulation. The presentation result MUST still match current display dependencies before it is shown.

The cache MUST be session-local, bounded by explicit finite byte and entry limits, and restricted to the accepted source. Its accounting MUST include cached rasters, GUI textures, queued completions, and retained preparation data attributable to thumbnails.

Thumbnail demand, identity, errors, and textures MUST NOT enter preset documents, photograph records, GUI-state exports, startup defaults, or persistent browsing files. A failed result MUST remain an unavailable state for its request identity, not trigger a retry loop every frame; explicit refresh or a changed dependency MAY request it again.

## Browser state ownership

The GUI MUST keep layout preferences separate from photographic applied identity. Existing `rust.presets` and photograph fields remain valid for explicit restoration; the global carrier MUST neither restore nor write a last-used applied identity.

Layout-only restoration MUST cover old browser carriers and the fallback to `gui_default_state.json` when a carrier is absent. Loading that file automatically at startup is not an explicit GUI-state load.

Explicit state loading, photograph navigation, and Undo/Redo MUST retain their existing source/control restoration boundaries. A new source that deliberately inherits accepted adjustments MAY inherit their applied identity; it MUST NOT acquire one from unrelated global preferences.

Modified detection MUST use the applied group/name and the payload matcher for the applied kind. A film look uses `LookParameters::matches` with film/print references; a LUT preset compares its LUT file path and hash, declaration, effects, and grade. It MUST NOT infer explicit ownership, use prepared render trees, or depend on thumbnail availability.

Core MUST retain one capture, validator, resolver, library, and atomic-write authority. Film look optional camera and Tone & Color ownership, section-document support, and workflow/scanner normalization remain unchanged. LUT preset capture and LUT file resolution MUST use the same library authority without decoding the LUT into a film look.

## Implementation boundary

This is the approved native implementation target, not a claim that the replacement browser ships. The existing browser uses a fixed-reference bundle and discovery preferences; the HTML review only demonstrates interaction with pre-rendered images.

The native cutover MUST replace that browser rather than keep old and new versions or a compatibility switch. It MUST remove fixed-reference loading/generation/fingerprints, dedicated image assets, credit footer, Favorites/Recently used filters, their preference writes, and obsolete inventory/catalog entries.

An old `preset_discovery_preferences.json` file MUST remain untouched and MUST NOT be read or migrated into another model. Actual user preset files, unrelated preferences, explicit GUI-state applied fields, and photograph records MUST remain intact.

Removing the reference bundle MUST update [asset attribution](../../docs/attribution.md) for any removed derivatives. Notices required by remaining assets MUST remain; bundled preset and material documents MUST NOT be removed with preview resources.

The change MUST NOT add a preset/state schema version, UUID, external renderer service, persistent cache database, new CLI option, generic effect-strength control, or partial GUI capture mode. Broader audition, arbitrary candidate A/B storage, cloud distribution, and recommendation systems remain out of scope.

Implementation MUST follow this dependency order:

1. Extend the existing core mutation boundary for valid-look replacement, retaining current section updates, canonical names, metadata, carrier rules, and cross-process conflict protection.
2. Add isolated candidate snapshot, bounded execution, dependency identity, cancellation, cache, and auxiliary result routing through existing render/display owners.
3. Compose the compact browser with asynchronous states, keyboard operation, library actions, layout-only preferences, and the existing accepted-edit transaction.
4. Remove superseded assets and state consumers, migrate affected inventory and bilingual messages, and update native acceptance and user guidance as one cutover.

The [GUI guide](../../docs/gui.md) and [preset guide](../../docs/presets.md) MUST describe the replacement when native implementation lands, not present a design-only HTML page as shipped behavior. Existing wording-only or removed-affordance assertions MUST be deleted rather than re-pinned; meaningful ownership, state, history, file, and stale-result regressions remain required.

Native verification MUST satisfy [preset acceptance](spec.md#acceptance) plus the owning render, display, file-format, and numerical contracts. It MUST exercise real candidates for both groups, foreground interruption, source replacement, retained Save versus independent Export, IME/keyboard input, and wide/narrow layouts.

Work/cache limits and operation responsiveness MUST be measured with declared source size, route, backend, precision, preview size, and library/visible-card counts. A timing from the static Page, a different renderer build, or a preloaded JPEG MUST NOT establish native performance or pixel equivalence.
