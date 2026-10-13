# GUI workflow

## Contract

- The GUI exposes controls for input, film, print, scanner, output, advanced color, and configuration.
- Preview renders a bounded disposable image. Scan renders the original working resolution. Both use the current runtime controls.
- Save writes retained output. Export re-renders through the selected backend and exposes cancellation. Render, saving-conversion, and viewer execution follow [GPU execution](../gpu-execution/spec.md); selecting CPU does not imply a precision different from the actual build.
- Startup controls may be saved and restored. Removing the startup default restores the factory profile.
- Viewer-only state, such as zoom, interpolation, reveal, and canvas presentation, must not alter saved or exported pixels.
- Simulation engine selection is independent of historical grain engine selection. SpektraFilm remains the factory default. Fotufilm displays the full original stock catalogue and executable original controls from the source ControlsManifest, dynamic menus and stock applicability. It must not relabel historical scanner or print controls as Fotufilm controls.
- Fotufilm engine and request persist in the runtime state snapshot. Preview, Scan and Export consume that same request through the runtime worker, including original Standard/Film grain selection, media/format, source illuminant, optics and process controls. Native options remain accessible with their authoritative original types. Source validation failures remain visible and never substitute defaults.
- Fotufilm Preview renders the complete original pipeline at source geometry, then resizes delivery. Its original grain and optics remain active. Fotufilm runs the original binary32 CPU graph independently of the historical backend selector.
- The simulation seed is the host UInt64 field; it is not a second native override. Original options are editable JSON with explicit Apply and Reset actions, preserving numeric, Boolean, string, array and object types through state and Fotufilm section presets.
- Original negative-stage output retains source geometry and gross record density metadata after engine or request edits. Crop, upscale and preview delivery limits do not resample retained density. Display alone may scale its transmission `10^-D` raster; Save and Export preserve the unencoded density samples in 32-bit TIFF or EXR. EXR normalization must retain the density domain rather than select an RGB color space.

- The Scanner controls show exactly the controls that
  [Scan medium and scanner controls](../film-chain/spec.md#scan-medium-and-scanner-controls)
  assign to the scanned medium. For a negative film profile on the direct
  `input > film > scan` route or the real negative-capture `input > convert-film > scan` route, they offer `direct_scan` and
  `negative_to_positive`. Labels must follow the [scan output naming contract](../film-chain/spec.md#scan-output-naming-acceptance). Exposure reconstruction additionally shows density brightness,
  three-channel balance, contrast, maximum density, and negative conversion
  method. A slide film scan and a reconstructed negative film scan show
  the per-channel black and white points. Film scans show receiver profile,
  scanner finish, finish strength, and the `off` or `per_frame` setup mode where eligible.
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
- Preview computation and viewer composition run outside the UI thread. Queued requests retain only the latest snapshot. Results from older requests, input images, or parameter revisions must not replace current output.
- The viewer retains the last completed frame while new composition runs. Automatic parameter updates skip reveal and crossfade. Explicit Preview and Scan retain those configured animations.
- Base workflow selection and optional finishing controls must follow the [Magazine print enablement contract](../magazine-print-color/spec.md#enablement).
- The workflow selector must offer exactly the routes the workflow validator supports.

## Material controls and output composition

Control placement and process meaning must follow [Material controls and output composition](../material-response/spec.md#user-operation). The GUI must retain the existing MAIN, FILM, PRINT, ADVANCED, and CONFIG pages and section menus. It must not add Digital response panels, a Reproduction section, saturation or color-density groups, or tonal-mask controls.

MAIN Enlarger must expose existing print exposure, Magenta and Yellow shifts, and preflash exposure with its independent Magenta and Yellow filter shifts. Calibrated cyan remains part of the neutral filter pack without a new Cyan shift control. Preflash must retain its existing units, defaults, bounds, and section ownership under [Preflash](../material-response/spec.md#preflash). Zero preflash exposure must disable flash without clearing its filters. A route without print exposure and development must identify these controls as inactive and retain their values.

Preflash help must explain that, on negative-to-photographic-paper printing, it can bring weak highlight exposure differences into a responsive part of the paper curve. It cannot recover detail absent from the negative. Excessive preflash can lower paper white, add fog, alter midtones, and change color. Help must not promise fixed white or midgray, improved detail at every setting, or equivalence to digital highlight compression. Printing-film routes must retain their own material polarity and must not inherit a paper-highlight promise.

FILM and PRINT must retain their existing chemistry and development controls. Developer exhaustion must keep its model-specific development meaning and capability conditions. It must not be renamed as a generic highlight or high-dynamic-range control. Profile and route feedback must identify unsupported or inactive controls without clearing stored values or fabricating model support. Film-development controls are inactive when converting a real captured negative; print-development controls remain available only on admitted printing routes. Finished RGB must mark material and Scanner controls inapplicable. Existing Scanner black and white points retain their medium-specific meaning; the GUI must not add duplicate generic levels.

Material edits must use existing scanner calibration, inverse reconstruction, and automatic setup semantics. The GUI must not promise creative-reference isolation or add separate baseline and effective reference controls. Resets and section presets must retain existing ownership without new response resets or preset sections.

MAIN Output must expose Overall effect amount as the full-result/prepared-source blend under [Output mix](../material-response/spec.md#output-mix), with its existing section menu. Existing output gamut algorithm and chroma-knee controls remain available without duplicate editable copies. Output must not expose standalone lightness compression, an Output response section or preset, or a new Finished RGB output-processing switch. A negative-capture source must state that zero Overall effect amount returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent from rendered effect amount.

Loading retired settings must follow [Output boundary and retirement](../material-response/spec.md#output-boundary-and-retirement). A rejected document must identify the exact offending path and leave the source document, active controls, accepted render identity, and retained output unchanged. The GUI must require explicit removal of a retired numeric lightness triplet without suggesting withdrawn response groups or automatically substituting preflash or chemistry. Withdrawn draft response groups and cyan-shift paths must fail normal unknown-field validation without aliases or migration.

## Optical and grain labels

MAIN Scanner must distinguish Lens blur (readout optics) from Unsharp mask (digital sharpening). Identity point controls must not imply a different optical order. Tooltips must explain the [optical-order cutover](../film-chain/spec.md#optical-order-cutover) without offering a legacy late-blur mode.

PRINT may retain the `print_render.glare` controls in its existing preset section, but their visible group must be labeled Print viewing glare. Its description must identify readout stray light, not preflash or a change in material density. Preflash and Enlarger diffusion must remain separately labeled exposure and optical controls. Group location must not imply that all Print controls execute during development.

The existing grain selector must retain its stable engine names and preset section. It must identify Spektra and Dye as material grain and Procedural as post-scan grain appearance. Selecting Procedural must describe its Film Resolution control as part of that appearance algorithm, not as scanner lens resolution. No duplicate grain selector or automatic engine conversion is required.

