# Telemetry design
## Boundary

- An operation owns its report from acceptance of an executable request through its terminal outcome. File chooser interaction and parameter editing are outside the operation.
- GUI debounce delay before acceptance is outside operation duration; the operation carries the input and parameter revisions captured by its worker. Save refers to the retained output's render identity and owns only its saving work.
- The frontend owns request acceptance, input loading, worker handoff, presentation handoff, and file publication. Runtime owns preparation and simulation observations.
- The compute backend owns actual dispatch, submission, transfer, cache, and GPU timing observations. The native writer owns encoding and metadata observations; lower layers MUST NOT infer frontend intent from environment variables or backend names.
- Observations MUST attach to the current operation explicitly. A backend MAY serve multiple operations; it MUST NOT maintain an unqualified mutable "last render" report.
- Existing last-call timing accessors retain their public behavior, but they MUST NOT supply operation attribution under concurrency. Worker results MUST carry their report to the owner that accepts, rejects, or publishes the result.
- A caller MUST propagate the operation context across spawned threads and bounded tasks that emit observations. Pure per-pixel Rayon work needs no telemetry context or span entry.
- CPU stage timing MUST enclose the stage call once, rather than create events per pixel; a `tracing` span MUST be entered on the thread that emits its work observations. Backend work MUST carry its parent operation and stage so GPU work cannot be attributed to a concurrent request.
- The shared typed observation vocabulary MUST live below core in the compute backend crate so model and backend code do not acquire a dependency on core. Its location is governed by [compute backend boundary](../compute-backend/design.md).
- Core owns the complete operation report and its serialization; frontends own collection selection and report storage. A scoped backend view MAY bind an operation sink to the existing `ComputeBackend` interface, but it MUST share the original device and cache; default backend implementations MUST identify the CPU image work they actually perform.
- Host orchestration and image-independent preparation are host observations, not CPU image execution; instrumentation MUST NOT require a second rendering pipeline or a new telemetry crate. Context MUST be passed explicitly through the operation's borrowed sink or scoped backend view; a thread-local current report is not an attribution boundary.

Counter aggregation, overflow reconciliation, and report validation MUST use one counter field definition. Phase recording and validation MUST use one ordered phase vocabulary.

The authoritative stage-name privacy vocabulary is `STAGE_NAMES` and `GpuStage` in the [shared observation types](../../crates/spektrafilm-backend/src/telemetry/types.rs). Device ingestion, saving conversion, histogram, and viewer stage producers MUST use that vocabulary; arbitrary stage labels remain rejected.

CLI saving sample conversion MUST bind the selected backend to its saving phase, just as loading and simulation bind their operation scopes. An identity saving colour transform still performs the device sample-conversion and terminal readback boundary; those actual executor and transfer observations MUST remain attached to saving rather than disappear through an unobserved backend.

## Collection and signals

- The system MUST use one operation-local observation source for local reports and `tracing` events. Stable event names and typed numeric fields MUST represent starts, routing decisions, backend batches, and terminal outcomes.
- Timing values MUST NOT be formatted strings. Span hierarchy MUST describe operation, phase, stage, and backend batch ownership.
- Summary mode collects fixed-size counters, bounded phase observations, and routing facts. GPU timing mode adds timestamp queries to existing compute passes.
- Neither mode MAY add image readbacks, image copies, retries, simulation passes, or synchronization points for observation. Query results MAY use a small auxiliary readback alongside an existing completion boundary; collection MUST NOT create per-pixel observations.
- Each operation MUST retain at most 512 observations across attempts, phases, stages, batches, and passes. Attempts, phases, and stages MUST use a separate budget of 256 slots; batches and passes MUST use the other 256 slots without consuming the high-level budget.
- Root outcome, configuration, exact total work counters, and fixed-category phase, semantic-stage, host-interval, and GPU pass totals MUST remain outside this detail limit. Later observations MUST update those totals and mark omitted detail as truncated without allocating more detail; a child that cannot retain its parent MUST also be omitted.
- Retained diagnostic data MUST occupy at most 1 MiB per operation, excluding temporary JSON serialization. Fixed-category aggregates MUST count against this budget.
- After fixed storage, batches and passes MAY consume at most half the remaining byte budget. Exhausting a slot or byte budget MUST stop the affected detail retention and report truncation.
- The report MUST identify its limits, retained and omitted observation counts, and covered timing scope. Counter overflow MUST report an unavailable value rather than wrap; the observation source MUST NOT allocate image-sized storage.
- Disabled collection MUST NOT add telemetry records, formatted fields, query resources, or synchronization beyond existing public timing behavior. Existing Runtime timing maps are a separate compatibility cost; removing them requires a separate API decision.
- Stage-level collection MUST NOT introduce a process-wide contended lock. Local structured telemetry events MUST use the report's field allowlist and privacy rules; existing human application logs remain outside the shareable report.
- The local report is the diagnostic source of truth for a completed operation. `tracing` is the causal event surface and can support external CPU profilers.
- The target system does not require an OpenTelemetry SDK, collector, metrics server, or exporter. This choice keeps local operation attribution independent of process-wide aggregation and avoids a second exporter lifecycle; metric names, units, and bounded dimensions remain suitable for a separately authorized OpenTelemetry integration.
- Event filtering MUST NOT change explicitly requested report collection; summary collection MUST cover every accepted operation while enabled. Pass timing MUST apply to the selected operations consistently; a report MUST identify its requested and effective collection mode.
- Unsupported GPU timing retains summary with a stable availability reason; a benchmark that selects only some iterations for timing MUST report that selection. Aggregates MUST identify sample count and MUST NOT silently combine collection modes.

