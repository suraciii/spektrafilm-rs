# GPU execution design

## Ownership

The [product contract](spec.md) owns execution policy and scope. The calibrated runtime MUST remain the production computation entry point. Film, scanner, grain, input, output, and viewer owners MUST retain their domain rules.

GPU acceleration MUST NOT introduce a second photographic parameter schema or a second interpretation of those rules.

Stage preparation MUST produce one resolved semantic configuration for CPU and GPU. It MUST retain the full reference values and data identity until an explicitly defined numerical boundary. Backend-specific buffers and packed uniforms are representations of that configuration, not independent sources of defaults, calibration, or capability decisions.

The CPU implementation owns reference evaluation. The WGPU backend owns device resources, kernel dispatch, device error handling, and synchronization. The runtime owns stage ordering and complete-operation validation.

Input and output adapters own file boundaries. Viewer code owns disposable presentation state, not a second color model.

The execution policy MUST be resolved once per operation from the accepted backend request and available adapter. Lower layers MUST NOT select CPU through trait defaults or effect-specific fallback. A GPU capability check MUST cover the complete operation before dispatch where the limitation is knowable.

Mid-operation failures MUST preserve publication guarantees.

The stage sequence is carried by an internal pipe. Nodes are assembled per operation from the resolved parameters and selected backend under [pipe assembly](#pipe-assembly). A generic public effect graph, new backend, or backend-specific serialized model is still not required.

## Preparation boundary

The host MAY parse and validate profiles, resolve controls, select reference data, check dimensions, and derive bounded scalar coefficients or small matrices in f64. This host work MUST NOT evaluate image samples. Image-independent validation MAY use reference arithmetic to decide whether a model is well-defined.

Display-transform preparation follows the [GUI display color-management boundary](../gui-workflow/design.md#display-color-management).

Gamut validation MUST check controls and destination compatibility without constructing numerical boundary tables or initializing CPU reference Cmax caches. The selected backend MUST own numerical boundary construction during preparation.

Numerical calibration grids, spectral lookup generation, full-source statistics, spatial field formation, stochastic population preparation, and image-sized intermediate construction MUST have GPU implementations. CPU-generated image or grain data MUST NOT be relabeled as cached preparation to satisfy GPU coverage. A cache hit does not prove where its content was computed.

CPU and GPU preparation MUST derive the same calibrated model. Scanner preparation MUST retain the owning spectral grid and clear-film white reference. The negative-density inverse MUST retain profile-derived monotone density-to-log-exposure knots, finite endpoint slopes, and per-record independence under the [film-chain design](../film-chain/design.md).

The convert-film solver MUST keep its bounded solve and stopping policy.

Cache identity MUST include resolved parameters, source data identities, coordinate geometry where relevant, precision representation, and device ownership. Backend selection MUST NOT mutate accepted controls. An image revision or device change MUST invalidate resources whose contents depend on it.

Runtime construction MUST pass the selected backend into pipeline calibration and numerical table preparation. A prepared spectral table MUST retain its device identity and f32 and f64 representations for its consumers without a full-table host readback. Only the three-component f64 neutral response needed by scalar exposure calibration MAY return to the host.

CPU-selected construction MUST retain the CPU reference preparation.

Native RAW unpacking MUST expose the sensor samples and metadata needed by the selected decoding algorithm. GPU numerical processing MUST retain that algorithm's supported sensor modes, demosaicing, corrections, orientation, white balance, and color assignment. Moving only the final ACES transform to GPU MUST NOT count as GPU RAW processing.

## Device residency

Device image ownership MUST represent dimensions, sample domain, color and transfer state, working pixel pitch, and lifetime. Stages MUST consume that representation without obtaining host pixels implicitly. CPU reference execution MAY use host images under the same domain contracts.

The operation MUST retain device ownership through input numerical preparation, film and scan work, output composition, and final saving or display conversion. Save MUST consume retained output in its actual storage domain without rerendering. A final host materialization for the native writer or a diagnostic output is an explicit handoff, not an internal stage boundary.

An operation MUST hand off its owned final image pages without duplicating the image solely for publication. Crop extraction MAY allocate a separate carrier. Later operations MUST NOT modify retained input or previously published output.

Dye density readout pages MUST directly supply downstream density consumers. Optional cell-spectral scan response MAY retain a separate RGB destination.

A selected GPU operation MUST NOT materialize an intermediate on the host to execute CPU image arithmetic.

Named diagnostic taps use the same typed `Tap` boundaries on CPU and GPU:

| Tap | Boundary |
| --- | --- |
| `rgb_pre` | Prepared input before filming exposure |
| `log_e_film` | Complete exposure and logarithm |
| `cmy_film` | Development, DIR, and density grain |
| `log_e_print` | Complete print exposure |
| `cmy_print` | Paper development |
| `rgb_out` | Scanner and finishing tail |

`rgb_in` preprocessing remains with the retained runtime.

Injection binds the supplied carrier at its declared boundary and omits earlier nodes; collection returns the already-computed boundary buffer and omits later nodes. Logarithmic exposure and density are distinct retained sample domains, not RGB. Grain readout dimensions, pitch and deferred regional crops follow the selected collection's downstream spatial context.

A same-tap request returns the original carrier unchanged. Only the requested diagnostic artifact MAY materialize at the caller boundary.

The public host-source `Runtime::process_with_taps` entry uses this retained path for every selected-GPU tap span, not only `rgb_in` through `rgb_out`. Its upload is the source boundary and its sole image materialization is the requested collection. A missing retained operation fails without entering the CPU tap topology; diagnostic operation telemetry MUST NOT label this device route as a per-stage CPU execution.

Retained processing MUST record working dimensions from the carrier after working geometry, before the film chain. A full-source field therefore reports its source working lattice, not its final cropped or resampled output. The execution path MUST describe a reached retained route; successful geometry-only and input-only spans are resident routes even when they do not execute a film chain.

Backend and adapter identity come from the bound execution backend.

A viewer MUST consume resident image data when compute and presentation devices can share it. Separate devices MAY require a final presentation transfer. Presentation MUST NOT download an image for CPU resampling or ICC evaluation and upload that computed raster as if it were full-GPU viewing.

Software WGPU MUST remain visibly identified.

Multiple submissions, ping-pong buffers, GPU reductions, and tiles are valid resident execution. Device memory residency MUST NOT be equated with one command buffer, one binding, or physical VRAM counters. Logical transfer accounting follows [telemetry metrics](../telemetry/metrics/spec.md#actual-gpu-work).

Cancellation belongs to the render operation, not the shared device. Retained entry points bind the pipeline's shared atomic signal to a derived backend; observation and batch derivations preserve that signal without changing device identity or cancelling sibling renders. Bounded work checks the signal before submission and propagates `operation cancelled` as an ordinary error.

Cancellation MUST NOT panic or publish a partial result.

Render display preparation and export saving conversion MUST derive their backend with the owning request's cancellation signal, including when observation is disabled. Each viewer composition MUST own a separate signal. A new viewer request or worker shutdown MUST cancel active composition before waiting for the worker.

Completed fit data MAY remain reusable, but incomplete presentations MUST NOT replace accepted pixels. A new operation MUST replace or clear an inherited signal without resetting another operation's token.

A full-source reduction MUST retain the specified source population before crop or resize. GPU reductions MUST preserve the reference measurement law and supply device-side scanner or exposure interpretation. Only bounded control results and validation flags MAY return to host orchestration.

Retained RGB images carry full dimensions and absolute pixel-indexed device pages. Upload and final materialization traverse those pages; intermediate extraction and output assembly copy page intersections on the device. `SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES` MAY lower the physical binding limit to exercise the same paging route.

Full-source exposure metering gathers reflected source support from those pages, streams bounded f32 vertical blocks one preview row at a time, and preserves vertical-then-horizontal tap order and the reference 256-pixel preview cap. The complete preview luminance grid feeds the existing global pattern reduction; only the final f32 EV returns to the host.

The preview grid and fixed reduction coefficients remain the metering minimum working set.

## Pipe assembly

The stage sequence is carried by an internal pipe. Each numerical stage is a node: a `Gpu` node dispatches a registered shader over device buffers and never reads host pixels; a `Cpu` node wraps the CPU reference stage for a CPU-selected operation. Node selection happens at assembly time from the resolved parameters and selected backend.

A GPU-selected operation MUST NOT assemble a `Cpu` image-stage node. An inactive feature does not enter the pipe, create a pass-through node, or appear in decline accounting. Fixed stage ordering, image-domain transitions, and the runtime's ordering ownership are preserved exactly; only which nodes exist is dynamic.

Compute pipelines are created lazily per shader identity and held for the process lifetime; device buffers are reused through the pipe's pools and a cross-frame cache, and buffer lifetime MUST respect command-completion rules.

## Selected-backend execution

Automatic selection MAY select the complete CPU path only when no usable hardware GPU is available under [backend selection](spec.md#backend-selection). After GPU selection, a missing kernel, unsupported parameter, precision limitation, resource limit, or device failure MUST fail the operation before publication. This rule applies to automatic and explicit GPU requests.

The operation MUST NOT insert a CPU fence or restart on CPU. A user MAY explicitly retry with CPU.

Gaussian blur has no CPU fallback exception for small sigma or wide support on a selected GPU operation. It MUST preserve the reference kernel on the device or fail explicitly. A reduction node, for example metering, MAY return only bounded control results and validation flags to host orchestration; its image-dependent interpretation remains on the device.

A missing GPU implementation or a required CPU intermediate is an execution limitation, not a reason to select another photographic route or hide CPU arithmetic in a fence. Workflow and output-gamut selection MUST retain the accepted controls. A supported configuration MUST gain a complete GPU implementation to satisfy acceptance.

## Numerical alignment

Each stage MUST define its reference input and output domains and its permitted arithmetic differences. The CPU f64 model is the oracle; CPU native-f32 distinguishes host rounding from algorithm changes. A GPU representation MUST identify any binary16 composition, f32 arithmetic, integer arithmetic, or extended-precision computation that affects its result.

The executor MUST preserve per-channel coefficients, normalization, wavelength grids, kernel support, interpolation continuity, spectral missing-value policy, extrapolation, and unclipped headroom. An implementation MAY fuse linear operations only after stage and integrated evidence proves agreement under the frozen budget. A nonlinear operation MUST NOT move across a spatial filter or domain transform.

Retained RGB color conversion follows the CPU `convert_rgb` law: decode the source transfer when requested, apply the source-to-destination matrix exactly once, then encode the destination transfer when requested. When separate device passes implement that sequence, an encoding pass that also accepts a matrix MUST receive identity after the conversion matrix has already run.

Preserve signed and out-of-gamut samples without clipping; neither storage partitioning nor observation binding changes the conversion.

Spatial implementation MUST retain the same convolution law, support and boundary extension. CPU IIR and GPU FIR MUST NOT be assumed equivalent because both are called Gaussian. They MUST have measured agreement over the declared parameter domain or use an aligned implementation.

DIR exponential tails and channel-specific halation MUST NOT be approximated away. Device limits MUST be handled through exact decomposition, bounded processing, or explicit failure.

Floating tolerances MUST be frozen per stage and complete scenario using the [acceptance rules](spec.md#acceptance). They MUST protect endpoints, dark-region behavior, signed samples, neutral preservation, continuity, optical profiles, and spatial correlation, not only average final RGB error. File-encoding tolerance MUST remain separate from floating computation tolerance.

A device without native f64 does not waive reference-sensitive arithmetic. The GPU implementation MUST retain the decisions its owning engine requires, or fail explicitly.

The declared arithmetic is native binary64, a stage-declared f32 kernel under its frozen budget, or validated integer or extended-precision arithmetic. A capability limitation is not permission for a CPU image-stage fallback or an approximate model. [Devices without native f64](#devices-without-native-f64) owns which stages MAY declare f32.

No universal f64 shader-support requirement is added to WGPU.

An accepted CPU algorithm MUST NOT be changed merely to make GPU results pass. If an alternative numerical implementation also changes CPU behavior, it MUST preserve the frozen reference under the same domain budgets. A changed photographic model requires a separately approved contract change.

## Devices without native f64

A stage whose law and discrete decisions the owning engine defines in f32 MAY run on a device without `SHADER_F64` under its frozen budget. Each such stage keeps the reference law, stage order, coefficients, tap order, kernel support, paging decomposition and coordinate identity; only the arithmetic precision changes. The measured device error and the frozen limit are:

| Stage | Metric | Measured | Frozen limit |
| --- | --- | --- | --- |
| Auto-exposure metering | EV against CPU f64 | 7.6e-5 | 2e-4 |
| Gaussian on a restricted device, through blurred glare | model output | 3.0e-9 | 1e-8 |
| Spline resize | scaled sample against CPU f64 | 1.9e-6 | 4e-6 |
| Direct scanner acquisition | RGB against CPU f64 | 6.6e-7 | 1e-5 |
| Negative readout and setup | log10 exposure / tone | 8.5e-7 / 3.6e-7 | 1e-5 / 1e-5 |
| Output gamut, modes 0-3 | RGB against CPU reference | 1.2e-6 | 5e-6 |
| Optical diffusion | sample against CPU f64 | 1.3e-6 | 5e-5 |
| Enlarger/scanner spectral LUT | sample against CPU f64 | 3.8e-8 / 7.0e-7 | 2e-5 |

`SPEKTRAFILM_GPU_DISABLE_F64=1` is an acceptance control, not a user-facing profile: it makes the adapter report no binary64 so the same device path is exercised. It MUST NOT select the CPU backend or change resolved controls.

Host-only preparation MUST stay outside image arithmetic and use the CPU reference in std f64 when the device cannot serve it: spectral transport tables, output-gamut boundary tables, and histogram bin boundaries. The resulting tables are uploaded; the device MUST NOT require f64 to consume them.

Stochastic engines whose appearance depends on reference-exact draw identity (procedural v1 and Spektra) still require native binary64 and 64-bit integers and MUST fail explicitly on such a device. The Dye engine's coordinate-addressed draw runs.

## Stochastic alignment

Each grain or glare owner retains its existing generator, random distributions, seed interpretation, stream ordering or coordinate identity, and sampling domain. Alignment MUST preserve those rules rather than replace them with a convenient parallel generator. Different engines MUST NOT share a new generic noise approximation.

For stream-based sampling, GPU parallelization MUST map logical random draws to the same reference stream positions. It MUST preserve the reference consumption of variable-length rejection sampling; scheduling MUST NOT shift draws to different samples.

For coordinate-based sampling, the same global coordinates, layer and channel identity, seed, and footprint MUST address the same random values. An owning engine MAY define its appearance as independent of the reference's individual draws; the address MUST still be a pure function of those coordinates.

Chunking, crop, queue order, and workgroup size MUST NOT reseed or redefine a realization.

For identical reference conditioning values, uniform random words, integer counters, count-law branches, Poisson and binomial decisions, and other model-defined discrete choices MUST match the reference exactly where the owning algorithm defines them.

Where a decision quantity is transcendental, the owning engine's design MUST define the decision-agreement budget that stands in for unattainable identity: the measured device error, the reference's own rounding noise, the demonstrated disagreement rate, and the evidence that establishes each.

An owning engine whose appearance model does not depend on draw identity MAY instead define the device's arithmetic precision and its random-word source, under these conditions:

- The law's branches, boundary, rejection test, and tail remain the reference's.
- Statistical evidence is established against a control build that replaces the law.
- The design states both the precision and random-word source.

Dye grain is such an engine. An unqualified claim of identical decisions on hardware without native f64 is not admissible, and neither is a budget widened after a measurement. Reference f64 parameters MUST NOT be narrowed before a stochastic decision when the owning engine forbids it.

The GPU MUST preserve the reference conditioning needed for those decisions; native-f32 host conditioning is a separately identified precision comparison, not a substitute oracle. Floating formation, accumulation and readout MUST satisfy the owning engine's certified domain budgets in addition to the shared alignment checks.

Mean, variance, spectrum, correlation, and photographic grain appearance checks MUST supplement sample and discrete-decision checks. Equal statistics with a different count law, clipped tail, conditional allocation, or grain population MUST fail algorithm alignment. Source-library parity and CPU/GPU alignment are distinct comparisons with identified references.

### Viewing glare device execution

Viewing glare MUST use the CPU reference's u32-truncated seed without an offset.

It MUST retain the row-major MT19937 word stream, the two-word uniform construction, Marsaglia polar rejection, cached normal pair, and `sigma < 1e-6` no-draw branch. Chunk boundaries MUST retain both generator state and the cached normal.

Viewing glare MAY evaluate conditioning and the normal transform with f32 arithmetic.

The device MUST NOT require native f64 or int64 for this stage.

The f32 path MUST preserve the reference law, seed interpretation, and word-consumption structure.

Floating results and near-boundary rejection decisions MUST satisfy measured agreement checks rather than an unqualified identity claim.

An insufficient generator-state binding MUST fail explicitly.

## Bounded execution

The GPU backend MUST validate buffer arithmetic, binding sizes, workgroup limits, storage requirements and the minimum working set before allocating known workloads. It MUST plan bounded device resources for both small images and full photographic frames. Resource planning MUST NOT rely on a full microscopic grain canvas when the owning field algorithm permits bounded readout.

Spatial tiles MUST include the context required by the actual kernel and route. Grain tiles MUST retain absolute film coordinates and actual rounded output footprints. Global measurements MUST use the complete input once and remain invariant under tile selection.

Intermediate GPU storage MAY be reused only after prior commands finish accessing it. Queue dependencies MAY establish this ordering without host waits.

Command-count limits MUST be paired with a bound on unfinished submissions and their retained transient resources. Work producers MUST apply completion backpressure before exceeding that bound.

Partial resource retirement MUST NOT mark a complete image batch ready.

Gaussian gathering and scattering MUST address target-channel samples on the device without changing sample bits. Copy-only or binding-incompatible storage MAY use bounded contiguous device copies for adaptation, but host commands MUST NOT be generated for individual pixels or filter taps.

Gaussian transport MUST preserve the reference support, endpoint policy, and axis order. Recursive filter state MUST continue through forward and backward page boundaries without restarting. Above sigma 8, CPU and device both use the reflected unit-sum box cascade with the same width selection.

Its frozen kernel error against the truncated Gaussian is 1.68e-3 at sigma 8, 3.33e-4 at 32, and 7.10e-5 at 147 of unit kernel mass. No smaller-sigma branch changes.

Gaussian page planning SHOULD balance serial coordinate work against independent lines. A complete supported raster that fits binding, coordinate, and line-state limits SHOULD remain one page.

Sequential Gaussian jobs MAY share operation-owned intermediate and output pages sized for the largest active geometry. Each job MUST retain its exact logical extent and keep live sources and combine accumulators distinct from reusable output.

Scratch reuse MUST preserve dependent reads and writes in queue order. Mutable scratch MUST NOT be shared between operations.

Source working pages MAY supply an equal-geometry tail after their final source read. Reuse MUST preserve live inputs and each pass's required source and destination separation.

Spektra raster storage windows preserve the original flattened pixel identity for fast statistics. Sequential MT19937 retains each plane's generator across windows and completes the complete-source Poisson walk before the binomial walk; a storage boundary never reseeds a plane. Multiplicative grain USM reduces complete-source mass before applying normalization.

Procedural grain gathers its original-coordinate conditioning samples on the device, retains the original dimensions in the resolution model, and carries tap accumulation rather than shortening support when a spatial footprint exceeds a binding. Image, layer, Poisson-seed, and mass-reduction transports are physically paged; no complete-source raster or complete layer stack MUST fit one physical GPU buffer.

A tile boundary MUST NOT change random assignment, endpoint policy, source conditioning, spectral aperture readout, or optical order. Whole-frame and tiled outputs MUST pass identical budgets. Data-dependent iteration MUST preserve reference stopping and error policy and MUST terminate within the owning algorithm's bounds.

Cancellation MUST be checked before additional chunks, immediately before each additional submission or readback, and before publication. This includes histogram pages, saving-conversion pages, and viewer preparation and presentation pages. An in-flight GPU pass MAY finish.

A readback that has already entered the driver's blocking wait cannot be interrupted; cancellation MUST stop the next submission or stage after that wait returns. Results from cancelled, failed, or superseded operations MUST NOT enter retained output or destination files. Releasing an operation MUST NOT invalidate resources still referenced by submitted work.

### Pointwise storage views

Pointwise execution MUST distinguish these access contracts:

| Access | Required sample preservation |
| --- | --- |
| Read | The source MUST remain unchanged. |
| Complete write | Every valid output sample MUST be written before handoff. Old output samples MUST NOT be gathered. |
| Read-modify-write | Each result MUST use the required previous sample value. |
| Partial write | Samples outside the dispatch's ownership MUST retain their previous values until their owning dispatch completes. |

Compatible storage pages MUST execute through direct binding views without image gathering or scattering. Copy-only or incompatible bindings MAY use bounded device adapters.

Each binding MUST retain its own aligned byte range and scalar residual. Work partitions MUST respect every participating page boundary and binding limit.

Host pointwise work MUST scale with actual view intersections, not individual pixels. Adapter capacity MUST remain independent of the hardware's maximum binding capacity.

Alias safety MUST cover reads across the complete pass, not only the current partition. Unsupported mappings MUST fail before the first output mutation.

Sparse curve and lattice dispatches MUST complete their output ownership before an adapted result is scattered or published.

### Operation resource ownership

The actual injection and collection span MUST determine the resource plan before large allocations. Stages outside that span MUST NOT construct execution resources.

Resource planning and execution MUST use the same canonical stage identities.

The plan MUST separate retained carriers and caches, live cross-stage data, temporary workspaces, and submitted resources awaiting retirement. Aliased views of one physical buffer MUST count once within an owner inventory.

Stage resources MUST be constructed when needed and released after their last required use. Shared Gaussian storage MUST follow the first and last active spatial consumer, not the complete chain lifetime.

Workspace plans MUST derive capacity from the active geometry and support. A conservative peak bound MUST NOT be described as a proven minimum working set.

An overlap scheduling target MUST NOT become a device capability limit. A larger valid operation MAY run exclusively rather than overlap another operation's workspace.

Shared-device derivatives MUST use one submission backpressure boundary. Cancellation and failure MUST preserve retirement ownership for already submitted work, including commands encoded before submission.

Resource credit MUST remain charged until the owning resources and submitted work permit its release. Queue completion MAY permit reuse without a host wait; dropping a state alone MUST NOT prove retirement.

Application owner capacity, native allocator usage, driver residency, and cumulative allocation traffic MUST remain distinct evidence classes. An unavailable native measurement MUST NOT be reported as zero usage.


## Verification ownership

Stage owners MUST supply domain and discrete-invariant comparisons. Runtime owners MUST prove integrated route and parameter composition. Frontends MUST prove real operations, saving attribution and stale-result behavior.

The backend MUST prove actual device execution, resource bounds and transfer accounting. Telemetry MUST preserve its existing wire meanings and distinguish host orchestration from CPU image arithmetic.

Acceptance evidence MUST join these scopes through immutable input, parameter, data, executable and adapter identities. A legacy `gpu_resident` report covers its historical fused-film scope; it MUST NOT be reinterpreted as evidence for RAW, preparation, display or saving coverage. Missing or truncated observations MUST remain insufficient evidence.

This design does not add a telemetry schema or rename existing execution enums.

Production cutover requires the [complete acceptance](spec.md#acceptance), not successful shader compilation or a new interface with unimplemented stages. Tests and retained evidence MUST cover plausible output and policy failures. Scaffolds, CPU pass-through implementations labeled GPU, and mixed-stage execution MUST NOT satisfy the full-GPU target.
