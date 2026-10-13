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

## Stage-owned color controls

Control placement and process meaning must follow [Film and print digital response](../material-response/spec.md#user-operation). The GUI must retain the existing MAIN, FILM, PRINT, ADVANCED, and CONFIG pages. It must not introduce a generic Reproduction section.

MAIN Enlarger must expose Cyan shift beside the existing Magenta and Yellow shifts, using the existing enlarger filtration units and neutral default. It must show print exposure and reference compensation in the same section. A route without actual print exposure and development must disable these controls and identify the missing stage without discarding values.

FILM must expose a Digital response group owned by `film_render.response`: active bypass, highlight start in EV, highlight amount, saturation, and color density. It must retain film chemistry and development-shoulder controls as distinct controls. The digital response changes developed film record density; it is not a final RGB correction. The group must provide bypass and response-only reset. The existing Film section preset menu must carry these controls without a separate overlapping response preset section or redundant group-strength control.

PRINT must expose a Digital response group owned by `print_render.response`: active bypass, highlight start in the reference material whiteward tone coordinate, highlight amount, saturation, color density, and shadows, midtones, and highlights tone. Tone controls must be labeled as warm/cool print exposure filtration, not final RGB split toning. Print chemistry, preflash, and print base controls remain distinct. The group must provide bypass and response-only reset. The existing Print section preset menu must carry these controls without a separate overlapping response preset section.

Film response controls must be disabled when the route does not develop film, including conversion of a real captured negative. Print response controls must be disabled when the route does not expose and develop a print. Disabled or bypassed response controls retain their stored values. Finished RGB must mark both response sections and material and Scanner controls inapplicable. Existing Scanner black and white points retain their medium-specific meaning; the GUI must not add duplicate generic levels.

MAIN Output must expose Overall effect amount as the full-result/prepared-source blend under [Output mix](../material-response/spec.md#output-mix), with its existing section menu. Existing output gamut algorithm and chroma-knee controls remain available without duplicate editable copies. Output must not expose standalone lightness compression, an Output response section or preset, or a new Finished RGB output-processing switch. A negative-capture source must state that zero Overall effect amount returns the prepared negative, not a positive interpretation. Input/Output viewing remains independent from rendered effect amount.

Loading retired settings must follow [Output boundary and retirement](../material-response/spec.md#output-boundary-and-retirement). A rejected document must identify the exact retired path and leave the source document, active controls, accepted render identity, and retained output unchanged. The GUI must not silently replace a retired numeric lightness triplet with Film or Print response settings; the user must explicitly remove it and choose any replacement.

## Optical and grain labels

MAIN Scanner must distinguish Lens blur (readout optics) from Unsharp mask (digital sharpening). Identity point controls must not imply a different optical order. Tooltips must explain the [optical-order cutover](../film-chain/spec.md#optical-order-cutover) without offering a legacy late-blur mode.

PRINT may retain the `print_render.glare` controls in its existing preset section, but their visible group must be labeled Print viewing glare. Its description must identify readout stray light, not preflash or a change in material density. Preflash and Enlarger diffusion must remain separately labeled exposure and optical controls. Group location must not imply that all Print controls execute during development.

The existing grain selector must retain its stable engine names and preset section. It must identify Spektra and Dye as material grain and Procedural as post-scan grain appearance. Selecting Procedural must describe its Film Resolution control as part of that appearance algorithm, not as scanner lens resolution. No duplicate grain selector or automatic engine conversion is required.

