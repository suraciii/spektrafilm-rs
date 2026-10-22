# GUI workflow
## Presentation language

The [GUI presentation specification](../gui-presentation/spec.md) owns visual hierarchy, semantic tokens, component states, layout rhythm, feedback, and accessibility presentation rules. This specification remains authoritative for workflow behavior, page ownership, control inventory, editing behavior, persisted-state behavior, and viewer semantics. Visible labels and help text follow [GUI product language](../gui-language/spec.md).

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
  `negative_to_positive`. Labels must follow the [scan output naming contract](../film-chain/spec.md#scan-output-naming-acceptance). Exposure reconstruction additionally shows brightness,
  three-channel balance, contrast, maximum density, and negative conversion
  method. Conversion control display names must follow the [conversion control naming contract](../film-chain/spec.md#conversion-control-naming-acceptance). A slide film scan and a reconstructed negative film scan show
  the per-channel black and white points. Film scans show receiver profile, scanner finish, finish strength, and the `off`, `density`, `density_color`, or `full` setup mode where eligible. The legacy `per_frame` spelling is accepted only when loading compatible state and is serialized canonically as `full`.
  The GUI identifies approximate receiver profiles as approximate. A route or
  film profile change that makes the selected mode ineligible resets it to
  `direct_scan` and clears ineligible scanner controls.
- Selecting `negative_to_positive` clears scanner white and black correction flags.
  Hidden capture-correction settings must not prevent this mode selection.
  State and preset validation still reject an explicitly supplied incompatible
  scan configuration.
- Imported states and presets must reject the removed mode as defined by [Scanner language](../film-chain/spec.md#scanner-language). A rejected import must not replace accepted parameters or the retained image.
- Scanner setup controls must expose whether the current setup is disabled or
  derived for the current input revision. Preview, Scan, and Export must use
  the same accepted setup for one input and parameter revision.
- Digital Reference selection, labels, control visibility, and compatible receiver pairing follow the [Digital Reference contract](../film-chain/digital-reference/spec.md#integration-and-control-ownership). Its fixed Reference Exposure placement exposes reference output gain and balance while hiding exposure-reconstruction controls and automatic setup.
- Numeric parameter fields use text editing and the default text cursor. Each original wheel event over a field changes its value by the declared step. Smoothed scroll residue must not repeat that change in later frames. Invalid or incomplete text must not enter runtime parameters.
- A numeric editor must offer a usable range and step, and must not accept a value the parameter validator rejects. [Numeric editor ranges](#numeric-editor-ranges) owns both rules.
- Preview computation and viewer composition run outside the UI thread. Queued requests retain only the latest snapshot. Results from older requests, input images, or parameter revisions must not replace current output.
- The viewer retains the last completed frame while new composition runs. The development animation is fixed viewer behaviour under [Viewer presentation](#viewer-presentation).
- Base workflow selection and optional CMYK ink style controls must follow the [CMYK ink style enablement contract](../cmyk-style/spec.md#enablement).
- The workflow selector must offer exactly the routes the workflow validator supports.
- MAIN → Camera exposes `camera.capture_illuminant` as a single optional text or selector control that writes the canonical runtime leaf. Its semantics, inheritance, and validation follow the [capture illuminant contract](../image-input/spec.md#capture-illuminant). An unset value must be selectable and must display the inherited value — the selected film profile's `reference_illuminant` — as the effective value, so the control never hides which light the render assumes. The control must not offer a value the shared illuminant validator rejects and must not derive a value from image content or RAW metadata. It must be labeled with the product concept, for example `Capture illuminant`, and must be distinguished from the enlarger illuminant in MAIN → Enlarger and the scan illuminant in FILM → Convert. A route that computes no film exposure from scene RGB must state that the control is inactive and retain its stored value. The control and the canonical CLI `--set` path must share one type, validator, and effect conditions.
- The capture illuminant control belongs to the Camera section. It must not be duplicated on another page, and no second surface may edit the same leaf.

## Material controls and output composition

Control placement and process meaning must follow [Output mix and material controls](../output-mix/spec.md#user-operation). The GUI must retain the existing MAIN, FILM, PRINT, ADVANCED, and CONFIG pages and section menus. It must not add Digital response panels, a Reproduction section, saturation or color-density groups, or tonal-mask controls.

Each page owns one stage of the workflow. [Page ownership](#page-ownership) assigns every section to exactly one page.

MAIN Enlarger must expose print exposure mode, print exposure, Magenta and Yellow shifts, and preflash exposure with its independent Magenta and Yellow filter shifts. Print exposure mode must follow [Print exposure mode](../film-chain/spec.md#print-exposure-mode). Calibrated cyan remains part of the neutral filter pack without a new Cyan shift control. Preflash must retain its existing units, defaults, bounds, and section ownership under [Preflash](../output-mix/spec.md#preflash). Zero preflash exposure must disable flash without clearing its filters. A route without print exposure and development must identify these controls as inactive and retain their values.

Preflash help must explain that, on negative-to-photographic-paper printing, it can bring weak highlight exposure differences into a responsive part of the paper curve. It cannot recover detail absent from the negative. Excessive preflash can lower paper white, add fog, alter midtones, and change color. Help must not promise fixed white or midgray, improved detail at every setting, or equivalence to digital highlight compression. Printing-film routes must retain their own material polarity and must not inherit a paper-highlight promise.

FILM and PRINT must retain their existing chemistry and development controls. Developer exhaustion must keep its model-specific development meaning and capability conditions. It must not be renamed as a generic highlight or high-dynamic-range control. Profile and route feedback must identify unsupported or inactive controls without clearing stored values or fabricating model support. Film-development controls are inactive when converting a real captured negative; print-development controls remain available only on admitted printing routes. The `input` workflow must mark material and Scanner controls inapplicable. Existing Scanner black and white points retain their medium-specific meaning; the GUI must not add duplicate generic levels.

Material edits must use existing scanner calibration, inverse reconstruction, and automatic setup semantics. The GUI must not promise creative-reference isolation or add separate baseline and effective reference controls. Resets and section presets must retain existing ownership without new response resets or preset sections.

MAIN Output must expose Simulation blend as the full-result/prepared-source blend under [Output mix](../output-mix/spec.md#output-mix), with its existing section menu. Existing output gamut algorithm and chroma-knee controls remain available without duplicate editable copies. Output must not expose standalone lightness compression, an Output response section or preset, or a separate output-processing switch for the `input` workflow. A negative-capture source must state that zero Simulation blend returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent from rendered effect amount.

Loading retired settings must follow [Output boundary and retirement](../output-mix/spec.md#output-boundary-and-retirement). A rejected document must identify the exact offending path and leave the source document, active controls, accepted render identity, and retained output unchanged. The GUI must require explicit removal of a retired numeric lightness triplet without suggesting withdrawn response groups or automatically substituting preflash or chemistry. Withdrawn draft response groups and cyan-shift paths must fail normal unknown-field validation without aliases or migration.

## Window layout

The window has three regions and one header.

- The viewer occupies the central region and is the primary surface. It presents the retained input, output, or paper back under [Viewer presentation](#viewer-presentation), and its footer owns the layer selector.
- The control panel occupies the right region. It owns the page tabs, the control search, the tonal-distribution readout, and the primary actions. It is the only surface that edits parameter values.
- The preset browser occupies the left region and is collapsible. Its width and expanded state follow [the preset browser design](../presets/design.md).
- The window header spans the full width above the three regions. It shows the current film profile, print profile, and workflow route as read-only text, and it is the only surface that shows the operation status.

A region is a layout slot, not a page. A control belongs to exactly one region, and a persisted value keeps exactly one editable surface under [Labels and surfaces](#labels-and-surfaces).

## Optical and grain labels

MAIN Scanner must distinguish Lens blur (readout optics) from Unsharp mask (digital sharpening). Identity point controls must not imply a different optical order. Tooltips must explain the [optical-order cutover](../film-chain/spec.md#optical-order-cutover) without offering a legacy late-blur mode.

PRINT may retain the `print_render.glare` controls in its existing preset section, but their visible group must be labeled Print viewing glare. Its description must identify readout stray light, not preflash or a change in material density. Preflash and Enlarger diffusion must remain separately labeled exposure and optical controls. Group location must not imply that all Print controls execute during development.

The existing grain selector must retain its stable engine names and preset section. It must identify Spektra and Dye as material grain and Procedural as post-scan grain appearance. Selecting Procedural must describe its Film Resolution control as part of that appearance algorithm, not as scanner lens resolution. No duplicate grain selector or automatic engine conversion is required.

## Page ownership

Each page owns one stage of the workflow. A visible section belongs to exactly one page.

MAIN owns image intake and the delivered look. It holds Import RGB, Import Raw, Crop and upscale, Input, Camera, Profiles, Enlarger, Scanner, CMYK ink style, Output, and Export options.
FILM owns the film material. It holds Chemistry for film, Film base, Halation, Couplers, Grain, Camera diffusion, and Convert.

PRINT owns the print material. It holds Chemistry for print, Print base, Glare, and Enlarger diffusion.

ADVANCED owns processing controls that most sessions never change. It holds Spectral upsampling, Input gamut compress, and Output gamut compress.

CONFIG owns session configuration. It holds Diagnostics, GUI parameters, and Display.

The Enlarger and Scanner sections belong to MAIN even though the scanner reads a medium the print route produces. The section-preset menus are unaffected by page ownership: they follow the [preset section table](../presets/design.md).

A section that a route makes inactive must state that it is inactive and retain its stored values. An inactive section must not present a disabled control block as if it were editable.

A visible section must not be duplicated across pages.

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

The viewer footer owns the layer selector. It is the only surface that exposes it. CONFIG must not repeat it.

The development animation is fixed viewer behaviour. It must not have an editor, and it must not be persisted as an editable control. An automatic parameter update skips it; an explicit Preview or Scan retains it.

The viewer must let the user select a display ICC profile. The selected profile applies to the disposable viewer raster only. Windows discovers the primary display profile when the user selects none. An explicitly selected profile applies on every platform that supports LittleCMS. Without a usable profile, the encoded sRGB preview is used directly.

The viewer must expose a tonal-distribution readout of the retained output. It must be viewer-only: it must not alter saved or exported pixels.
- The readout is a fixed strip at the top of the control panel, above the tab row. It is co-visible with the controls, does not occlude the image, and does not increase the control panel width. It is default-on, collapsible, and its visibility state is persisted. It is viewer-only and never affects saved or exported pixels.

The readout must derive from the retained output. It must not require a second render.

An empty or absent output must present an explicit empty state instead of a blank or broken readout.

## Experimental controls

The following controls carry an experimental marker. The marker is a presentation cue; it does not change the parameter's behavior or validation.

- `print illuminant` selector — MAIN → Enlarger
- Input gamut-compression algorithm selector — ADVANCED → Input gamut compress
- Spektra `rms granularity` — FILM → Grain
- `film_channel_swap`, `print_channel_swap` — FILM → Convert

The channel-swap validator must reject a non-permutation. A value such as `[0, 0, 0]` currently passes validation and silently duplicates one density channel while dropping the other two. The shared renderer applies the swap to `profile.data.channel_density`, so a non-permutation corrupts the profile at render time. The validator must require exactly three indices in `0..2` that form a permutation of `0, 1, 2`.

The GUI must not present a selector that offers one choice.
A parameter that no control can change and that no pipeline stage reads must not own a GUI section under the [preset section rule](../presets/design.md). Every parameter carried by a look or section preset must have one visible GUI control or named action that writes the canonical runtime leaf; inactive conditions retain the value and explain why editing is disabled. GUI controls and canonical CLI `--set` paths must share the same type, validator, units, and conditions. Calibration baselines, derived values, and internal state are not preset-owned controls.

## Control search

The GUI must expose a search box at the top of the control panel. The search filters the controls of the parameter sections on the current tab by label.

- The search box is a single-line text field, always visible above the tab content.
- Filtering is case-insensitive substring match against the control label.
- A control whose label does not match the filter must not appear while the filter is not empty.
- A section with a matching control shows the section title and its matching controls, also when the section or one of its sub-sections is collapsed.
- A section without a matching control must not appear.
- Surfaces outside the parameter sections — import file selection, export options, profiles, the viewer, and GUI settings — keep their normal layout.
- The filter is transient UI state. It is not persisted and does not affect parameter values.
- An empty filter must restore the normal panel layout, so no control is hidden by the search.

## Labels and surfaces

A label must name the current product concept. A label must not name a removed implementation, a development tool, or a file format of a former dependency.

An operation status must appear on exactly one surface, the window header. It must be short and must describe the last accepted operation, under the [telemetry status rule](../telemetry/spec.md#local-surfaces).

A control that changes a persisted value must be reachable from the GUI. A persisted value with no control, no pipeline consumer, and no owning specification must be removed from the document.

Two controls that edit the same persisted value must not both be editable. When two surfaces must show one value, one is read-only.

Two different parameters that share a label must carry a qualifier that distinguishes them.

