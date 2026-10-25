# Telemetry

## Contract

Telemetry explains the cost and execution of one image operation. An operation is one Preview, Scan, Save, Export, CLI process invocation, or CLI render invocation. A CLI invocation with repeated simulation MUST retain each attempt as a distinct child observation with an index, duration, and outcome.

It MUST produce one invocation terminal record and attribute the final write only to the output attempt that was saved. Save MUST reference the retained render that produced its pixels.

Collection modes are `off`, `summary`, and `gpu_timing`. GUI collection MUST default to `summary` with automatic local report writing. CLI collection MUST default to `off` unless explicitly requested.

Existing diagnostic output MUST remain available without a remote service. Enabling telemetry MUST NOT change image parameters, random seeds, precision, adapter selection, fallback policy, or publication behavior.

Each collected operation MUST have one terminal record. The record MUST identify its operation kind and outcome. Outcomes are `succeeded`, `failed`, `cancelled`, and `superseded`.

A GUI result rejected because its captured input or parameters became stale MUST be `superseded`. A successful Export record MUST include final publication. A failed or cancelled operation MUST retain measurements completed before termination.

The record MUST distinguish the requested backend, selected compute backend, actual execution path, and numerical precision. A selected WGPU adapter MUST NOT imply that all stages ran on the GPU. Software adapters MUST be identified as software adapters.

GUI presentation and film simulation MUST identify separate devices when they use separate devices.

When the historical fused resident route is evaluated and declined, the record MUST identify why. The record MUST identify which image stages actually used CPU execution. A CPU reason MUST identify CPU image work that ran; backend selection, effect selection, host orchestration, or image-independent preparation alone MUST NOT imply CPU image execution.

A request for an unsupported measurement MUST report its availability. An unavailable measurement MUST NOT be represented as zero or estimated from another duration.

## Local surfaces

CLI telemetry MUST use a separate local report destination. It MUST NOT add text to machine-readable stdout. Human diagnostic events MUST use stderr.

