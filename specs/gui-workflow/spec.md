# GUI workflow
## Presentation language

The [GUI presentation specification](../gui-presentation/spec.md) owns visual hierarchy, semantic tokens, component states, layout rhythm, feedback, and accessibility presentation rules. This specification remains authoritative for workflow behavior, stage and surface ownership, control inventory, editing behavior, persisted-state behavior, and viewer semantics. Visible labels and help text follow [GUI product language](../gui-language/spec.md).

## Contract

- The GUI exposes controls for input, film, print, scanner, output, advanced color, and configuration.
- Preview renders a bounded disposable image. Scan renders the original working resolution. Both use the current runtime controls.
- Save writes retained output. Export re-renders through the selected backend and exposes cancellation. Render, saving-conversion, and viewer execution follow [GPU execution](../gpu-execution/spec.md); selecting CPU does not imply a precision different from the actual build.
- Startup controls may be saved and restored. Removing the startup default restores the factory profile.
- Viewer-only state, such as zoom, interpolation, reveal, and canvas presentation, must not alter saved or exported pixels.

- The Scanner controls show exactly the controls that
  [Scan medium and scanner controls](../film-chain/spec.md#scan-medium-and-scanner-controls)
  assign to the scanned medium. For a negative film profile on the direct
  `input > film > scan` route or the real negative-capture `input > convert-film > scan` route, they offer `direct_scan` and
  `negative_to_positive`. Labels must follow the [scan output naming contract](../film-chain/spec.md#scan-output-naming-acceptance). Every route shows the scanner lamp, lens blur, and unsharp mask. Conversion control display names must follow the [conversion control naming contract](../film-chain/spec.md#conversion-control-naming-acceptance).
  A route or film profile change that makes the selected mode ineligible resets it to
  `direct_scan` and clears ineligible scanner controls.
- Imported states and presets must reject the removed mode as defined by [Scanner language](../film-chain/spec.md#scanner-language). A rejected import must not replace accepted parameters or the retained image.
- Numeric parameter fields use text editing and the default text cursor. Each original wheel event over a field changes its value by the declared step. Smoothed scroll residue must not repeat that change in later frames. Invalid or incomplete text must not enter runtime parameters.
- A numeric editor must offer a usable range and step, and must not accept a value the parameter validator rejects. [Numeric editor ranges](#numeric-editor-ranges) owns both rules.
- Preview computation and viewer composition run outside the UI thread. Queued requests retain only the latest snapshot. Results from older requests, input images, or parameter revisions must not replace current output.
- The viewer retains the last completed frame while new composition runs. The development animation is fixed viewer behaviour under [Viewer presentation](#viewer-presentation).
- Base workflow selection and optional CMYK ink style controls must follow the [CMYK ink style enablement contract](../cmyk-style/spec.md#enablement).
- The workflow selector must offer exactly the routes the workflow validator supports.
- Input and capture → Camera exposes `camera.capture_illuminant` as a single optional text or selector control that writes the canonical runtime leaf. Its semantics, inheritance, and validation follow the [capture illuminant contract](../image-input/spec.md#capture-illuminant). An unset value must be selectable and must display the inherited value — the selected film profile's `reference_illuminant` — as the effective value, so the control never hides which light the render assumes. The control must not offer a value the shared illuminant validator rejects and must not derive a value from image content or RAW metadata. It must be labeled with the product concept, for example `Capture illuminant`, and must be distinguished from the enlarger illuminant in Print → Enlarger and the scanner lamp in Scan → Scanner. A route that computes no film exposure from scene RGB must state that the control is inactive and retain its stored value. The control and the canonical CLI `--set` path must share one type, validator, and effect conditions.
- The capture illuminant control belongs to the Camera section. It must not be duplicated on another surface, and no second surface may edit the same leaf.

## Material controls and output composition

Control placement follows [Stage and surface ownership](#stage-and-surface-ownership). Material behavior follows [Output mix and material controls](../output-mix/spec.md#user-operation). The inspector must not add Digital response panels, a Reproduction section, saturation or color-density groups, or tonal-mask controls.

Inspector stages organize access to controls; they do not change pipeline execution order or serialized preset ownership.

Print must expose print exposure mode, print exposure, Magenta and Yellow shifts, and preflash exposure with its independent Magenta and Yellow filter shifts. Print exposure mode must follow [Print exposure mode](../film-chain/spec.md#print-exposure-mode). Calibrated cyan remains part of the neutral filter pack without a new Cyan shift control. Preflash must retain its existing units, defaults, bounds, and preset ownership under [Preflash](../output-mix/spec.md#preflash). Zero preflash exposure must disable flash without clearing its filters. A route without print exposure and development must identify these controls as inactive and retain their values.

Preflash help must explain that, on negative-to-photographic-paper printing, it can bring weak highlight exposure differences into a responsive part of the paper curve. It cannot recover detail absent from the negative. Excessive preflash can lower paper white, add fog, alter midtones, and change color. Help must not promise fixed white or midgray, improved detail at every setting, or equivalence to digital highlight compression. Printing-film routes must retain their own material polarity and must not inherit a paper-highlight promise.

Film and Print must retain their existing chemistry and development controls. Developer exhaustion must keep its model-specific development meaning and capability conditions. It must not be renamed as a generic highlight or high-dynamic-range control. Profile and route feedback must identify unsupported or inactive controls without clearing stored values or fabricating model support. Film-development controls are inactive when converting a real captured negative; print-development controls remain available only on admitted printing routes. The `input` workflow must mark material and Scanner controls inapplicable.

Material edits must use existing scanner calibration and inverse reconstruction semantics. The GUI must not promise creative-reference isolation or add separate baseline and effective reference controls. Module and row resets follow [Section reset](#section-reset). Section presets retain their serialized ownership; no response reset or preset section is added.

Output must expose Simulation blend as the full-result/prepared-source blend under [Output mix](../output-mix/spec.md#output-mix), with its existing section menu. Existing output gamut algorithm and chroma-knee controls remain available without duplicate editable copies. Output must not expose standalone lightness compression, an Output response section or preset, or a separate output-processing switch for the `input` workflow. A negative-capture source must state that zero Simulation blend returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent from rendered effect amount.

Loading retired settings must follow [Output boundary and retirement](../output-mix/spec.md#output-boundary-and-retirement). A rejected document must identify the exact offending path and leave the source document, active controls, accepted render identity, and retained output unchanged. The GUI must require explicit removal of a retired numeric lightness triplet without suggesting withdrawn response groups or automatically substituting preflash or chemistry. Withdrawn draft response groups and cyan-shift paths must fail normal unknown-field validation without aliases or migration.

## Window layout

The window has three regions and one header.

- The viewer occupies the central region and is the primary surface. It presents the retained input, output, or paper back under [Viewer presentation](#viewer-presentation), and its footer owns the layer selector.
- The adjustment inspector occupies the right region. It owns photographic parameter editors, stage headings and their jump strip, control search, and the tonal-distribution readout. Application settings and Export own their separate editors under [Stage and surface ownership](#stage-and-surface-ownership).
- The preset browser occupies the left region and is collapsible. Its width and expanded state follow [the preset browser design](../presets/design.md).
- The window header spans the three regions. It owns Open image, Undo, Redo, application settings, and independent Export actions. It shows the accepted source, current film profile, print profile, and physical workflow as read-only context. It is the only operation-status surface. Preview, Scan, and Save remain reachable from a persistent action area without opening an inspector module.

A region is a layout slot, not a mutually exclusive page. A control belongs to exactly one surface under [Labels and surfaces](#labels-and-surfaces). A dialog may edit only the values assigned to it.

## Optical and grain labels

Scan → Scanner must distinguish Lens blur (readout optics) from Unsharp mask (digital sharpening). Tooltips must explain the [optical-order cutover](../film-chain/spec.md#optical-order-cutover) without offering a legacy late-blur mode.

Scan must expose `print_render.glare` in a module labeled Print viewing glare. Its description must identify readout stray light, not preflash or material density formation. Its canonical paths and `print` preset scope remain unchanged. Preflash and Enlarger diffusion remain separate Print modules for exposure and optics.

The existing grain selector must retain its stable engine names and preset section. It must identify Spektra, Dye, and Fotufilm Film as material grain and Procedural as post-scan grain appearance. Selecting Procedural must describe its Film Resolution control as part of that appearance algorithm, not as scanner lens resolution. No duplicate grain selector or automatic engine conversion is required.

## Stage and surface ownership

Each module has one editing owner. The inspector must contain these stages and modules. A less frequently used control stays with its stage; frequency must not move it to a separate Advanced page.

| Owner | Modules and controls |
| --- | --- |
| Input and capture | Workflow selector; input color space and decoding; RAW white balance, temperature, tint, lens correction, and reprocess action; crop, upscale, and source rotation; Camera exposure, metering, film format, color filter, and capture illuminant; Camera diffusion; input gamut compression; spectral reconstruction and its applicable adaptation controls; Convert exposure, base estimation, calibration, channel swaps, and calibration actions. |
| Film | Film material selector and its development-time family; Chemistry; Film base; Halation; Couplers; Grain selector, shared material-grain parameters, and every engine-specific parameter group. |
| Print | Print material selector and its development-time family; Enlarger exposure mode, exposure, exposure tracking, M/Y shifts, and illuminant; Preflash; Enlarger diffusion; print Chemistry and independent density-curve gamma; Print base. |
| Scan | Scanner lamp, eligible scan output and conversion controls, lens blur, unsharp mask; Print viewing glare. |
| Output | Output color space and encoding where exposed by their contracts; output gamut processing; CMYK ink style; Simulation blend; retained Save color and encoding settings. |
| Application settings | Auto preview and preview limits; rendering preferences that are not photographic controls; startup state save/load/default actions; display transform, ICC selection, canvas, padding, and interpolation; Diagnostics and report actions. |
| Export dialog | Independent full-resolution export backend, format, depth, saving color and encoding, applicable codec/compression options, destination selection, and export submission. |
| Viewer footer | Input/Output/Paper back selection; zoom, fit, comparison and its split, reveal, and read-only zoom state. |
| Preset browser | Look selection, browser search, library refresh/reveal, and look-preset management. |

Rotation is a source operation and must not be offered as viewer-only pan or zoom. A read-only summary or action that locates its owner may appear elsewhere; a second editable copy must not.

The stage jump strip must reveal and scroll to one heading in the continuous inspector. It must not close other stages or change any parameter. A heading toggle must affect only its own open state. Open state and scroll position must survive ordinary edits, render completion, and route changes.

The [preset section table](../presets/design.md#sections) remains authoritative for serialized section payloads and their menu anchors. A visual module is not a new serialized section. Moving Print viewing glare to Scan or splitting Preflash from Enlarger must not migrate their canonical leaves or expand a preset's scope.

## Complete control access

Every supported user-editable parameter must have one visible editor or a named action that writes its canonical value. Coverage includes non-numeric, boolean, optional, vector, engine-specific, and currently inactive controls. A count copied from a prototype is not a coverage definition.

The application must maintain an inspectable inventory that maps each editable canonical path to its stage, module or dialog, label, editor or action, validation metadata, and applicability condition. Parameter discovery and existing preset-owned paths define the required set. Viewer and session-only controls must be identified separately from render parameters. Each editable vector component must be covered. A deliberately derived component must be read-only and explain its derivation.

The GUI must expose a searchable control index that can locate each inventory entry at its real editing owner. The index must not contain parallel editors. Actions such as RAW reprocessing, calibration, feature-profile application, and file writing must be listed as actions, not fabricated numeric or boolean parameters.

Calibration baselines, prepared values, no-op leaves, removed parameters, and internal state must not be promoted to creative controls. The inventory must distinguish such exclusions from supported controls that have not yet been connected. A missing editor for a supported control is a defect, not an exclusion.

Halation must expose `active`, `scatter_amount`, `scatter_spatial_scale`, `halation_amount`, `halation_spatial_scale`, `source_compression`, `boost_ev`, `protect_ev`, `boost_range`, `scatter_core_um`, `scatter_tail_um`, `scatter_tail_weight`, `halation_strength`, `halation_first_sigma_um`, `halation_n_bounces`, `halation_bounce_decay`, and `halation_renormalize` under `film_render.halation`. Each editable RGB component must remain available. Its feature-profile selector follows [Feature profiles](../feature-profiles/spec.md#halation-profile-family); it must not add a persisted Halation profile identity or replace the individual controls.

Preflash must expose exactly its existing `enlarger.preflash_exposure`, `enlarger.preflash_y_filter_shift`, and `enlarger.preflash_m_filter_shift` controls. Main enlarger M/Y controls remain independent. A visual off state must derive from zero exposure rather than a new enable parameter.

All supported Grain engines must retain their independent control sets and shared selector. Shared material parameters must have one editing location. A named Procedural profile and Custom must obey [Feature profiles](../feature-profiles/spec.md#procedural-grain-profile-family), including inherited optional values. The interface must not impose a blanket Custom-only restriction on a parameter whose owning contract allows a named-profile override.

Module disable, unsupported material capability, source type, route, and selected engine are different applicability reasons. The GUI must name the actual reason and retain readable values. Expanding an inactive group must not activate it. A route or material change must not erase inactive values except for normalizations explicitly required by the owning contract, such as scanner-output eligibility; the header must report those normalizations.

`settings.print_exposure_ev_tracking` belongs to Print → Enlarger because it controls exposure behavior. Its `settings` wire group must not move it to application preferences or remove its existing look ownership.

Inherited optional values must show both their inherited state and effective value. Entering an explicit value and returning to inheritance must remain distinct operations. Development-time controls must use the selected material's actual family rather than invented common time choices.

Rows and modules expose factory resets under [Section reset](#section-reset). Control coverage acceptance must exercise every inventory owner and every engine branch through production GUI controls, with applicable and inapplicable states. It must check canonical values and component independence, not only count widgets.

## Section reset

A visible module reset must restore exactly that module's editable canonical paths to their projected factory values. An individual reset must restore exactly its canonical leaf; a vector leaf resets its editable components together. Neither operation may reset another module because the modules share a serialized preset section. In particular, Preflash reset must leave main Enlarger exposure and filtration unchanged, and Print viewing glare reset must leave Print chemistry unchanged.

A factory projection must include runtime-only editable leaves and preserve each leaf's type and optional state. It is not the selected material's suggested values, the last applied preset, or the prototype's example configuration. The row marker, module marker, Only modified filter, and reset action must use the same projection and comparison rule under [GUI presentation](../gui-presentation/design.md).

The reset action must name its scope and baseline. A module's section-preset menu must name its different scope when it covers more controls than the visible module. Reset and Revert to applied preset must remain separate actions. Neither reset may change a preset file or startup default. A reset is one reversible edit; normal validation must complete before it changes accepted controls.

## Edit history

Undo and Redo apply to accepted adjustments for the current source: direct parameter edits, workflow and material selection, feature-profile and section/look-preset application, and individual or module reset. Each action includes its required dependent normalizations. A rejected edit creates no history entry.

One slider drag is one history entry, even if intermediate valid values schedule previews. A completed numeric text edit is one entry. One original wheel event over a numeric field is one entry. Incomplete or invalid text must not change accepted values or history. Escape must cancel the current uncommitted edit. Canceling an active drag must restore its pre-gesture values without an undo entry.

Undo must restore the preceding accepted values, profile selections, workflow, explicit-leaf ownership, applied-preset marker, and affected transient rendering state. Redo must restore the recorded successor, not reapply a possibly changed preset file or recompute stock defaults. A new accepted edit after Undo must discard the redo branch. Factory-modified and preset-modified indicators must be recomputed against their own baselines.

History must not own image buffers, viewer transforms, library files, startup defaults, diagnostics, or completed Save/Export artifacts. Library mutations and file writes must not be presented as reversible by adjustment Undo. Application settings and an unsubmitted Export draft remain outside adjustment history. Export draft cancellation restores the draft boundary without rewriting current adjustments.

Accepting a replacement source, completing RAW reprocessing, or loading a complete GUI state must start a new adjustment history for that accepted context. A failed import, reprocess, or state load must retain the preceding history and context. Loading a GUI state must not serialize or reconstruct an old undo stack.

Undo and Redo must advance render revisions and use the same validation, invalidation, and Auto preview rules as other accepted edits. They must not rewind revision counters or publish an old result as current. In-flight Export retains its captured configuration; adjustment history must not mutate it.

The header actions must name the next undoable or redoable operation and be disabled when unavailable. Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z must operate on adjustment history outside a text editor. While a text editor owns focus, its text-editing shortcuts must not unexpectedly undo a photograph adjustment.

## Numeric editor ranges

An editor range is a usability bound. It is not the parameter's document bound.

Each numeric editor must declare a minimum, a maximum, and a step. The parameter discovery description must declare the same usable range. That usable range may be narrower than the accepted document range. The editor must accept every value inside the usable range and must reject values outside it.

The parameter validator owns the accepted document range. Recipes, presets, CLI parameters, and saved state must carry every value the validator accepts, including values outside the editor range.

An editor must reject input that the validator would reject, before the value enters runtime parameters. The editor must not accept a value that fails at render time.

The GUI editor reads the single declared range for `camera.film_format_mm` from the editor-range table. Recipes, presets, CLI parameters, and saved state must carry every strictly positive value accepted by the validator, including values outside the editor range. The [Film geometry](../film-grain/v3/design.md#film-geometry) contract owns the Dye-specific geometry consequence.

The editor range is declared once per parameter path in a single table. The GUI editor reads that table; a missing entry is a defect, not a silent default to the machine range. The same table is the source for the CLI discovery `minimum`/`maximum` output. The editor range must be contained within the validator's accepted range and must be wide enough to reach the parameter's documented usable values. Where the validator bound is unbounded above, the editor must still declare a finite maximum derived from the parameter's typical operating range.

## Viewer presentation

The viewer presents the retained input, the retained output, or the paper back. Its presentation state is viewer-only under the [GUI workflow design](design.md).

The viewer presents an explicit empty state when no image is loaded. The empty state must name the import action and must not present a blank canvas. It is viewer-only and must not alter saved or exported pixels.

The viewer footer owns the layer selector. It is the only surface that exposes it. Application settings must not repeat it.

The development animation is fixed viewer behaviour. It must not have an editor, and it must not be persisted as an editable control. An automatic parameter update skips it; an explicit Preview or Scan retains it.

The viewer must let the user select a display ICC profile. The selected profile applies to the disposable viewer raster only. Windows discovers the primary display profile when the user selects none. An explicitly selected profile applies on every platform that supports LittleCMS. Without a usable profile, the encoded sRGB preview is used directly.

The viewer must expose a tonal-distribution readout of the retained output. It must be viewer-only: it must not alter saved or exported pixels.
- The readout is a fixed strip at the top of the control panel, above search and stage navigation. It is co-visible with the controls, does not occlude the image, and does not increase the control panel width. It is default-on, collapsible, and its visibility state is persisted. It is viewer-only and never affects saved or exported pixels.

The readout must derive from the retained output. It must not require a second render.

An empty or absent output must present an explicit empty state instead of a blank or broken readout.

## Viewer interaction

Viewer gestures act on the viewer surface. They change the retained view and must not alter saved or exported pixels. They must not edit numeric fields and must not scroll the control panel.

- The wheel and pinch zoom at the pointer.
- A primary-button drag pans the view.
- A Shift or secondary-button drag selects a rectangle. Its release zooms so the selection fills the viewer and its center lands at the viewer center.
- One click zooms one step at the pointer.
- A double click resets the view to the fit zoom and the centered pan.

A gesture that arrives while a composition runs must keep the last completed frame until the current frame is ready.

## Experimental controls

The following controls carry an experimental marker. The marker is a presentation cue; it does not change the parameter's behavior or validation.

- `print illuminant` selector — Print → Enlarger
- Input gamut-compression algorithm selector — Input and capture → Input gamut compress
- Spektra `rms granularity` — Film → Grain
- `film_channel_swap`, `print_channel_swap` — Input and capture → Convert

The channel-swap validator must reject a non-permutation. A value such as `[0, 0, 0]` currently passes validation and silently duplicates one density channel while dropping the other two. The shared renderer applies the swap to `profile.data.channel_density`, so a non-permutation corrupts the profile at render time. The validator must require exactly three indices in `0..2` that form a permutation of `0, 1, 2`.

The GUI must not present a selector that offers one choice.
A parameter that no control can change and that no pipeline stage reads must not own a GUI section under the [preset section rule](../presets/design.md). Every parameter carried by a look or section preset must have one visible GUI control or named action that writes the canonical runtime leaf; inactive conditions retain the value and explain why editing is disabled. GUI controls and canonical CLI `--set` paths must share the same type, validator, units, and conditions. Calibration baselines, derived values, and internal state are not preset-owned controls.

## Control search

The inspector must expose one always-visible search field. Matching must use case-insensitive substrings of the visible control label, module title, and canonical parameter path. Search must cover all stages and the separately owned application, viewer, and Export controls, including collapsed and inactive entries.

Within the inspector, search must reveal matching rows under their owning stage and module headings. A matching module title reveals its controls. Nonmatching modules may be hidden while the query is active. Matches in application settings, Export, or the viewer must be locator results that name and open the real owner; they must not duplicate editors or filter the destination's unrelated controls.

An inactive match must retain its value and explanation. Search must not change the route, selected engine, enable state, preset, or parameter value to make a match editable. A search result may expand and focus the owner's control without committing an edit.

The search field must provide a clear action and an explicit no-match state that includes the query. Search and its temporary disclosure state must be session-local. Clearing the query must restore the preceding normal open state and must not hide controls previously available without search.

Only modified is an independent, reversible view filter against the factory projection in [Section reset](#section-reset). It combines with search by intersection. Its no-match state must distinguish no modified controls from no search matches. Turning it off must restore the ordinary inventory. A preset's modified marker must not be used as the filter predicate.

Preset browser search is a separate operation owned by [Presets](../presets/spec.md). Neither query may alter the other surface's filter or selection.

## Labels and surfaces

A label must name the current product concept. A label must not name a removed implementation, a development tool, or a file format of a former dependency.

An operation status must appear on exactly one surface, the window header. It must be short, under the [telemetry status rule](../telemetry/spec.md#local-surfaces). While Preview or Scan runs, it names the operation and the work unit that executes now. An export reports its own operation status with its cancellation states. When no operation runs, it describes the last accepted operation.

A control that changes a persisted value must be reachable from the GUI. A persisted value with no control, no pipeline consumer, and no owning specification must be removed from the document.

Two controls that edit the same persisted value must not both be editable. When two surfaces must show one value, one is read-only.

Two different parameters that share a label must carry a qualifier that distinguishes them.

