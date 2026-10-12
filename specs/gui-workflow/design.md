# GUI workflow design

- The GUI is an adapter over `core::runtime`; it does not implement a second film pipeline or color model.
- UI edits serialize to the runtime parameter model. Preview and Scan use the same model with different geometry limits.
- Cached preview preparation and independent export preparation must share parameter digestion, scanned-medium selection, and channel-swap rules. Cache reuse and operation observation scopes remain separate.
- The profile selection module owns catalog discovery and stock and development-time pickers. The application owns selection state and render invalidation.
- Viewer presentation uses disposable display rasters. Save and Export consume retained native output or invoke the export renderer and shared writer.
- Preview and viewport composition use separate persistent workers. Each worker retains one queued snapshot and replaces it when a newer request arrives.
- A render request, the pending UI job, and the worker result share one captured render identity. The worker must preserve its collection mode and animation intent.
- The UI accepts pipeline output only when request, input, and parameter revisions match. It accepts composed frames only when the generation and viewport key match.
- Pipeline and viewer workers share immutable native images. Prepared display pixels use one cache identity rule: source allocation, display settings, and preview size must match.
- Persistent startup state and transient viewer state use separate stores and invalidation rules.
- Export work runs outside the UI event loop. Completion, failure, and cancellation are explicit states; temporary output is never presented as completed output.
- Numeric editors associate visible labels with their text inputs. Tuple editors identify each component. Profile and development-time selectors associate their visible labels with the selection control.
- Native Save and Export actions select a path before entering the shared output operation. Headless acceptance enters at that path boundary and executes the same worker and writer.
- Headless App acceptance exercises production controls and real CPU workers. It waits for operation completion and checks accepted output identities. Offscreen layout checks and native desktop acceptance have separate evidence scopes, as defined in the [acceptance guide](../../scripts/parity/README.md#headless-gui-acceptance).
