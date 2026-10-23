# GUI product design implementation boundaries

## Authority

[GUI product design](spec.md) defines the user-visible workspace. [GUI workflow](../gui-workflow/spec.md) owns stage and surface assignment, complete control access, reset scopes, edit history, and worker invariants. This document owns the composition of those boundaries. It must not define another parameter schema, preset resolver, or render pipeline.

## Workspace composition

The native application must compose the existing preset browser, viewer, and control panel into the three-region workspace. It must reuse the existing toolkit and [presentation components](../gui-presentation/design.md).

The right panel must contain one continuous stage inspector rather than one control tree per tab. Stage jump actions and collapsible headings must address the same module instances. Application settings and Export dialogs must render only their assigned owners. Closing or opening a dialog must not create another live editor for the same value.

A read-only material summary, a preset card, and a search result may identify a value. They must not become independent editing models. The viewer must continue to consume retained image state, not inspector widget state.

Independent module disclosure is preferred to basic/expert modes because it preserves a single complete control inventory. Stage jumps are preferred to retaining both old tabs and a new sidebar because a control needs one stable location. These choices do not change pipeline stage execution or preset serialization scope.

## Control inventory and shared metadata

One GUI-owned control inventory must bind canonical paths to their visual owners, supported editor or named action, and applicability presentation. It must reuse existing typed field descriptions, validators, units, ranges, and feature-profile resolvers. It must not copy numeric defaults, bounds, enum lists, or profile tables from the web prototype.

The inventory must supply search locations and the control index. The module-to-path projection used by reset and modified detection must follow [GUI presentation design](../gui-presentation/design.md). Parameter discovery defines public parameter semantics; the GUI inventory defines only where and how an accepted control is presented.

Composite controls must bind every component to its canonical leaf and component identifier. An aggregate editor must not silently couple independent components. Derived components must reuse the owning derivation and validation, not a second GUI approximation. Optional editors must distinguish an explicit value from inheritance and display the effective inherited value.

UI applicability must come from route, source, material capability, engine, and feature conditions. A shared condition must not be independently reimplemented by row rendering, search, and reset. The view must distinguish an unavailable editor from an absent control. Inactive conditions must not mutate parameters during rendering.

## Module scope versus preset scope

A visual module owns its displayed canonical paths. A serialized preset section owns the path set in [Preset design](../presets/design.md#sections). These sets need not be identical.

Preflash illustrates the boundary: its module owns three leaves, while the Enlarger section preset retains its broader serialized payload. Print viewing glare is displayed under Scan while its preset ownership remains in `print`. The module reset must use the visible-module projection; section-preset capture and application must use the core preset scope.

One primary menu anchor must represent each serialized section. A menu whose scope crosses module or stage boundaries must state those affected modules. The application must not create duplicate menus, change serialization keys, or expand capture because the inspector layout changed.

Feature-profile actions must use the shared feature resolver. They are scoped edit operations rather than new preset documents or GUI state fields. The Halation selector derives its effective named or custom state from its two scoped arrays. Procedural keeps its existing stored profile and override semantics.

## Accepted edits and history

All photographic editors and actions must enter the application's existing validated-edit boundary. A candidate edit must be normalized and validated before publication. One accepted transaction must include the edited values, dependent material/route/scanner changes, explicit-leaf provenance, applied-preset identity, and any affected transient rendering state. Rejection must leave all of them unchanged.

The application must capture a transaction's pre-edit and accepted post-edit control state. History must store control changes and the small associated state needed to restore them, not cloned images, profile spectral arrays, prepared runtimes, or export jobs. The implementation may reuse existing immutable material references.

Slider input may publish intermediate valid control states for interactive preview. The gesture must retain one pre-edit boundary and create one history entry at completion. Cancellation must restore that boundary through ordinary invalidation. Numeric text keeps a separate editing buffer until the value is accepted. A view rebuild must preserve the focused canonical field and its uncommitted buffer.

Undo and Redo must apply recorded accepted transactions without reading a changed preset file or rerunning stock initialization. They must restore provenance along with numeric values; restoring values alone can allow a later material change to overwrite an explicit edit. The [workflow history contract](../gui-workflow/spec.md#edit-history) owns source-replacement and complete-state-load boundaries.

Each accepted edit, undo, or redo must receive a new render revision. The revision counter must not be restored from history. Existing worker result guards remain authoritative. A pending result for an abandoned intermediate state must not replace the accepted output of a newer history state.

## Search and disclosure state

Search must inspect control descriptions without invoking editor mutations, material loading, or rendering. It must include inactive and folded controls in the index. A locator for another surface must open that owner and focus its control rather than constructing a filtered editor copy.

Normal disclosure state and query-induced disclosure must be separate. Clearing a query must remove only the query-induced expansion. Only modified must read the same factory comparison as row markers, not a separately maintained dirty flag.

Search text, temporary result selection, and adjustment history remain session-local. Browser metadata and any persisted layout values retain their existing GUI-state ownership. Inspector organization must not leak into look presets, runtime recipes, CLI parameter documents, or Python-compatible GUI sections.

## Retained output and delivery

The viewer must continue to use its existing presentation and composition workers. A material edit must not directly filter, recolor, or swap the displayed image to simulate a result. Until an accepted render completes, the application keeps the last accepted frame and identifies it as not current when appropriate.

Preview, Scan, Save, and Export must retain their existing operation boundaries. Export draft controls must not mutate the live render configuration. Submitting Export captures one validated immutable request. Later edits and history navigation must not change that request. Save must consume retained output, never a prototype image or a display raster.

Library writes, startup-default writes, diagnostics, and output files retain their own transactional owners. Adjustment Undo must not claim to reverse these side effects. Global factory restoration must retain its existing confirmation or explicitly reversible-state contract; module reset does not delete a startup-default file.

## Cutover and acceptance boundary

The native cutover must remove the old mutually exclusive page navigation rather than keep it as a compatibility UI. Existing state documents and preset documents must retain their parameter meanings. Obsolete page-selection metadata must not choose a hidden legacy editor; layout adaptation must not change accepted controls or persisted preset scope.

The implementation must integrate the inspector, preset browser search, editing/history boundary, settings and Export owners, and control coverage in the production entry point. A static HTML page, duplicated test-only editors, or an inventory with disconnected rows does not satisfy the target.

Verification must cover actual parameter transitions, factory reset scope, preset scope, history provenance, stale-worker rejection, and retained Save versus independent Export. Native layout and keyboard review must complement existing offscreen coverage. Image and backend checks must use the owning numerical contracts; layout work must not change those tolerances or claim new rendering parity.
