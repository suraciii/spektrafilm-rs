# Telemetry metrics

## Record semantics

A report describes one operation as defined in the [contract](../spec.md). Ownership and timing boundaries are defined in the [design](../design.md).

- The report format is one UTF-8 JSON object with `schema_version` equal to `1`. Its root fields are `schema_version`, `build`, `operation`, `source_render`, `configuration`, `execution`, `measurements`, `attempts`, `phases`, `stages`, `gpu_batches`, `diagnostic_issues`, and `coverage`.
- `source_render` is present only for Save. Numeric measurements MUST use JSON numbers, never formatted strings, NaN, or infinity.
- Integer counters MUST preserve u64 values through serialization and parsing. `build` identifies the application version and source revision when available.
- `operation` contains a session-local `id`, stable `kind`, `outcome`, and optional `input_revision` and `parameter_revision`. Report readers MUST ignore unknown fields within a supported schema version.
- A change to a field's meaning or unit requires a new schema version.
- Each measurement has a `status`: `available`, `unavailable`, or `not_applicable`. An `available` measurement contains a finite numeric `value`.
- Other statuses contain a stable `reason` and no value. Zero means measured absence of work or elapsed time; it never means missing instrumentation.
- Duration units are seconds (`s`), byte units are bytes (`By`), and counts are nonnegative integers. Unexecuted phases MUST be omitted rather than fabricated as zero-duration work.
- `phases`, `stages`, and `gpu_batches` are flat arrays of observations with unique local identifiers, a `parent_id`, and a stable `name`. Every retained child's parent MUST be retained.
- Attempts parent to the operation; phases parent to the operation, attempt, or enclosing phase; stages parent to a phase or enclosing stage; batches parent to a stage or phase; GPU passes parent to a batch. Repeated names MUST retain distinct occurrences.
- Host observations use `start_offset` relative to operation acceptance and inclusive `duration`, both in seconds. CPU and GPU clocks MUST NOT be compared as if they shared an epoch.
- GPU pass durations are measurements attached to their batch, without host start offsets derived from device ticks.
- `attempts` is empty for Save and contains indexed simulation attempts for rendering operations. Each attempt owns its route, simulation observations, and GPU batches.
- An attempt that failed before simulation MUST identify that boundary. The root execution summary MUST identify differing attempt routes instead of selecting the last route silently.
- Invocation totals aggregate actual work from all attempts; final saving phases reference only the saved attempt. Details beyond the record limit follow [collection limits](../design.md#collection-and-signals).
- `coverage` identifies `detail_limit`, retained and omitted observation counts, and truncation. Fixed-category totals MUST remain available even when occurrence details are omitted.
- Omitted or truncated observations do not establish an unobserved stage's executor or prove complete computation coverage.
- `coverage.omitted_high_level_observations` counts omitted attempts, phases, and stages, including children of omitted parents. Its value MUST NOT exceed the total omitted observation count.
- Zero means that every emitted high-level observation was retained, even when batch or pass detail was truncated. Zero does not prove that every numerical stage was instrumented or that an observation's executor was recorded.
- Missing or `null` means that high-level retention coverage is unknown, including historical reports that did not distinguish observation kinds. A nonzero value means that high-level executor coverage is incomplete.
- Readers MUST require explicit executors and the expected computation scope before claiming full-GPU execution.

`coverage.truncated` MUST be true when observations are omitted. It MAY also be true when retained scopes lose bounded measurement or routing detail at the byte limit. A true value does not require a nonzero omitted observation count.

- A terminal outcome closes the operation's total duration. Incomplete intervals on failure MUST identify themselves as incomplete; they MUST NOT be presented as completed stage timings.
- An unavailable duration requires a reason such as `not_collected`, `unsupported`, `query_failed`, or `counter_overflow`. Partial or truncated detail MUST NOT be treated as a complete distribution.
- Observed configuration fields MUST be omitted when unresolved. `configuration.unavailable_fields` MUST identify each unresolved required field and its reason.
- Save MUST describe retained pixels and saving configuration; it MUST NOT copy current controls as if they produced the retained render. `source_render` MUST include the producing operation identifier, input and parameter revisions, dimensions, and `diagnostics_collected`; it MUST NOT retain a full parameter snapshot.
- Stable kinds are `preview`, `scan`, `save`, `export`, `process`, and `render`.
- `diagnostic_issues` contains bounded stable categories for unavailable collection and report warnings. Image errors use `input`, `configuration`, `backend`, `simulation`, `writing`, `publication`, or `cancelled` with their observed boundary.
- Recoverable diagnostic issues do not rewrite operation outcomes. Build identifiers, device descriptions, and bundled identifiers MUST be bounded to 256 Unicode characters per value; truncated descriptions MUST be marked.
- Unknown user-defined identifiers MUST use `custom`, not a path-derived name. At most 64 issue entries MAY be retained, with exact omitted issue counts.
- Default JSON reports MUST NOT embed human log messages or arbitrary `tracing` fields.

## Live progress records

The progress journal is a separate UTF-8 JSONL document. It MUST NOT change the final report schema or replace the final report's exact counters and detailed observations.

The file MUST retain one immutable begin line followed by 127 circular record slots. Every line MUST occupy 1,024 bytes, including JSON whitespace padding and its final newline; writing new snapshots MUST reuse slots rather than grow the file without a bound.

| Field | Meaning |
| --- | --- |
| `schema_version` | Progress format version, currently `1`, independent of the final report schema |
| `seq` | Monotonically increasing snapshot sequence within the operation |
| `operation_id` | Correlation identifier of the final report's operation |
| `observation_root_id` | Root identifier for backend observation ancestry |
| `operation_kind` | Stable operation kind from the report vocabulary |
| `event` | `begin`, `progress`, or `end` |
| `elapsed_seconds` | Finite monotonic wall time since operation acceptance, in seconds |
| `current_phase` | Current observed coarse phase name, or `null` |
| `current_stage` | Current observed coarse stage name, or `null` |
| `last_begin` | Most recent observed scope-begin name, or `null` |
| `last_end` | Most recent observed scope-end name, or `null` |
| `outcome` | Stable terminal outcome for `end`, otherwise `null` |
| `dropped_events` | Number of events omitted by bounded progress collection |

Scope names MUST remain bounded and allowlisted. Progress MUST NOT include image data, paths, arbitrary messages, or complete parameter snapshots.

Readers MUST order retained records by `seq`, not their physical file positions. They MUST ignore malformed or partially written crash records. Completion requires a valid `end` record with a non-null `outcome`; otherwise readers MUST present the operation as incomplete.

A changed current scope MUST reflect observed begin and end events. Dropped events indicate incomplete progress coverage; elapsed time and coarse scopes MUST NOT be interpreted as sampled CPU utilization, isolated kernel time, or proof of complete executor coverage.

## Effective configuration
- `configuration` contains `collection_mode_requested` and `collection_mode_effective` (`summary` or `gpu_timing`), operation input dimensions, simulation working dimensions, and published output dimensions when a file was published. GPU timing can be only partially available across batches; pass coverage MUST describe that separately.
- It MUST also contain host scalar precision (`f32` or `f64`) and GPU arithmetic precision (`f32` when GPU work executed). Spectral preparation that uses f64 MUST remain identified independently of host scalar precision.
- For rendering operations, the configuration MUST contain workflow route, bundled film and print stock identifiers when applicable, RGB-to-raw algorithm, input/output color-space roles, transfer decode/encode flags, requested spectral LUT flags, active gamut algorithm, and effective grain engine and mode. Effective grain settings MUST include whether grain ran and whether Spektra used the fast statistics sampler.
- For the opt-in scalar-record law, `configuration.render.grain.record` MUST contain `law` (`scalar_fixed_contribution`), `event_contribution_d_um2` in `D·µm²`, `support_um`, `cell_um`, the full u64 `seed`, `expectation`, and `optical_order` (`formed_density_then_cell_response_then_fractional_integration`). The render owner MUST resolve these effective inputs after parameter calibration with the same resolver used for rendering.
- `grain.record` MUST be omitted for other laws. Historical schema-version-1 reports without it MUST remain readable; its absence does not identify a scalar-record realization.
- Record configuration MUST use typed, allowlisted values, not arbitrary environment text.
- Effective spatial settings MUST identify active lens blur, halation, optical diffusion, DIR, glare, and unsharp processing. Saving configuration MUST contain format, depth, compression, saving color space, and saving transfer encoding when a write occurred.
- Save MUST include the retained pixels' color space and encoding; simulation settings absent from its small source reference MUST be unavailable as `source_configuration_not_retained`.
- Rendering configuration MUST identify `scan_output` as `direct_scan` or `negative_to_positive`. Negative conversion MUST identify the resolved readout illuminant and the material-category rule that selected it, plus the scan provenance of the neutral scan model: the colorimetric sensor, the clear-film white reference, the output space white, and the identity scan matrix.
- Record the existing `negative_to_positive_output` reason only when negative conversion actually executes on CPU and that condition declined the historical resident route; GPU execution MUST NOT carry a forced CPU or resident-decline reason. The report MUST preserve the selected backend identity when the pipeline executes an explicit CPU override.
- Scan provenance MUST identify the neutral scan model and version. A static state declaration MUST NOT claim prepared scan identities before preparation.
- Render provenance MUST identify the model/version implementing the [optical-order cutover](../../film-chain/spec.md#optical-order-cutover). Effective stage descriptions MUST distinguish scanner readout blur, digital unsharp mask, print viewing glare, material grain, and Procedural post-scan appearance.
- Fused execution MUST retain those semantic boundaries in its declared model even when it has no separate timing observation per operation. A declaration alone MUST NOT be reported as measured stage-order acceptance.
- Rendering configuration MUST retain `output_mix.amount` and identify whether final output composition ran or was bypassed for an intermediate tap under [Output mix](../../output-mix/spec.md#output-mix). Source-branch rejection MUST be an operation failure, not a reported successful amount-1 render.
- Preflash eligibility follows the actual print-development route; its controls retain existing parameter provenance. No digital-response definition, baseline/effective response identity, or new calibration pass is required.
- Save MUST NOT invent settings absent from its retained source reference.
- `execution` contains `backend_requested` (`auto`, `cpu`, or `wgpu`), `backend_selected` (`cpu` or `wgpu`), and an attempt `path` (`cpu`, `gpu_resident`, or `per_stage`). The root path is `mixed` when attempt paths differ.
- These values retain their schema-version-1 meanings. Unresolved execution fields MUST be listed in `execution.unavailable_fields` with reasons.
- Save's render backend and path are not applicable, regardless of the current GUI backend. WGPU execution MUST identify compute adapter API, device type, and name; presentation identifies a separate device only when that device information is observed.
- Adapter device types distinguish `cpu`, `integrated`, `discrete`, `virtual`, and `other`; a CPU adapter MUST NOT be described as hardware GPU evide…
- A `per_stage` record MUST include each executed stage's `executor` (`cpu`, `gpu`, or `mixed`) and `purpose` (`image` or `calibration`). A composite Spektra grain stage with actual CPU sampling and GPU blur MUST be `mixed`, with a CPU sampler observation and separate GPU batches; a GPU grain implementation MUST be reported as GPU work.
- `gpu_resident` retains its historical meaning: the fused chain route, not proof that all image computation ran on the GPU. It does not claim that input numerics, preparation, post-scan handling, or saving ran on the GPU.
- It MUST NOT be reinterpreted or silently expanded to assert complete GPU computation. Save's path is `not_applicable` because it does not render.
- Host orchestration, cache lookup, image-independent coefficient preparation, and other host work MUST NOT be reported as CPU image arithmetic. An actual CPU image stage MUST be identified from executor observations, not inferred from backend selection, an effect or engine choice, a host duration, or missing GPU detail.
- Full-GPU acceptance MUST retain evidence for the complete [computation scope](../../gpu-execution/spec.md#computation-scope) and [acceptance](../../gpu-execution/spec.md#acceptance), including RAW numerical processing after decode, image and device grain preparation, negative conversion, saving conversion, and viewer/display work. Schema-version-1 reports MAY be accompanied by this scope evidence.
- A report that lacks actual executor evidence for a scope leaves that scope unknown. High-level retention coverage follows `coverage.omitted_high_level_observations`; omitted batch or pass detail alone MUST NOT invalidate retained stage executor evidence.
- Device identity, dispatches, `gpu_resident`, and absence of a CPU reason are not substitutes for executor evidence.
- The retained input route MUST enclose its actual source-transfer decode, color matrix, and destination-transfer encode in the canonical `colorspace_convert` stage. Bind the device, queue, and pipe to that stage's context so submitted dispatches, resource work, and inherited purpose belong to the conversion, not only its parent operation.
- GPU executor attribution MUST follow actual nonzero submitted dispatches; upload-only, copy-only, or no-op work MUST NOT acquire a GPU numerical executor from backend selection alone. The conversion MUST NOT introduce an image upload or readback between its device passes; initial source upload and final result materialization remain separate boundaries.
- `execution.resident_decline_reasons` contains stable codes from the routing authority, including workflow, chemistry, input transfer, spectral LUT, optical diffusion, Spektra grain, gamut support, blur support, front-pass availability, and backend support. Each code MUST identify the effective condition that declined the route.
- A stage's CPU implementation MUST have a separate reason when its own GPU implementation was declined. Normal CPU selection MUST NOT be called a fallback.
- A tap request that bypasses the resident route MUST be identifiable as a diagnostic route.

Stable availability reasons are `not_collected`, `not_reached`, `not_applicable`, `unsupported`, `feature_request_failed`, `not_enabled_on_device`, `query_failed`, `not_ready`, `invalid_timestamp`, `detail_limit`, `incomplete`, `counter_overflow`, and `source_configuration_not_retained`. Resident-decline codes are `workflow_route`, `negative_to_positive_output`, `langmuir_chemistry`, `input_transfer_decoding`, `requested_spectral_lut`, `active_optical_diffusion`, `faithful_grain_distribution`, `unsupported_output_gamut`, `blur_radius_exceeds_backend_support`, `missing_resident_front_pass`, `backend_no_resident_support`, `device_resource_limits`, `diagnostic_tap_route`, `dye_grain_field_cpu`, and `fotufilm_film_cpu`. Human labels MUST be mapped from these codes; arbitrary error strings MUST NOT supply a code.

## Required diagnostic measurements

These measurements answer where an operation spent time and whether work crossed the host/device boundary repeatedly.

### Operation and phase wall time
- `operation.duration` is wall time from acceptance of an executable request to its terminal outcome. CLI acceptance occurs after argument parsing and before command-specific validation; parser usage failures do not create image-operation records.
- GUI acceptance occurs after required file choices are complete and before operation-specific validation or worker dispatch. Its scope includes worker wait and final acceptance, publication, or machine-result preparation.
- It excludes user interaction before acceptance and report persistence afterward.
- Phase names are `input_load`, `backend_init`, `input_prepare`, `runtime_prepare`, `simulation`, `display_prepare`, `presentation_handoff`, `saving_convert`, `file_write`, `metadata`, `publication`, and `result_prepare`. A phase is recorded only when that work occurs.
- `worker_wait` measures worker dispatch to worker start; it is a host scheduling observation and MUST NOT be called GPU queue wait. Input snapshot copies belong to `input_prepare`.
- Post-publication output hashing and machine-result assembly belong to `result_prepare` and remain inside invocation duration.
- `simulation` encloses the Runtime processing call, including metering, geometry, scan preparation, the executed film chain, and output post-scan handling. Input loading and Runtime construction belong to separate phases.
- A GUI image load completed before render acceptance MUST NOT be charged again to Preview or Scan. Stage observations identify `metering`, `geometry`, `filming_expose`, `filming_develop`, `converting`, `grain_spektra`, `grain_procedural`, `grain_dye_field`, `grain_fotufilm`, `printing`, `scanning`, `negative_to_positive`, and `post_scan` when separately observable.
- Historical reports MAY retain the `color_reference` stage name; current execution MUST NOT invent that removed preparation stage. A fused chain MUST report its batch rather than invent independent host stage durations for passes within it.
- `file_write` includes native pixel conversion, encoding/compression, and metadata work when the writer does not expose them separately. A separately observed `metadata` duration is a child of `file_write`; it MUST NOT be added to that inclusive duration.
- A nonfatal metadata warning MUST remain a warning on a successful pixel write. `publication` ends after the destination has been published or publication has failed.
- `presentation_handoff` measures result acceptance and submission to the GUI presentation system; it MUST NOT claim display scanout latency. A display raster recomputed during GUI result acceptance MUST record a separate `display_prepare` occurrence.

### Backend batch wall time
- Each backend batch MUST report `host_prepare`, `submit_to_map_ready`, `host_materialize`, and total batch duration. Their boundaries follow [routing and timing](../design.md#routing-and-timing).
- Values MUST come from direct monotonic observations rather than subtracting rounded log values. A backend operation with no host result readback MUST identify the completion boundary it actually observed.
- `host_prepare` explains conversion, setup, and command-encoding cost. `submit_to_map_ready` explains the interval in which the host awaited a result.
- `host_materialize` explains mapped data copying and result construction. These are elapsed intervals, not processor utilization, pure kernel duration, or physical transfer duration.

### Actual GPU work
- GPU work counters MUST be recorded per operation and per observed batch. Their scope and `purpose` MUST be explicit.
- Operation counters cover all compute-backend work caused by the operation; batch detail distinguishes image work from calibration. Telemetry overhead counters are separate.
- Presentation counters belong to the presentation owner.
- `gpu.dispatch_count` counts `dispatch_workgroups` calls in submitted command buffers. It does not count workgroups, pixels, or commands discarded before submission.
- `gpu.submit_count` counts actual `Queue::submit` calls. `gpu.command_buffer_count` counts submitted command buffers.
- `gpu.host_wait_count` counts blocking host synchronization boundaries used to await device work. Multiple polling and receiving calls at one completion boundary count as one wait.
- `gpu.upload_count` counts logical host-to-device writes of application buffers. `gpu.upload_bytes` counts their supplied payload bytes.
- Image, spectral/LUT, uniform, and scratch-initialization payloads MUST have separate categories. This includes initialized mapped buffers, such as `create_buffer_init`, and queue writes; resource allocation without a host payload is not an upload.
- Repeated writes count each actual write. Host data prepared for a discarded command without a device-visible write MUST NOT count as an upload.

Queue writes to uniform buffers MUST count as uniform uploads, not image uploads. Uploading retained image samples MUST NOT count as a readback; only materializing device results into host-owned data crosses that boundary.

- `gpu.readback_count` counts result ranges materialized into host-owned data. `gpu.readback_bytes` counts the device representation bytes in those ranges.
- Materializing f32 GPU output into f64 host storage MUST NOT double the logical readback value; host precision expansion belongs to host materialization and copy accounting. Directly mapping a shared buffer still counts one logical readback.

`gpu.staging_copy_count` and `gpu.staging_copy_bytes` count device-to-device copies made to expose application results to the host. Other device copies, such as ping-pong initialization, MUST use a separate `gpu.device_copy_count` and `gpu.device_copy_bytes`. An output copied to staging and then materialized MUST count one staging copy and one logical readback, not two readbacks.

- A resident image batch can have one image upload and one image readback while also uploading parameters and spectral resources. Its dispatch count is the number of dispatches in submitted command buffers.
- A per-stage route MUST count every observed batch and result boundary. CPU-only operations have available zero GPU work counters.

## Explicit GPU timing

`gpu.pass.duration` measures the beginning-to-end timestamp interval for one actual compute pass. Each observation identifies a stable pass name, occurrence, parent batch, and `purpose`. The pass names MUST identify the executed work, including front transform, blur axes, spectral integration, density interpolation, DIR, grain, and output processing where those passes execute.

- `gpu.compute_pass_sum` is the sum of available compute-pass intervals within its stated scope. Coverage MUST identify executed, timed, valid, and omitted passes.
- The full-scope sum MUST be unavailable as `incomplete` when any executed pass lacks a valid timestamp; an observed subset MAY retain a separately labeled partial sum. It excludes queue residence before the first pass, commands outside the measured passes, host preparation, host wakeup, and host result construction.
- It MUST NOT be used to calculate GPU utilization or a CPU/GPU percentage of operation time.

Each timed report MUST identify whether timestamp support was available, whether the device enabled it, and the timestamp period in nanoseconds per tick. Query allocation, resolution, and query result copies MUST have separate `telemetry` work counters. An unavailable pass time MUST NOT be replaced with `submit_to_map_ready`.

## Supporting measurements

Supporting measurements are optional. Each MUST report availability and scope. They MUST NOT delay the required diagnostic measurements.

- `gpu.pipeline_cache_hit_count` and `gpu.pipeline_cache_miss_count` count actual compute-pipeline cache lookups caused by the operation. `gpu.pipeline_create.duration` measures the synchronous API interval that creates a pipeline on a miss.
- It MUST NOT claim to measure all driver compilation. A cache hit does not prove that resources or the Runtime were reused.
- Benchmark context MUST state which Runtime, backend, process, and driver cache lifetimes were reused; the report MUST NOT infer a universal `warm` flag.

`gpu.buffer_create_count` and `gpu.buffer_create_bytes` count GPU API buffer creations and requested sizes caused by the operation. Reused resources MUST NOT count as new creations. These measurements distinguish repeated allocation from reuse; they MUST NOT be labeled VRAM usage or physical memory residency.

- `host.image_copy_count` and `host.image_copy_bytes` count instrumented deep copies of pixel storage. A shared `Arc` clone is not an image copy.
- Copies made during input preparation, simulation, and saving MUST identify their owner. This counter MUST be marked partial unless all image-copy sites in its stated scope are instrumented.
- It is not a count of allocator activity.

CPU time, RSS, physical GPU memory, and instruction hotspots remain external profiler observations. In a concurrent GUI, process CPU time and RSS describe the process rather than one operation. CPU time can exceed wall time when threads run concurrently; it MUST NOT be inferred from host time minus GPU timestamps.

- A sampled process-memory maximum MUST state its sampling interval and MUST NOT be called an exact operation peak. Process high-water marks MUST identify their lifetime.
- Driver-managed GPU memory and device utilization require a platform source. These observations are outside the local operation metric schema.

## Comparison and aggregation

A diagnostic comparison MUST hold input pixels, dimensions, effective parameters, seed, saving settings, and collection mode constant. It MUST state adapter type/API, build identity, host precision, process lifetime, Runtime/backend reuse, and the number and ordering of iterations. Device identification in a local report MUST NOT imply comparability across drivers.

- Aggregate durations MAY use seconds histograms. Work totals MAY use monotonic counters across operations.
- An individual operation's report contains interval counts, not a process-lifetime counter. Allowed aggregate dimensions are operation kind, outcome, selected backend, execution path, precision, stage/pass category, purpose, and stable reason code.
- An instrument MUST use only the subset needed for its comparison. Correlation IDs, exact dimensions, filenames, device names, stock names, arbitrary settings, and error strings MUST NOT become labels.
- Percentiles require a stated sample count and one defined population. Cold setup and repeated execution MUST NOT be merged into an unexplained average.
- Throughput MAY be derived from simulation working pixels divided by its simulation duration; it MUST be labeled simulation throughput and excluded when duration is zero or unavailable. It MUST NOT be presented as end-to-end export throughput.

A useful diagnosis first identifies the executed route and CPU stages, then compares stage wall time, batch waits, and logical boundary counts. GPU pass durations can distinguish expensive device passes when explicitly collected. CPU instruction hotspots, hardware transfer bandwidth, and device utilization require independent profilers.

- The implementation MUST define one typed report model and deterministic validator for the JSON contract. Validation MUST reject nonfinite durations, negative counts, missing parents, duplicate identifiers, invalid availability/value combinations, and completed child intervals outside their completed parent scope.
- Schema fixtures MUST include failed-before-load, CPU, resident, mixed Spektra grain, Save without source diagnostics, and truncated reports. Round-trip validation MUST preserve a counter above 2^53.
- Validation and serialization MUST NOT parse human log wording.

## Grain naming compatibility

New stage and reason names MUST identify Spektra, Procedural, or Dye.
Readers MUST accept the historical numbered stage and reason names in schema-one reports and normalize them to canonical names.
This migration MUST NOT accept arbitrary stage names outside the existing privacy allowlist.
