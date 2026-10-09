# Photo export design

- Rendering and writing are separate boundaries. The renderer returns native floating output; one shared writer handles CLI, GUI Save, and f64 Export.
- Export chooses a backend and re-renders at export geometry. Save starts from retained output and applies only the requested saving conversion.
- Simulation color settings and saving color settings are distinct. A writer option never changes film-chain gamut mapping.
- Writers validate format-specific constraints before opening the destination, write to a temporary path, and publish only after the write succeeds.
- Cancellation is checked before rendering and around publishable work. A native write that cannot be interrupted completes into a temporary path and is removed on cancellation.
