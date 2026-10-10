# Telemetry metrics

## Record semantics

A report describes one operation as defined in the [contract](spec.md). Ownership and timing boundaries are defined in the [design](design.md).

The report format is one UTF-8 JSON object with `schema_version` equal to `1`. Its root fields are `schema_version`, `build`, `operation`, `source_render`, `configuration`, `execution`, `measurements`, `attempts`, `phases`, `stages`, `gpu_batches`, `diagnostic_issues`, and `coverage`. `source_render` is present only for Save. Numeric measurements must use JSON numbers, never formatted strings, NaN, or infinity. Integer counters must preserve u64 values through serialization and parsing. `build` identifies the application version and source revision when available. `operation` contains a session-local `id`, stable `kind`, `outcome`, and optional `input_revision` and `parameter_revision`. Report readers must ignore unknown fields within a supported schema version. A change to a field's meaning or unit requires a new schema version.

Each measurement has a `status`: `available`, `unavailable`, or `not_applicable`. An `available` measurement contains a finite numeric `value`. Other statuses contain a stable `reason` and no value. Zero means measured absence of work or elapsed time; it never means missing instrumentation. Duration units are seconds (`s`), byte units are bytes (`By`), and counts are nonnegative integers. Unexecuted phases must be omitted rather than fabricated as zero-duration work.

`phases`, `stages`, and `gpu_batches` are flat arrays of observations with unique local identifiers, a `parent_id`, and a stable `name`. Every retained child's parent must be retained. Attempts parent to the operation; phases parent to the operation, attempt, or enclosing phase; stages parent to a phase or enclosing stage; batches parent to a stage or phase; GPU passes parent to a batch. Repeated names must retain distinct occurrences. Host observations use `start_offset` relative to operation acceptance and inclusive `duration`, both in seconds. CPU and GPU clocks must not be compared as if they shared an epoch. GPU pass durations are measurements attached to their batch, without host start offsets derived from device ticks.

