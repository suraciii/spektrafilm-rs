# GUI workflow

## Contract

- The GUI exposes controls for input, film, print, scanner, output, advanced color, and configuration.
- Preview renders a bounded disposable image. Scan renders the original working resolution. Both use the current runtime controls.
- Save writes retained output. Export re-renders through the selected CPU f64 or WGPU f32 backend and exposes cancellation.
- Startup controls may be saved and restored. Removing the startup default restores the factory profile.
- Viewer-only state, such as zoom, interpolation, reveal, and canvas presentation, must not alter saved or exported pixels.

- For the direct `input > film > scan` route, the Scanner controls expose
  `direct_scan` and `positive_scan`. Route changes away from direct film scan
  reset the output mode to `direct_scan`.
- Numeric parameter fields use text editing and the default text cursor. Scrolling over a field changes its value by the declared step. Invalid or incomplete text must not enter runtime parameters.
- Preview computation and viewer composition run outside the UI thread. Queued requests retain only the latest snapshot. Results from older requests, input images, or parameter revisions must not replace current output.
- The viewer retains the last completed frame while new composition runs. Automatic parameter updates skip reveal and crossfade. Explicit Preview and Scan retain those configured animations.
