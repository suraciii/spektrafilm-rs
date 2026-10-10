# GUI workflow design

- The GUI is an adapter over `core::runtime`; it does not implement a second film pipeline or color model.
- UI edits serialize to the runtime parameter model. Preview and Scan use the same model with different geometry limits.
- The profile selection module owns catalog discovery and stock and development-time pickers. The application owns selection state and render invalidation.
- Viewer presentation uses disposable display rasters. Save and Export consume retained native output or invoke the export renderer and shared writer.
- Preview and viewport composition use separate persistent workers. Each worker retains one queued snapshot and replaces it when a newer request arrives.
- The UI accepts pipeline output only when request, input, and parameter revisions match. It accepts composed frames only when the generation and viewport key match.
- Pipeline and viewer workers share immutable native images. The viewer reuses prepared display pixels when the source, display settings, and preview size match.
- Persistent startup state and transient viewer state use separate stores and invalidation rules.
- Export work runs outside the UI event loop. Completion, failure, and cancellation are explicit states; temporary output is never presented as completed output.
