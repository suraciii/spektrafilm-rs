# Telemetry

## Contract

Telemetry explains the cost and execution of one image operation. An operation is one Preview, Scan, Save, Export, CLI process invocation, or CLI render invocation. A CLI invocation with repeated simulation must retain each attempt as a distinct child observation with an index, duration, and outcome. It must produce one invocation terminal record and attribute the final write only to the output attempt that was saved. Save must reference the retained render that produced its pixels.

Collection modes are `off`, `summary`, and `gpu_timing`. Collection must default to `off`. Users must be able to select summary or GPU timing for subsequent operations without restarting the GUI. Existing diagnostic output must remain available without a remote service. Enabling telemetry must not change image parameters, random seeds, precision, adapter selection, fallback policy, or publication behavior.

Each collected operation must have one terminal record. The record must identify its operation kind and outcome. Outcomes are `succeeded`, `failed`, `cancelled`, and `superseded`. A GUI result rejected because its captured input or parameters became stale must be `superseded`. A successful Export record must include final publication. A failed or cancelled operation must retain measurements completed before termination.

The record must distinguish the requested backend, selected compute backend, actual execution path, and numerical precision. A selected WGPU adapter must not imply that all stages ran on the GPU. Software adapters must be identified as software adapters. GUI presentation and film simulation must identify separate devices when they use separate devices.

The record must identify why resident execution was declined and which stages used CPU execution. A request for an unsupported measurement must report its availability. An unavailable measurement must not be represented as zero or estimated from another duration.

## Local surfaces

CLI telemetry must use a separate local report destination. It must not add text to machine-readable stdout. Human diagnostic events must use stderr. GUI diagnostics must expose the operation summary and allow explicit local report export. The normal status bar must remain a short operation status.

The GUI must expose a `Diagnostics` section in `CONFIG`. It must provide a collection-mode selector, the current operation's collection state, recent completed operations, and `Save report…` for the selected completed operation. Changing the mode must affect only operations accepted after the change. It must not trigger a render or change an in-flight operation. Diagnostics mode and reports must remain session-local and outside saved GUI states, look presets, and recipes.

The GUI summary must show operation kind, outcome, working dimensions, elapsed time, requested and selected backend, actual route, and the largest observed phase. It must show CPU-stage reasons and software-adapter status in plain language. Phase durations, logical transfers, and GPU pass timing must be available in expanded detail. The UI must not rank nested phases as independent costs or diagnose an unmeasured shader bottleneck from a host wait. An empty history must explain that collection applies to the next operation; exporting a report must not rerun the image.

The GUI must retain the latest 20 terminal reports in memory, newest first. It must label superseded operations separately from accepted output. The retained output must keep a small render reference independently of history eviction. Save must identify that source render and state whether its diagnostic report was collected. Closing the application must discard report history; closing during work must finalize reachable cancellation observations before releasing worker-owned records. Abrupt process termination need not produce a terminal report.

A GUI session launched with `SPEKTRAFILM_GUI_DIAGNOSTICS_DIR`, an existing directory, must publish every completed operation's report as a new `operation-<id>.json` file in that directory. The launch request selects summary collection, as a CLI report request does; the panel may still change the mode. Publication must not replace an existing file, must not change the operation outcome, and must not enter the operation duration. A publication failure must surface as a status diagnostic without failing the operation. The directory is launch configuration, not saved GUI state.

The `process` and `render` CLI commands must accept `--diagnostics PATH` for one UTF-8 JSON report. `--gpu-timings` must require `--diagnostics` and select GPU timing; a report request without it selects summary. Existing `process --timings` must keep its human timing output and must not require a report path. `--help` must describe these options and explain that GPU timing may be unavailable. Other subcommands must not accept these options.

The report path must name a regular local file, not stdout, a pipe, or a device. Its parent directory must exist. It must not alias an input, recipe, parameter, image-output, or raw-output file used by that invocation. A protected-path collision or invalid option combination must be rejected before image work. Other report persistence failures must emit a clear stderr diagnostic without changing the image command's existing exit outcome. Successful report writes must be acknowledged on stderr.

CLI report publication must create a new file atomically and must not replace an existing destination, including one created by another process after validation. GUI report saving may replace an existing file only after the normal save-dialog overwrite confirmation. Report persistence must not affect image publication. Serialization and report writing occur after the operation terminal boundary and must not enter image operation duration.

A report must contain a schema version, application build identity, operation identity, effective execution configuration, dimensions, timing observations, GPU work observations, and measurement availability. Report fields and units are defined in [metrics](metrics.md). Operation identifiers must be local correlation values, not persistent user or machine identifiers.

A failure before loading, parameter resolution, or device selection must retain only the configuration observed before failure. Required fields that were not reached must report availability rather than guess their values.

Summary collection must include operation duration, phase durations, execution path, fallback reasons, logical GPU transfers, submissions, and dispatches. GPU timestamp collection must be explicit. CPU profiling, driver profiling, and operating-system memory sampling are separate measurements; summary collection must not claim to supply them.

Reports must remain valid for CPU execution, WGPU resident execution, and WGPU per-stage execution with CPU stages. Save must report writing costs without claiming a new simulation. A report must distinguish render output configuration from file saving configuration.

## Privacy and failure

Telemetry must remain local. Collection must not start a network connection or install a remote exporter. Local report export must be a user action or an explicit CLI request.

Default reports must exclude image pixels, image or profile content hashes, absolute paths, filenames, EXIF/IPTC/XMP, camera serial numbers, and complete parameter snapshots. They may include bundled stock identifiers and the effective algorithm settings required by [metrics](metrics.md). Error records must use stable error categories; user data in error messages must not enter default reports.

Recoverable collection and report persistence failures must not fail an image operation or change its destination publication. They must produce a separate diagnostic failure or unavailable measurement. Device loss, invalid image command submission, and process-wide allocation failure remain real execution failures; they must not be hidden as successful image operations. Collection must not block on an external consumer.

## Acceptance

With identical inputs and effective parameters, enabling summary collection must preserve output pixels and publication outcomes. Enabling GPU timestamps must preserve the same image passes and execution route.

Resident and per-stage scenarios must show different observed GPU boundary counts. A Grain V1 scenario must identify its CPU grain stage even when WGPU is selected. A Grain V2 resident scenario must identify the actual dispatched grain pass. CPU-only execution must report zero observed GPU work and unavailable GPU timestamp measurements.

Concurrent GUI render and export operations must keep separate records. Superseded previews must not replace the diagnostics attached to the accepted output. Cancellation and write failure must produce the correct terminal record without publishing a successful Export record.

A device without timestamp support must still produce its summary. Timestamp results must be checked against a known dispatched pass. Logical transfer counts must be checked for both staging and directly mapped readback paths. CLI report output must preserve existing machine-readable stdout.

Acceptance must exercise switching modes while a GUI operation is in flight, recent-history eviction, Save from uncollected retained output, CPU and unsupported timestamp devices, and report export without rendering. CLI acceptance must cover existing report destinations, protected-path aliases, unwritable report destinations, failed image processing, repeated attempts, and unchanged machine stdout and image exit outcome.
