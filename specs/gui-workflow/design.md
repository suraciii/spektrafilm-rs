# GUI workflow design

- The GUI is an adapter over `core::runtime`; it does not implement a second film pipeline or color model.
- UI edits serialize to the runtime parameter model. Preview and Scan use the same model with different geometry limits.
- The profile selection module owns catalog discovery and stock and development-time pickers. The application owns selection state and render invalidation.
- Viewer presentation uses disposable display rasters. Save and Export consume retained native output or invoke the export renderer and shared writer.
- Persistent startup state and transient viewer state use separate stores and invalidation rules.
- Export work runs outside the UI event loop. Completion, failure, and cancellation are explicit states; temporary output is never presented as completed output.
