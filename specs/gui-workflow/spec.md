# GUI workflow

## Contract

- The GUI exposes controls for input, film, print, scanner, output, advanced color, and configuration.
- Preview renders a bounded disposable image. Scan renders the original working resolution. Both use the current runtime controls.
- Save writes retained output. Export re-renders through the selected CPU f64 or WGPU f32 backend and exposes cancellation.
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
  `negative_to_positive`. Labels must follow the [scan output naming contract](../film-chain/spec.md#scan-output-naming-acceptance). Negative conversion additionally shows density brightness,
  three-channel balance, contrast, maximum density, and negative conversion
  method. A slide film scan and a negative film scan with `negative_to_positive` show
  the per-channel black and white points. Film scans show receiver profile,
  scanner finish, finish strength, and the `off` or `per_frame` setup mode.
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
- Numeric parameter fields use text editing and the default text cursor. Each original wheel event over a field changes its value by the declared step. Smoothed scroll residue must not repeat that change in later frames. Invalid or incomplete text must not enter runtime parameters.
- Preview computation and viewer composition run outside the UI thread. Queued requests retain only the latest snapshot. Results from older requests, input images, or parameter revisions must not replace current output.
- The viewer retains the last completed frame while new composition runs. Automatic parameter updates skip reveal and crossfade. Explicit Preview and Scan retain those configured animations.
- Base workflow selection and optional finishing controls must follow the [Magazine print enablement contract](../magazine-print-color/spec.md#enablement).
- The workflow selector must offer exactly the routes the workflow validator supports.
