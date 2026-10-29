# GUI product design implementation boundaries

## Authority

[GUI product design](spec.md) defines the user-visible workspace. [GUI workflow](../gui-workflow/spec.md) owns stage and surface assignment, complete control access, reset scopes, edit history, and worker invariants. This document owns the composition of those boundaries.

It MUST NOT define another parameter schema, preset resolver, or render pipeline.

## Workspace composition

The native application MUST compose the existing preset browser, viewer, and control panel into the three-region workspace. It MUST reuse the existing toolkit and [presentation components](../gui-presentation/design.md).

The right panel MUST contain one continuous stage inspector rather than one control tree per tab. Stage jump actions and collapsible headings MUST address the same module instances. Application settings and Export dialogs MUST render only their assigned owners.

Closing or opening a dialog MUST NOT create another live editor for the same value.

A read-only material summary, a preset card, and a search result MAY identify a value. They MUST NOT become independent editing models. The viewer MUST continue to consume retained image state, not inspector widget state.

Independent module disclosure is preferred to basic/expert modes because it preserves a single complete control inventory. Stage jumps are preferred to retaining both old tabs and a new sidebar because a control needs one stable location. These choices do not change pipeline stage execution or preset serialization scope.

The viewer's analysis workspace MUST use the [image-analysis boundary](image-analysis/design.md). Its regions and measurements MUST NOT become photographic editors or another subject-selection model.

## Control inventory and shared metadata

One GUI-owned control inventory MUST bind canonical paths to their visual owners, supported editor or named action, and applicability presentation. It MUST reuse existing typed field descriptions, validators, units, ranges, and feature-profile resolvers. It MUST NOT copy numeric defaults, bounds, enum lists, or profile tables from the web prototype.

The inventory MUST supply search locations and the control index. The module-to-path projection used by reset and modified detection MUST follow [GUI presentation design](../gui-presentation/design.md). Parameter discovery defines public parameter semantics; the GUI inventory defines only where and how an accepted control is presented.

Composite controls MUST bind every component to its canonical leaf and component identifier. An aggregate editor MUST NOT silently couple independent components. Derived components MUST reuse the owning derivation and validation, not a second GUI approximation.

Optional editors MUST distinguish an explicit value from inheritance and display the effective inherited value.

UI applicability MUST come from route, source, material capability, engine, and feature conditions. A shared condition MUST NOT be independently reimplemented by row rendering, search, and reset. The view MUST distinguish an unavailable editor from an absent control.

Inactive conditions MUST NOT mutate parameters during rendering.

## Module scope versus preset scope

A visual module owns its displayed canonical paths. A serialized preset section owns the path set in [Preset design](../presets/design.md#sections). These sets need not be identical.

Preflash illustrates the boundary: its module owns three leaves, while the `enlarger` section retains its broader serialized payload. Print viewing glare is displayed under Scan while its preset ownership remains in `print`. The module reset MUST use the visible-module projection; section capture and application MUST use the core preset scope.

The GUI MUST NOT present a section-preset menu, section capture, or section management. Section documents remain shared by `core`, the CLI, and scoped photograph copying under [Photograph work](photo-work/spec.md#copy-adjustments).

Inspector layout changes MUST NOT change serialization keys, section payloads, or the core capture scope.

Feature-profile actions MUST use the shared feature resolver. They are scoped edit operations rather than new preset documents or GUI state fields. The Halation selector derives its effective named or custom state from its two scoped arrays.

Procedural keeps its existing stored profile and override semantics.

## Accepted edits and history

All photographic editors and actions MUST enter the application's existing validated-edit boundary. A candidate edit MUST be normalized and validated before publication. One accepted transaction MUST include the edited values, dependent material/route/scanner changes, explicit-leaf provenance, applied-preset identity, and any affected transient rendering state.

Rejection MUST leave all of them unchanged.

The application MUST capture a transaction's pre-edit and accepted post-edit control state. History MUST store control changes and the small associated state needed to restore them, not cloned images, profile spectral arrays, prepared runtimes, or export jobs. The implementation MAY reuse existing immutable material references.

Slider input MAY publish intermediate valid control states for interactive preview. The gesture MUST retain one pre-edit boundary and create one history entry at completion. Cancellation MUST restore that boundary through ordinary invalidation.

Numeric text keeps a separate editing buffer until the value is accepted. A view rebuild MUST preserve the focused canonical field and its uncommitted buffer.

Undo and Redo MUST apply recorded accepted transactions without reading a changed preset file or rerunning stock initialization. They MUST restore provenance along with numeric values; restoring values alone can allow a later material change to overwrite an explicit edit. The [workflow history contract](../gui-workflow/spec.md#edit-history) owns source-replacement and complete-state-load boundaries.

Each accepted edit, undo, or redo MUST receive a new render revision. The revision counter MUST NOT be restored from history. Existing worker result guards remain authoritative.

A pending result for an abandoned intermediate state MUST NOT replace the accepted output of a newer history state.

## Search and disclosure state

Search MUST inspect control descriptions without invoking editor mutations, material loading, or rendering. It MUST include inactive and folded controls in the index. A locator for another surface MUST open that owner and focus its control rather than constructing a filtered editor copy.

Normal disclosure state and query-induced disclosure MUST be separate. Clearing a query MUST remove only the query-induced expansion. Only modified MUST read the same factory comparison as row markers, not a separately maintained dirty flag.

Search text, temporary result selection, and adjustment history remain session-local. Browser metadata and any persisted layout values retain their existing GUI-state ownership. Inspector organization MUST NOT leak into look presets, runtime recipes, CLI parameter documents, or Python-compatible GUI sections.

## Retained output and delivery

The viewer MUST continue to use its existing presentation and composition workers. A material edit MUST NOT directly filter, recolor, or swap the displayed image to simulate a result. Until an accepted render completes, the application keeps the last accepted frame and identifies it as not current when appropriate.

Preview, Scan, Save, and Export MUST retain their existing operation boundaries. Export draft controls MUST NOT mutate the live render configuration. Submitting Export captures one validated immutable request.

Later edits and history navigation MUST NOT change that request. Save MUST consume retained output, never a prototype image or a display raster.

Library writes, startup-default writes, diagnostics, and output files retain their own transactional owners. Adjustment Undo MUST NOT claim to reverse these side effects. Global factory restoration MUST retain its existing confirmation or explicitly reversible-state contract; module reset does not delete a startup-default file.

## Cutover and acceptance boundary

The native cutover MUST remove the old mutually exclusive page navigation rather than keep it as a compatibility UI. Existing state documents and preset documents MUST retain their parameter meanings. Obsolete page-selection metadata MUST NOT choose a hidden legacy editor; layout adaptation MUST NOT change accepted controls or persisted preset scope.

The implementation MUST integrate the inspector, preset browser search, editing/history boundary, settings and Export owners, and control coverage in the production entry point. A static HTML page, duplicated test-only editors, or an inventory with disconnected rows does not satisfy the target.

Verification MUST cover actual parameter transitions, factory reset scope, preset scope, history provenance, stale-worker rejection, and retained Save versus independent Export. Native layout and keyboard review MUST complement existing offscreen coverage. Image and backend checks MUST use the owning numerical contracts; layout work MUST NOT change those tolerances or claim new rendering parity.