## Live progress ownership

The operation MUST attach its bounded progress sink before any observation context is cloned for image work. Backend scopes MUST emit coarse begin and end events through that sink; core MUST own progress snapshots, journal serialization, and destination validation.

Frontends MUST own journal creation and writer lifetime. The CLI MUST create the journal before command work, poll it on a dedicated thread every 250 milliseconds, then finish the operation, join and flush the writer, and publish the final report.

GUI progress writes MUST run outside the interface and image workers. The background publisher MUST synchronize the begin record before releasing image-worker readiness; begin failure MUST release readiness with a separate diagnostic rather than fail image work.

Image-worker readiness waits MUST honor cancellation, supersession, and shutdown without requiring the journal writer to become ready. Cancelling readiness MUST stop image work rather than classify diagnostic unavailability as an image failure.

The frontend MUST defer retained Save nonblockingly until its journal is ready. A deferred Save MUST retain the pixels, saving settings, metadata, and source-render identity captured when accepted, independently of later interface changes.

The frontend MUST retain at most one Save awaiting journal readiness. Every request entry point MUST reject further Saves before allocating another operation collector or copying metadata; the Save action MUST remain disabled until the accepted Save leaves that pending state.

The session publisher MUST periodically drain active journals even when no terminal report is queued. It MUST flush and retire the matching progress journal before publishing its terminal report. Shutdown MUST cancel deferred image work and drain accepted progress and terminal reports before releasing its workers.

Progress events MUST remain independent of the final report detail budget. Full progress transport MUST increment a bounded loss counter rather than wait for a consumer; disabled collection MUST allocate neither a transport nor a writer.