`attempts` is empty for Save and contains indexed simulation attempts for rendering operations. Each attempt owns its route, simulation observations, and GPU batches. An attempt that failed before simulation must identify that boundary. The root execution summary must identify differing attempt routes instead of selecting the last route silently. Invocation totals aggregate actual work from all attempts; final saving phases reference only the saved attempt. Details beyond the record limit follow [collection limits](design.md#collection-and-signals). `coverage` identifies `detail_limit`, retained and omitted observation counts, and truncation. Fixed-category totals must remain available even when occurrence details are omitted.

A terminal outcome closes the operation's total duration. Incomplete intervals on failure must identify themselves as incomplete; they must not be presented as completed stage timings. An unavailable duration requires a reason such as `not_collected`, `unsupported`, `query_failed`, or `counter_overflow`. Partial or truncated detail must not be treated as a complete distribution.

Observed configuration fields must be omitted when unresolved. `configuration.unavailable_fields` must identify each unresolved required field and its reason. Save must describe retained pixels and saving configuration; it must not copy current controls as if they produced the retained render. `source_render` must include the producing operation identifier, input and parameter revisions, dimensions, and `diagnostics_collected`; it must not retain a full parameter snapshot. Stable kinds are `preview`, `scan`, `save`, `export`, `process`, and `render`.

`diagnostic_issues` contains bounded stable categories for unavailable collection and report warnings. Image errors use `input`, `configuration`, `backend`, `simulation`, `writing`, `publication`, or `cancelled` with their observed boundary. Recoverable diagnostic issues do not rewrite operation outcomes. Build identifiers, device descriptions, and bundled identifiers must be bounded to 256 Unicode characters per value; truncated descriptions must be marked. Unknown user-defined identifiers must use `custom`, not a path-derived name. At most 64 issue entries may be retained, with exact omitted issue counts. Default JSON reports must not embed human log messages or arbitrary `tracing` fields.

## Effective configuration

`configuration` contains `collection_mode_requested` and `collection_mode_effective` (`summary` or `gpu_timing`), operation input dimensions, simulation working dimensions, and published output dimensions when a file was published. GPU timing can be only partially available across batches; pass coverage must describe that separately. It must also contain host scalar precision (`f32` or `f64`) and GPU arithmetic precision (`f32` when GPU work executed). Spectral preparation that uses f64 must remain identified independently of host scalar precision.

For rendering operations, the configuration must contain workflow route, bundled film and print stock identifiers when applicable, RGB-to-raw algorithm, input/output color-space roles, transfer decode/encode flags, requested spectral LUT flags, active gamut algorithm, and effective grain engine and mode. Effective grain settings must include whether grain ran and whether V1 used the fast statistics sampler. Effective spatial settings must identify active lens blur, halation, optical diffusion, DIR, glare, and unsharp processing. Saving configuration must contain format, depth, compression, saving color space, and saving transfer encoding when a write occurred. Save must include the retained pixels' color space and encoding; simulation settings absent from its small source reference must be unavailable as `source_configuration_not_retained`.

`execution` contains `backend_requested` (`auto`, `cpu`, or `wgpu`), `backend_selected` (`cpu` or `wgpu`), and an attempt `path` (`cpu`, `gpu_resident`, or `per_stage`). The root path is `mixed` when attempt paths differ. Unresolved execution fields must be listed in `execution.unavailable_fields` with reasons. Save's render backend and path are not applicable, regardless of the current GUI backend. WGPU execution must identify compute adapter API, device type, and name; presentation identifies a separate device only when that device information is observed. Adapter device types distinguish `cpu`, `integrated`, `discrete`, `virtual`, and `other`; a CPU adapter must not be described as hardware GPU evidence.

A `per_stage` record must include each executed stage's `executor` (`cpu`, `gpu`, or `mixed`) and `purpose` (`image` or `calibration`). A composite Grain V1 stage with CPU sampling and GPU blur must be `mixed`, with a CPU sampler observation and separate GPU batches. `gpu_resident` describes the fused chain route; it does not claim that loading, preparation, post-scan handling, and saving ran on the GPU. Save's path is `not_applicable` because it does not render.

`execution.resident_decline_reasons` contains stable codes from the routing authority, including workflow, chemistry, input transfer, spectral LUT, optical diffusion, V1 grain, gamut support, blur support, front-pass availability, and backend support. Each code must identify the effective condition that declined the route. A stage's CPU implementation must have a separate reason when its own GPU implementation was declined. Normal CPU selection must not be called a fallback. A tap request that bypasses the resident route must be identifiable as a diagnostic route.

Stable availability reasons are `not_collected`, `not_reached`, `not_applicable`, `unsupported`, `feature_request_failed`, `not_enabled_on_device`, `query_failed`, `not_ready`, `invalid_timestamp`, `detail_limit`, `incomplete`, `counter_overflow`, and `source_configuration_not_retained`. Resident-decline codes are `workflow_route`, `langmuir_chemistry`, `input_transfer_decoding`, `requested_spectral_lut`, `active_optical_diffusion`, `faithful_grain_distribution`, `unsupported_output_gamut`, `blur_radius_exceeds_backend_support`, `missing_resident_front_pass`, `mallett_execution_parity`, `backend_no_resident_support`, and `diagnostic_tap_route`. Human labels must be mapped from these codes; arbitrary error strings must not supply a code.

## Required diagnostic measurements

These measurements answer where an operation spent time and whether work crossed the host/device boundary repeatedly.

### Operation and phase wall time

`operation.duration` is wall time from acceptance of an executable request to its terminal outcome. CLI acceptance occurs after argument parsing and before command-specific validation; parser usage failures do not create image-operation records. GUI acceptance occurs after required file choices are complete and before operation-specific validation or worker dispatch. Its scope includes worker wait and final acceptance, publication, or machine-result preparation. It excludes user interaction before acceptance and report persistence afterward.

Phase names are `input_load`, `backend_init`, `input_prepare`, `runtime_prepare`, `simulation`, `display_prepare`, `presentation_handoff`, `saving_convert`, `file_write`, `metadata`, `publication`, and `result_prepare`. A phase is recorded only when that work occurs. `worker_wait` measures worker dispatch to worker start; it is a host scheduling observation and must not be called GPU queue wait. Input snapshot copies belong to `input_prepare`. Post-publication output hashing and machine-result assembly belong to `result_prepare` and remain inside invocation duration.

`simulation` encloses the Runtime processing call, including metering, geometry, color-reference preparation, the executed film chain, and output post-scan handling. Input loading and Runtime construction belong to separate phases. A GUI image load completed before render acceptance must not be charged again to Preview or Scan. Stage observations identify `metering`, `geometry`, `color_reference`, `filming_expose`, `filming_develop`, `grain_v1`, `grain_v2`, `printing`, `scanning`, and `post_scan` when separately observable. A fused chain must report its batch rather than invent independent host stage durations for passes within it.

`file_write` includes native pixel conversion, encoding/compression, and metadata work when the writer does not expose them separately. A separately observed `metadata` duration is a child of `file_write`; it must not be added to that inclusive duration. A nonfatal metadata warning must remain a warning on a successful pixel write. `publication` ends after the destination has been published or publication has failed. `presentation_handoff` measures result acceptance and submission to the GUI presentation system; it must not claim display scanout latency. A display raster recomputed during GUI result acceptance must record a separate `display_prepare` occurrence.

### Backend batch wall time

Each backend batch must report `host_prepare`, `submit_to_map_ready`, `host_materialize`, and total batch duration. Their boundaries follow [routing and timing](design.md#routing-and-timing). Values must come from direct monotonic observations rather than subtracting rounded log values. A backend operation with no host result readback must identify the completion boundary it actually observed.

`host_prepare` explains conversion, setup, and command-encoding cost. `submit_to_map_ready` explains the interval in which the host awaited a result. `host_materialize` explains mapped data copying and result construction. These are elapsed intervals, not processor utilization, pure kernel duration, or physical transfer duration.

### Actual GPU work

GPU work counters must be recorded per operation and per observed batch. Their scope and `purpose` must be explicit. Operation counters cover all compute-backend work caused by the operation; batch detail distinguishes image work from calibration. Telemetry overhead counters are separate. Presentation counters belong to the presentation owner.

`gpu.dispatch_count` counts `dispatch_workgroups` calls in submitted command buffers. It does not count workgroups, pixels, or commands discarded before submission. `gpu.submit_count` counts actual `Queue::submit` calls. `gpu.command_buffer_count` counts submitted command buffers. `gpu.host_wait_count` counts blocking host synchronization boundaries used to await device work. Multiple polling and receiving calls at one completion boundary count as one wait.

`gpu.upload_count` counts logical host-to-device writes of application buffers. `gpu.upload_bytes` counts their supplied payload bytes. Image, spectral/LUT, uniform, and scratch-initialization payloads must have separate categories. This includes initialized mapped buffers, such as `create_buffer_init`, and queue writes; resource allocation without a host payload is not an upload. Repeated writes count each actual write. Host data prepared for a discarded command without a device-visible write must not count as an upload.

`gpu.readback_count` counts result ranges materialized into host-owned data. `gpu.readback_bytes` counts the device representation bytes in those ranges. Materializing f32 GPU output into f64 host storage must not double the logical readback value; host precision expansion belongs to host materialization and copy accounting. Directly mapping a shared buffer still counts one logical readback.

`gpu.staging_copy_count` and `gpu.staging_copy_bytes` count device-to-device copies made to expose application results to the host. Other device copies, such as ping-pong initialization, must use a separate `gpu.device_copy_count` and `gpu.device_copy_bytes`. An output copied to staging and then materialized must count one staging copy and one logical readback, not two readbacks.

A resident image batch can have one image upload and one image readback while also uploading parameters and spectral resources. Its dispatch count is the number of dispatches in submitted command buffers. A per-stage route must count every observed batch and result boundary. CPU-only operations have available zero GPU work counters.

## Explicit GPU timing

`gpu.pass.duration` measures the beginning-to-end timestamp interval for one actual compute pass. Each observation identifies a stable pass name, occurrence, parent batch, and `purpose`. The pass names must identify the executed work, including front transform, blur axes, spectral integration, density interpolation, DIR, grain, and output processing where those passes execute.

`gpu.compute_pass_sum` is the sum of available compute-pass intervals within its stated scope. Coverage must identify executed, timed, valid, and omitted passes. The full-scope sum must be unavailable as `incomplete` when any executed pass lacks a valid timestamp; an observed subset may retain a separately labeled partial sum. It excludes queue residence before the first pass, commands outside the measured passes, host preparation, host wakeup, and host result construction. It must not be used to calculate GPU utilization or a CPU/GPU percentage of operation time.

Each timed report must identify whether timestamp support was available, whether the device enabled it, and the timestamp period in nanoseconds per tick. Query allocation, resolution, and query result copies must have separate `telemetry` work counters. An unavailable pass time must not be replaced with `submit_to_map_ready`.

## Supporting measurements

Supporting measurements are optional. Each must report availability and scope. They must not delay the required diagnostic measurements.

`gpu.pipeline_cache_hit_count` and `gpu.pipeline_cache_miss_count` count actual compute-pipeline cache lookups caused by the operation. `gpu.pipeline_create.duration` measures the synchronous API interval that creates a pipeline on a miss. It must not claim to measure all driver compilation. A cache hit does not prove that resources or the Runtime were reused. Benchmark context must state which Runtime, backend, process, and driver cache lifetimes were reused; the report must not infer a universal `warm` flag.

`gpu.buffer_create_count` and `gpu.buffer_create_bytes` count GPU API buffer creations and requested sizes caused by the operation. Reused resources must not count as new creations. These measurements distinguish repeated allocation from reuse; they must not be labeled VRAM usage or physical memory residency.

`host.image_copy_count` and `host.image_copy_bytes` count instrumented deep copies of pixel storage. A shared `Arc` clone is not an image copy. Copies made during input preparation, simulation, and saving must identify their owner. This counter must be marked partial unless all image-copy sites in its stated scope are instrumented. It is not a count of allocator activity.

CPU time, RSS, physical GPU memory, and instruction hotspots remain external profiler observations. In a concurrent GUI, process CPU time and RSS describe the process rather than one operation. CPU time can exceed wall time when threads run concurrently; it must not be inferred from host time minus GPU timestamps.

A sampled process-memory maximum must state its sampling interval and must not be called an exact operation peak. Process high-water marks must identify their lifetime. Driver-managed GPU memory and device utilization require a platform source. These observations are outside the local operation metric schema.

## Comparison and aggregation

A diagnostic comparison must hold input pixels, dimensions, effective parameters, seed, saving settings, and collection mode constant. It must state adapter type/API, build identity, host precision, process lifetime, Runtime/backend reuse, and the number and ordering of iterations. Device identification in a local report must not imply comparability across drivers.

Aggregate durations may use seconds histograms. Work totals may use monotonic counters across operations. An individual operation's report contains interval counts, not a process-lifetime counter. Allowed aggregate dimensions are operation kind, outcome, selected backend, execution path, precision, stage/pass category, purpose, and stable reason code. An instrument must use only the subset needed for its comparison. Correlation IDs, exact dimensions, filenames, device names, stock names, arbitrary settings, and error strings must not become labels.

Percentiles require a stated sample count and one defined population. Cold setup and repeated execution must not be merged into an unexplained average. Throughput may be derived from simulation working pixels divided by its simulation duration; it must be labeled simulation throughput and excluded when duration is zero or unavailable. It must not be presented as end-to-end export throughput.

A useful diagnosis first identifies the executed route and CPU stages, then compares stage wall time, batch waits, and logical boundary counts. GPU pass durations can distinguish expensive device passes when explicitly collected. CPU instruction hotspots, hardware transfer bandwidth, and device utilization require independent profilers.

The implementation must define one typed report model and deterministic validator for the JSON contract. Validation must reject nonfinite durations, negative counts, missing parents, duplicate identifiers, invalid availability/value combinations, and completed child intervals outside their completed parent scope. Schema fixtures must include failed-before-load, CPU, resident, mixed Grain V1, Save without source diagnostics, and truncated reports. Round-trip validation must preserve a counter above 2^53. Validation and serialization must not parse human log wording.
