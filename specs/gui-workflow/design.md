# GUI workflow design
## Presentation language

The [GUI presentation design](../gui-presentation/design.md) owns reusable presentation decisions. This design specification remains authoritative for runtime boundaries, surface ownership, worker behavior, and rendering invariants.

The [GUI product design boundaries](../gui-product-design/design.md) own stage-inspector composition, control inventory integration, disclosure state, and adjustment-history integration. They MUST use the runtime and worker boundaries below.

The [GUI language design](../gui-language/design.md) owns localized presentation and the independent application-language preference. English labels in GUI specifications identify concepts; the selected language presents those concepts without changing their canonical identities or runtime semantics.

- The GUI is an adapter over `core::runtime`; it does not implement a second film pipeline or color model.
- UI edits serialize to the runtime parameter model. Preview and Scan use the same model with different geometry limits.
- Cached preview preparation and independent export preparation MUST share parameter digestion, scanned-medium selection, and channel-swap rules. Cache reuse and operation observation scopes remain separate.
- The private render preparation module owns captured render inputs and runtime construction without depending on the App or GUI widgets. The application owns source-revision tracking, worker scheduling, and result publication.
- The render worker's runtime cache owns its entry, preparation, and setup attachment. A present entry contains a valid runtime.
- Cached Preview and Scan parameter updates MUST use the core [material preparation invariants](../film-chain/design.md). The GUI MUST NOT maintain a separate material-control invalidation key.
- Setup failure leaves the entry unchanged; parameter-update failure clears it; failed preparation for a different key retains the previous entry.
- The input worker owns source decoding, device orientation, and initial-view preparation. Presentation refreshes MUST NOT supersede source transactions.
- The application MUST retain accepted contexts, histories, and drafts until all preparation succeeds. Only a commit that replaces the active input MAY invalidate its pending presentation refresh.
- Material controls MUST use existing runtime ownership and capability checks under [Ownership](../output-mix/design.md#ownership). Print contains Enlarger and Preflash under [Print exposure composition](../output-mix/design.md#print-exposure-composition).
- Film and Print retain existing chemistry and model-specific developer exhaustion. Inactive routes and unsupported profiles MUST retain stored values and report capability conditions.
- Print-filter neutralization MUST fit a deterministic pointwise request with spatial and stochastic effects disabled. It MUST retain caller parameters and vary only local trial M/Y shifts.
- Module resets and serialized section payloads have separate scopes under [Section reset](spec.md#section-reset). The GUI MUST NOT add digital-response preparation, tonal masks, creative cyan shifts, saturation or color-density groups, or duplicate Scanner levels.
- Material selection and reset-baseline construction MUST use the core-owned [material defaults](../presets/design.md#material-defaults), not a GUI copy of stock tables. Reset baseline construction starts from a fresh factory projection without live explicit edits.
- Ordinary rendering, full-state restoration, and history replay retain accepted values without material-default initialization. Existing state carriers and versions remain unchanged.
- Scanner references and inverses retain their existing dependencies. Input and capture → Camera exposes the canonical `camera.capture_illuminant` leaf under the [Camera control contract](spec.md#contract), including inherited effective values and the CLI's shared type, validator, and conditions.
- The output amount control MUST use [core output composition](../output-mix/design.md#output-composition). The GUI MUST NOT derive an output-mix source from the Input viewer raster, add an output lightness stage, or add a separate output-processing switch for the `input` workflow.
- State and preset loading MUST validate and migrate a candidate before committing any UI or runtime state. With a loaded source, state application prepares RAW reloads, GPU upload and the input view on the input worker before one UI-side commit.
- RGB state application MUST reuse accepted source samples and preserve their metadata and source identity without rereading the file. Completion status and history replacement occur only after that commit; preparation failure preserves the source, controls, accepted output, history and numeric drafts.
- Rejected retired settings likewise leave the accepted transaction intact under [Persistence and delivery](../output-mix/design.md#persistence-and-delivery).
- Core runtime validation emits semantic reason, canonical parameter path, and value arguments at the authoritative rule. GUI state loading retains that typed error until localized presentation; the CLI keeps the existing English formatting through the same validator.
- Validation MUST NOT be duplicated in the GUI or inferred by classifying English error text.
- Grain disclosure MUST consume the core-owned applicability query. The GUI MUST NOT parse condition descriptions or maintain a second applicability table.
- Production grain queries MUST reuse the cached actual film profile without per-row loading or copying. Isolated surfaces without material context MAY query configured applicability.
- The profile selection module owns catalog discovery and stock and development-time pickers. The application owns selection state and render invalidation.
- Viewer presentation uses disposable device or host display rasters under [GPU residency](../gpu-execution/design.md#device-residency). Save and Export consume retained floating output or invoke the export renderer and shared writer.
- The private raster module owns interpolation kernels and pixel sampling. The viewer owns color transforms, caching, and presentation state.
- Full-source sampling transforms each source tap before interpolation. Capped display rasters reduce the source before color conversion.
- GPU viewing MUST satisfy [numerical alignment](../gpu-execution/design.md#numerical-alignment) without CPU image arithmetic.
- Preview and viewport composition use separate persistent workers. Each worker retains one queued snapshot and replaces it when a newer request arrives.
- The UI always schedules Preview after the frame validates and accepts adjustments. The final edit MUST arm its debounce wakeup before the event loop becomes idle.
- It retains the existing calibration and modal guards.
- A render request, the pending UI job, and the worker result share one captured render identity. The worker MUST preserve its collection mode and animation intent.
- Preview, Scan, and Export MUST bind their actual retained backend to the operation's observation context. Host-only geometry preparation belongs to `input_prepare`.
- Device input upload, device geometry, and the retained film chain belong to `simulation`. Display conversion belongs to `display_prepare`; export conversion and its final readback belong to `saving_convert`.
- Workers MUST NOT infer backend, executor, or adapter fields from the selected controls.
- The UI accepts pipeline output only when request, input, and parameter revisions match. It accepts composed frames only when the generation and viewport key match.
- Viewer pending status is derived from the accepted dirty state, current render job, and composition worker lifecycle. It MUST NOT create a parallel status machine or depend on telemetry collection.
- A consumed composition failure ends pending presentation without replacing the last accepted texture.
- Pipeline and viewer workers share immutable image ownership in the selected execution domain. Prepared display samples use one cache identity rule: source allocation, display settings, and preview size MUST match.
- Full-source viewing caches the display transform per source pixel under the same identity rule. Cached sampling and direct per-tap sampling MUST produce identical pixels.
- Progress reporting observes work-unit boundaries for presentation only. It MUST NOT change routing, parameters, pixels, or telemetry.
- A frontend binds one progress sink per worker thread.
- Persistent startup state and transient viewer state use separate stores and invalidation rules.
- Export work runs outside the UI event loop. Completion, failure, and cancellation are explicit states; temporary output is never presented as completed output.
- Numeric editors associate visible labels with their text inputs. Tuple editors identify each component.
- Profile and development-time selectors associate their visible labels with the selection control.
- Native Save and Export actions select a path before entering the shared output operation. Headless acceptance enters at that path boundary and executes the same worker and writer.
- Headless App acceptance exercises production controls and real workers. CPU coverage, hardware-GPU computation coverage, offscreen layout checks, and native desktop acceptance have separate evidence scopes under [GPU acceptance](../gpu-execution/spec.md#acceptance) and the [GUI acceptance guide](../../scripts/parity/README.md#headless-gui-acceptance).
- Numeric editing and validation share one control implementation under [numeric editor ranges](spec.md#numeric-editor-ranges); an editor cannot accept a value the validator rejects.
- The editor range table follows [numeric editor ranges](spec.md#numeric-editor-ranges) and lives alongside the existing `unit()` and `leaf_numeric_bounds()` declarations in core, so the editor range, the validator range, and the discovery output cannot drift apart.
- Channel-swap validation follows the [experimental controls](spec.md#experimental-controls) permutation rule and rejects a non-permutation before rendering in both the GUI state loader and the CLI.
- Image analysis MUST follow the [analysis execution and publication boundary](../gui-product-design/image-analysis/design.md#execution-and-publication).
- Normal GPU Preview and Scan MUST publish retained device output with carrier-owned dimensions rather than a compulsory host float raster.
- Pixel inspection MAY request one original RGB triplet through a bounded readback; it never substitutes viewer pixels or edits retained samples. Explicit non-RGB diagnostic taps retain their intentional final materialization boundary.
- Image-to-plot sample linking MUST follow [linked pixel sampling](../gui-product-design/image-analysis/design.md#linked-pixel-sampling) rather than create another pixel-readback or coordinate-mapping owner.
- A GUI surface has one owner. Duplicated rendering of one value from two surfaces is a defect, not a convenience.
- Region, action, and editor ownership follow [Window layout](spec.md#window-layout) and [Stage and surface ownership](spec.md#stage-and-surface-ownership). The stage inspector replaces page tabs; neither application settings nor independent Export MAY create a second photographic editor.

## Display color management

Display color management MUST remain separate from simulation and saving conversion. The CPU MUST prepare immutable, image-independent ICC transform data through LittleCMS. Preparation MUST NOT evaluate source or retained image pixels.

This boundary MUST NOT move simulation, spectral calibration, or saving conversion to the host.

GPU application MUST use a bounded set of fixed conversion kernels with prepared coefficients and lookup data. Changing a profile MUST change data or select an existing kernel, not generate profile-specific shader code. ICC linking, curve inversion, and numerical optimizer preparation MUST NOT execute in GPU shaders.

Fast-path qualification MUST inspect the complete resolved transformation, not only matrix or tone-curve tags. The selected representation MUST preserve the [reference conversion](spec.md#viewer-presentation). Pixel execution remains subject to the [GPU execution contract](../gpu-execution/spec.md#backend-selection).

Transform preparation, cache I/O, and GPU compilation MUST run outside the UI event loop. The UI MUST NOT wait for these operations or locks held across them. Identical in-flight preparation requests MUST share work.

Result publication MUST follow the captured request and display-generation rules above.

Prepared-transform data, device resources, and compiled pipelines MUST have separate cache identities. Prepared data MUST be reusable across images and application sessions. Its identity MUST include profile content, conversion settings, sample representation, LittleCMS version, and preparation-format and algorithm versions.

A device change MUST invalidate device resources without invalidating compatible image-independent prepared data.

Persistent prepared data MUST NOT contain native handles or process pointers. Cache publication MUST be atomic. Cache storage MUST be bounded.

Rejected cache entries MUST be rebuilt in the background. Compiled pipeline persistence MAY be used only when the backend supports compatible reuse. Pipeline cache identity MUST include the shader, layout, compilation settings, backend, and device and driver compatibility.