GUI diagnostics MUST expose the operation summary and allow explicit local report export. Ordinary GUI status MUST remain short and follow [status surface ownership](../gui-workflow/spec.md#labels-and-surfaces). Viewer rendering status MUST NOT depend on telemetry collection or extend an operation's terminal boundary.

The GUI MUST expose `Diagnostics` in the Settings dialog under [Stage and surface ownership](../gui-workflow/spec.md#stage-and-surface-ownership). It MUST provide a collection-mode selector, the current operation's collection state, recent completed operations, and `Save report…` for the selected completed operation. Changing the mode MUST affect only operations accepted after the change.

It MUST NOT trigger a render or change an in-flight operation. Diagnostics mode and reports MUST remain session-local and outside saved GUI states, presets, and recipes.

GUI sessions MUST start recording before the initial image request. Users MUST NOT need to choose a folder or collection mode to obtain performance reports.

The application MUST create a distinct persistent report directory for each GUI session. Starting another session MUST NOT overwrite reports from an earlier session.

Diagnostics MUST offer `Open log folder` for the current session. Collection modes, report history, and individual report export MUST remain available under initially collapsed advanced details.

Automatic writing MUST keep the interface responsive. Normal shutdown MUST finish accepted report writes. Diagnostics settings MUST remain session-local and outside saved GUI states, presets, and recipes.

The GUI summary MUST show operation kind, outcome, working dimensions, elapsed time, requested and selected backend, actual route, and the largest observed phase. It MUST show CPU-stage reasons and software-adapter status in plain language. Phase durations, logical transfers, and GPU pass timing MUST be available in expanded detail.

The UI MUST NOT rank nested phases as independent costs or diagnose an unmeasured shader bottleneck from a host wait. An empty history MUST explain that collection applies to the next operation; exporting a report MUST NOT rerun the image.

The GUI MUST retain the latest 20 terminal reports in memory, newest first. It MUST label superseded operations separately from accepted output. The retained output MUST keep a small render reference independently of history eviction.

Save MUST identify that source render and state whether its diagnostic report was collected. Closing the application MUST discard report history; closing during work MUST finalize reachable cancellation observations before releasing worker-owned records. Abrupt termination MAY leave only an incomplete progress journal.

Automatic writing MUST publish each collected terminal report as a new `operation-<id>.json` file in the session directory. `SPEKTRAFILM_GUI_DIAGNOSTICS_DIR` MAY override the automatic directory for controlled launches without changing the collection default.

Publication MUST NOT replace an existing file, MUST NOT change the operation outcome, and MUST NOT enter the operation duration. A publication failure MUST surface as a status diagnostic without failing the operation.

The `process` and `render` CLI commands MUST accept `--diagnostics PATH` for one UTF-8 JSON report. `--gpu-timings` MUST require `--diagnostics` and select GPU timing; a report request without it selects summary. Existing `process --timings` MUST keep its human timing output and MUST NOT require a report path.

`--help` MUST describe these options and explain that GPU timing MAY be unavailable. Other subcommands MUST NOT accept these options.

The report path MUST name a regular local file, not stdout, a pipe, or a device. Its parent directory MUST exist. It MUST NOT alias an input, recipe, parameter, image-output, or raw-output file used by that invocation.

A protected-path collision or invalid option combination MUST be rejected before image work. Other report persistence failures MUST emit a clear stderr diagnostic without changing the image command's existing exit outcome. Successful report writes MUST be acknowledged on stderr.

CLI report publication MUST create a new file atomically and MUST NOT replace an existing destination, including one created by another process after validation. GUI report saving MAY replace an existing file only after the normal save-dialog overwrite confirmation. Report persistence MUST NOT affect image publication.

Final report serialization and writing occur after the operation terminal boundary and MUST NOT enter image operation duration. Live progress persistence follows [Live progress](#live-progress).

A report MUST contain a schema version, application build identity, operation identity, effective execution configuration, dimensions, timing observations, GPU work observations, and measurement availability. Report fields and units are defined in [metrics](metrics/spec.md). Operation identifiers MUST be local correlation values, not persistent user or machine identifiers.

A failure before loading, parameter resolution, or device selection MUST retain only the configuration observed before failure. Required fields that were not reached MUST report availability rather than guess their values.

Summary collection MUST include operation duration, phase durations, execution path, actual routing and CPU-stage reasons, logical GPU transfers, submissions, and dispatches. GPU timestamp collection MUST be explicit. CPU profiling, driver profiling, and operating-system memory sampling are separate measurements; summary collection MUST NOT claim to supply them.

Reports MUST remain valid for historical and explicit CPU execution, WGPU resident execution, and WGPU per-stage execution with CPU or GPU stages. Historical mixed-path validity MUST NOT authorize CPU image stages or a CPU restart after GPU selection under the [backend-selection contract](../gpu-execution/spec.md#backend-selection). Save MUST report writing costs without claiming a new simulation.

A report MUST distinguish render output configuration from file saving configuration.

## Live progress

Each collected operation MUST expose a bounded local progress journal before its image work starts. The journal MUST remain readable while the operation is running, including when image work stalls before a terminal report exists.

| Surface | Progress destination |
| --- | --- |
| CLI `--diagnostics FILE` | Append `.progress.jsonl` to the full report filename |
| GUI automatic recording | `operation-<id>.json.progress.jsonl` in the session report directory |
| Collection `off` | No progress file or writer |

Progress MUST identify the operation and elapsed time. It MUST expose the current coarse phase and stage, the most recent scope begin and end, and a terminal outcome when observed. It MUST NOT claim instruction-level sampling or GPU completion that was not observed.

Progress collection MUST NOT wait for file I/O or an external consumer on image threads. Overload MUST retain bounded storage and expose dropped events instead of blocking image work.

The progress destination MUST obey the report destination's local-file and protected-path rules. A protected alias or invalid destination MUST be rejected before CLI image work. Creating a progress file MUST NOT follow a symlink or replace an existing destination.

Progress I/O failures MUST produce a separate frontend diagnostic without changing the image outcome or preventing final report publication. A CLI progress failure MUST use stderr and MUST NOT alter machine-readable stdout.

Normal operation completion MUST drain terminal progress before publishing the final report. Normal shutdown MUST flush accepted progress writes. Missing terminal progress MUST mean incomplete observation, never successful completion.

Progress journals MUST use the privacy allowlist and [progress record semantics](metrics/spec.md#live-progress-records). A completed report remains the authoritative source for measured totals and detailed observations.

## Privacy and failure

Telemetry MUST remain local. Collection MUST NOT start a network connection or install a remote exporter.

Automatic GUI recording MUST follow [Local surfaces](#local-surfaces). Other report exports MUST require a user action or an explicit CLI request.

Default reports MUST exclude image pixels, image or profile content hashes, absolute paths, filenames, EXIF/IPTC/XMP, camera serial numbers, and complete parameter snapshots. They MAY include bundled stock identifiers and the effective algorithm settings required by [metrics](metrics/spec.md). Error records MUST use stable error categories; user data in error messages MUST NOT enter default reports.

Recoverable collection and report persistence failures MUST NOT fail an image operation or change its destination publication. They MUST produce a separate diagnostic failure or unavailable measurement. Device loss, invalid image command submission, and process-wide allocation failure remain real execution failures; they MUST NOT be hidden as successful image operations.

Collection MUST NOT block on an external consumer.

## Acceptance

With identical inputs and effective parameters, enabling summary collection MUST preserve output pixels and publication outcomes. Enabling GPU timestamps MUST preserve the same image passes and execution route.

Resident and per-stage scenarios MUST show different observed GPU boundary counts. A Grain V1 or film-coordinate grain scenario that actually executes CPU image sampling MUST identify that CPU work and its reason. The reason MUST include a Dye field stage or `grain_fotufilm` when applicable.

Selecting an engine alone MUST NOT force a CPU-stage reason or resident decline.

A Grain V2 resident scenario MUST identify the actual dispatched grain pass. GPU execution of supported grain stages MUST be reported as GPU work. CPU-only execution MUST report zero observed GPU work and unavailable GPU timestamp measurements.

Telemetry acceptance MUST also verify negative conversion with neutral and user-authored per-channel styles, scanner lens blur, and unsharp masking against their actual executors. GPU execution MUST be reported as GPU work without a forced CPU reason. CPU execution MUST retain its applicable existing reason.

Historical CPU and mixed reports remain valid. Full-GPU evidence MUST cover all in-scope image computation, including RAW numerical stages after native decoding, image and grain preparation, negative conversion, saving conversion, and viewer/display transforms. A report's `gpu_resident` path, backend identity, declarations, or dispatches alone MUST NOT establish this coverage.

Reports MUST be paired with explicit executor evidence for covered work; unobserved or truncated work remains unproven. Follow the [GPU execution contract](../gpu-execution/spec.md#computation-scope), its [backend selection](../gpu-execution/spec.md#backend-selection), [algorithm alignment](../gpu-execution/spec.md#algorithm-alignment), and [acceptance](../gpu-execution/spec.md#acceptance) rules, and the [device residency](../gpu-execution/design.md#device-residency), [numerical alignment](../gpu-execution/design.md#numerical-alignment), and [stochastic alignment](../gpu-execution/design.md#stochastic-alignment) design.

Concurrent GUI render and export operations MUST keep separate records. Superseded previews MUST NOT replace the diagnostics attached to the accepted output. Cancellation and write failure MUST produce the correct terminal record without publishing a successful Export record.

A device without timestamp support MUST still produce its summary. Timestamp results MUST be checked against a known dispatched pass. Logical transfer counts MUST be checked for both staging and directly mapped readback paths.

CLI report output MUST preserve existing machine-readable stdout.

Acceptance MUST exercise switching modes while a GUI operation is in flight, recent-history eviction, Save from uncollected retained output, CPU and unsupported timestamp devices, and report export without rendering. CLI acceptance MUST cover existing report destinations, protected-path aliases, unwritable report destinations, failed image processing, repeated attempts, and unchanged machine stdout and image exit outcome.