The journal MUST preserve its initial begin record and reuse fixed-size circular snapshot slots. Snapshot throttling MUST occur at the persistence boundary; terminal events and shutdown flush MUST bypass that throttle. [Progress record semantics](metrics/spec.md#live-progress-records) own the disk format and reader ordering.

Journal creation MUST use exclusive file creation after protected-path validation. Recoverable creation, polling, and flushing errors MUST remain independent of image execution and final report publication.

## Routing and timing

- Runtime reports the resident eligibility decision separately from the route that actually executed. A declined resident route can still dispatch GPU stages.
- The backend reports actual executed work; it MUST NOT count work from requested or prepared parameters; CPU image implementations used inside a WGPU backend MUST report CPU execution. Host preparation or orchestration MUST NOT be reported as CPU image arithmetic, and host activity alone MUST NOT be used to guess an image stage's executor.
- Full-GPU scope and acceptance follow the [GPU execution contract](../gpu-execution/spec.md#computation-scope), including its [backend selection](../gpu-execution/spec.md#backend-selection), [algorithm alignment](../gpu-execution/spec.md#algorithm-alignment), and [acceptance](../gpu-execution/spec.md#acceptance) rules.
- Device residency and numerical and stochastic evidence follow the GPU execution [device residency](../gpu-execution/design.md#device-residency), [numerical alignment](../gpu-execution/design.md#numerical-alignment), and [stochastic alignment](../gpu-execution/design.md#stochastic-alignment) design; telemetry reports actual work and retains its existing schema and path meanings.
- Host durations use a monotonic clock. Each observation MUST name its boundary and parent.
- Phase, stage, and backend batch durations are inclusive wall durations. Stages MAY nest within batches, phases, or stages; batches MAY nest within phases or stages.
- Nested durations MUST NOT be summed as disjoint intervals. Cross-thread overlaps MUST remain visible rather than become a serial total.

Direct host timing categories describe elapsed intervals, not exclusive CPU time. Resource creation covers buffer creation and initialization, shader compilation, and pipeline and binding creation; upload covers host uploads.

Command encoding spans the encoder lifetime and MAY overlap resource creation and uploads. Queue submission measures actual submit calls.

Submission and readback wait intervals MUST measure only existing blocking waits. Root and retained ancestor totals are inclusive; consumers MUST NOT sum ancestors with descendants.

Each host observation MUST derive its start offset and duration from the same captured start instant. Computing the offset MUST NOT read the clock again, because scheduling delay between readings would inflate the reconstructed end time.

- A backend batch covers host preparation, command submission, the observed completion boundary, and result materialization when present. Host preparation can include parameter conversion, cache lookup, resource creation, upload calls, and command encoding; it is not CPU utilization.
- When map readiness is actually observed, submit-to-map-ready includes submission overhead, queue contention, device work, staging copies, driver scheduling, callback scheduling, and host wakeup. It is not kernel time; materialization covers mapped-range access, CPU copying, scalar conversion, result construction, and unmap work.
- At compute-device creation, the backend MUST request adapter-supported `TIMESTAMP_QUERY` as an optional capability regardless of current collection mode. It MUST allocate query resources and write timestamps only for operations captured in GPU timing mode.
- This allows session mode changes without recreating the active device; each Adapter object MUST receive at most one device request. If optional-feature device creation fails, initialization MUST request one fresh Adapter with the same selection options and make one baseline device request using the existing feature and limit policy.
- A fresh adapter request does not guarantee the same physical device; the report MUST identify the final adapter actually used. This initialization policy MUST be identical in all collection modes and MUST NOT broaden adapter selection or required timestamp features.
- Failed baseline initialization retains the existing backend-unavailable behavior; timing MUST remain unavailable as `feature_request_failed` after a successful baseline request. Summary mode and off mode MUST use unchanged image commands.
- The backend MUST convert timestamp differences with that queue's timestamp period. Pass records MUST identify their batch and pass occurrence.
- Query resolution and telemetry buffer copies are observation overhead and MUST be separate from application work counts. Invalid, unresolved, or unsupported queries MUST produce unavailable observations with reasons; timing MUST NOT request broader inside-pass or inside-encoder timestamp features when pass boundaries suffice.
- Each timed compute pass uses two query slots. Bounded query blocks MUST belong to the command or submission that uses them and MUST NOT be reused until mapping and unmapping complete.
- Query resolve and copy commands MUST append to existing command buffers. Query maps MUST register after submission and complete through existing retirement or readback polls.
- Collection MUST NOT add a submission or blocking poll solely to obtain timestamps.
- Submitted passes MUST update typed stage, pass-kind, and purpose aggregates independently of detail retention. Discarded commands MUST NOT count as submitted work.
- Timestamp completion MUST update valid counts and sums once. Pending results MUST remain unavailable rather than become zero.
- Query-pool or per-command capacity exhaustion MUST leave image work unblocked and report `query_capacity`. Unsupported timestamp capability MUST also leave image work unblocked and report missing coverage.
- A query map failure MUST report `query_failed`. Results pending at the terminal snapshot MUST report `not_ready` or incomplete coverage.
- Command failure and device loss retain normal execution failure handling.
- Completion callbacks MUST NOT retain an operation through an ownership cycle. Completion after operation finish MUST NOT mutate the published report.
- A batch with an observed device poll uses `device_poll_complete`; otherwise it uses `commands_submitted`. Neither boundary claims image materialization; unobserved map-ready and materialization intervals MUST be `not_applicable`.

A pass duration covers the device's pass interval. It does not measure isolated shader instructions, CPU work, queue delay before the pass, or physical transfer bandwidth. A sum of pass durations MUST be named `compute_pass_sum`; it MUST NOT be described as operation GPU duration or subtracted from host time to estimate CPU time.

## GPU accounting

- Logical upload and readback account for bytes crossing the application host/device boundary. They do not claim physical PCIe traffic.
- A shared-memory or directly mapped path still has logical transfers. Device-to-device staging copies have separate counters and MUST NOT be added to logical readback bytes a second time.
- Work counts MUST include actual backend calls for both full-image simulation and calibration work within the operation. Purpose distinguishes `image`, `calibration`, and `telemetry`.
- A report MUST NOT hide calibration uploads behind a one-upload resident description. GUI texture presentation belongs to the presentation phase and MUST NOT enter the simulation backend's GPU work counts.
- Shared-queue submission order and global error scopes can include other operations. Work counters MUST attribute writes and commands to the operation that issued them, regardless of which submission flushes a scheduled write.
- Observations MUST identify shared device use; completion waits include interference from other queued work; they MUST NOT be presented as isolated operation service time. Telemetry MUST NOT introduce a device-wide serialization lock.
- It MUST NOT push recovery error scopes on a device already shared with concurrent work. Error-scope capability validation MAY occur only before the device is exposed to workers; batch map callbacks supply recoverable query-readback errors; normal uncaptured allocation, validation, and device errors MUST retain execution failure semantics.
- Requested GPU buffer bytes describe API resource sizes. They do not describe committed VRAM, allocator residency, or driver pools.
- A resource is owned by the operation, shared cache, or presentation subsystem. Shared cache resources MUST NOT be charged as new operation allocations on a hit; physical device memory and process memory require separate measured sources.

## Terminal records and overhead

- The frontend finalizes the record at the [operation terminal boundary](metrics/spec.md#operation-and-phase-wall-time). CPU or GPU failure MAY leave partial observations; those values MUST identify their completed scope.
- Telemetry persistence errors MUST NOT rewrite the operation outcome. Preview completion ends at the presentation handoff, not at physical display scanout.
- GUI startup MUST select summary collection before the initial input request. It MUST create a unique persistent directory under the application configuration directory's `logs/diagnostics` area unless launch configuration provides an explicit destination.
- GUI automatic publication MUST share immutable terminal reports with in-memory history rather than copy report contents. A session-local worker MUST serialize and write reports outside the GUI thread.
- The nonblocking publication queue MUST retain at most 20 waiting reports in addition to the in-flight write.
- Queue exhaustion, worker startup failure, and file errors MUST surface through the existing diagnostic status boundary without changing image outcomes. Error retention MUST remain bounded.
- Each queued write MUST retain its session destination independently of report history eviction.
- Normal shutdown MUST close the queue and drain accepted writes before releasing the publication worker.
- Opening the log folder MUST NOT wait on the GUI thread. Completed opener processes MUST be reaped.
- Detailed reports MUST use stable schema fields and stable enumerated categories. Paths and raw exceptions MUST NOT enter those fields.
- Operation and revision identifiers are correlation fields for local reports and traces; they MUST NOT become metric labels. Device names, stock identifiers, dimensions, seeds, and arbitrary user strings MUST NOT become aggregate metric labels.
- Validation MUST compare telemetry-off and telemetry-on image results using existing numerical budgets. A controlled warm/cold comparison MUST measure collection overhead on each exercised route and identify the adapter, dimensions, precision, and iterations.
- The implementation MUST report measured overhead rather than assume a universal percentage. CPU and driver profilers remain external tools for instruction-level attribution.
- Summary overhead acceptance MUST use paired telemetry-off and summary runs with identical reuse conditions on CPU, resident, and per-stage routes. After one setup run, it MUST collect at least ten alternating measured pairs per route and retain individual times.
- The median increase MUST NOT exceed the greater of 5 milliseconds or 3 percent of the off-mode median; a breach blocks overhead acceptance for that route. GPU timing overhead MUST be measured and shown separately, because pass queries can perturb device scheduling; software-adapter evidence MUST NOT be used to claim hardware-driver overhead acceptance.
