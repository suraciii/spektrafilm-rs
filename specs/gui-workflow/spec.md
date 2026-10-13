# GUI workflow

## Contract

- The GUI exposes controls for input, film, print, scanner, output, advanced color, and configuration.
- Preview renders a bounded disposable image. Scan renders the original working resolution. Both use the current runtime controls.
- Save writes retained output. Export re-renders through the selected CPU f64 or WGPU f32 backend and exposes cancellation.
- Startup controls may be saved and restored. Removing the startup default restores the factory profile.
- Viewer-only state, such as zoom, interpolation, reveal, and canvas presentation, must not alter saved or exported pixels.

- The Scanner controls show exactly the controls that
  [Scan medium and scanner controls](../film-chain/spec.md#scan-medium-and-scanner-controls)
  assign to the scanned medium. For a negative film profile on the direct
  `input > film > scan` route or the real negative-capture `input > convert-film > scan` route, they offer `direct_scan` and
  `positive_scan`. Positive scan additionally shows density brightness,
  three-channel balance, contrast, maximum density, and positive interpretation
  method. A slide film scan and a negative film scan with `positive_scan` show
  the per-channel black and white points. Film scans show receiver profile,
  scanner finish, finish strength, and the `off` or `per_frame` setup mode.
  The GUI identifies approximate receiver profiles as approximate. A route or
  film profile change that makes the selected mode ineligible resets it to
  `direct_scan` and clears ineligible scanner controls.
- Selecting `positive_scan` clears scanner white and black correction flags.
  Hidden capture-correction settings must not prevent this mode selection.
  State and preset validation still reject an explicitly supplied incompatible
  scan configuration.
- Scanner setup controls must expose whether the current setup is disabled or
  derived for the current input revision. Preview, Scan, and Export must use
  the same accepted setup for one input and parameter revision.
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

