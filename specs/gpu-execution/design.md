# GPU execution design

## Ownership

The [product contract](spec.md) owns execution policy and scope. The calibrated runtime must remain the production computation entry point. Film, scanner, grain, input, output, and viewer owners must retain their domain rules. GPU acceleration must not introduce a second photographic parameter schema or a second interpretation of those rules.

Stage preparation must produce one resolved semantic configuration for CPU and GPU. It must retain the full reference values and data identity until an explicitly defined numerical boundary. Backend-specific buffers and packed uniforms are representations of that configuration, not independent sources of defaults, calibration, or capability decisions.

The CPU implementation owns reference evaluation. The WGPU backend owns device resources, kernel dispatch, device error handling, and synchronization. The runtime owns stage ordering and complete-operation validation. Input and output adapters own file boundaries. Viewer code owns disposable presentation state, not a second color model.

The execution policy must be resolved once per operation from the accepted backend request and available adapter. Lower layers must not select CPU through trait defaults or effect-specific fallback. A GPU capability check must cover the complete operation before dispatch where the limitation is knowable. Mid-operation failures must preserve publication guarantees.

The stage sequence is carried by an internal pipe. Nodes are assembled per operation from the resolved parameters and selected backend under [pipe assembly](#pipe-assembly). A generic public effect graph, new backend, or backend-specific serialized model is still not required.

## Preparation boundary

The host may parse and validate profiles, resolve controls, select reference data, check dimensions, and derive bounded scalar coefficients or small matrices in f64. This host work must not evaluate image samples. Image-independent validation may use reference arithmetic to decide whether a model is well-defined.

Display-transform preparation follows the [GUI display color-management boundary](../gui-workflow/design.md#display-color-management).

Gamut validation must check controls and destination compatibility without constructing numerical boundary tables or initializing CPU reference Cmax caches. The selected backend must own numerical boundary construction during preparation.

Numerical calibration grids, spectral lookup generation, full-source statistics, spatial field formation, stochastic population preparation, and image-sized intermediate construction must have GPU implementations. CPU-generated image or grain data must not be relabeled as cached preparation to satisfy GPU coverage. A cache hit does not prove where its content was computed.

CPU and GPU preparation must derive the same calibrated model. Scanner preparation must retain the owning spectral grid and clear-film white reference. The negative-density inverse must retain profile-derived monotone density-to-log-exposure knots, finite endpoint slopes, and per-record independence under the [film-chain design](../film-chain/design.md). The convert-film solver must keep its bounded solve and stopping policy.

Cache identity must include resolved parameters, source data identities, coordinate geometry where relevant, precision representation, and device ownership. Backend selection must not mutate accepted controls. An image revision or device change must invalidate resources whose contents depend on it.

Runtime construction must pass the selected backend into pipeline calibration and numerical table preparation. A prepared spectral table must retain its device identity and f32 and f64 representations for its consumers without a full-table host readback. Only the three-component f64 neutral response needed by scalar exposure calibration may return to the host. CPU-selected construction must retain the CPU reference preparation.

Native RAW unpacking must expose the sensor samples and metadata needed by the selected decoding algorithm. GPU numerical processing must retain that algorithm's supported sensor modes, demosaicing, corrections, orientation, white balance, and color assignment. Moving only the final ACES transform to GPU must not count as GPU RAW processing.

## Device residency

Device image ownership must represent dimensions, sample domain, color and transfer state, working pixel pitch, and lifetime. Stages must consume that representation without obtaining host pixels implicitly. CPU reference execution may use host images under the same domain contracts.

The operation must retain device ownership through input numerical preparation, film and scan work, output composition, and final saving or display conversion. Save must consume retained output in its actual storage domain without rerendering. A final host materialization for the native writer or a diagnostic output is an explicit handoff, not an internal stage boundary. A selected GPU operation must not materialize an intermediate on the host to execute CPU image arithmetic.

Named diagnostic taps use the same typed `Tap` boundaries on CPU and GPU: `rgb_pre` is the prepared input before filming exposure, `log_e_film` includes the complete exposure and logarithm, `cmy_film` includes development, DIR and density grain, `log_e_print` includes complete print exposure, `cmy_print` includes paper development, and `rgb_out` includes the scanner and finishing tail. `rgb_in` preprocessing remains with the retained runtime. Injection binds the supplied carrier at its declared boundary and omits earlier nodes; collection returns the already-computed boundary buffer and omits later nodes. Logarithmic exposure and density are distinct retained sample domains, not RGB. Grain readout dimensions, pitch and deferred regional crops follow the selected collection's downstream spatial context. A same-tap request returns the original carrier unchanged. Only the requested diagnostic artifact may materialize at the caller boundary.

The public host-source `Runtime::process_with_taps` entry uses this retained path for every selected-GPU tap span, not only `rgb_in` through `rgb_out`. Its upload is the source boundary and its sole image materialization is the requested collection. A missing retained operation fails without entering the CPU tap topology; diagnostic operation telemetry must not label this device route as a per-stage CPU execution.

Retained processing must record working dimensions from the carrier after working geometry, before the film chain. A full-source field therefore reports its source working lattice, not its final cropped or resampled output. The execution path must describe a reached retained route; successful geometry-only and input-only spans are resident routes even when they do not execute a film chain. Backend and adapter identity come from the bound execution backend.

A viewer must consume resident image data when compute and presentation devices can share it. Separate devices may require a final presentation transfer. Presentation must not download an image for CPU resampling or ICC evaluation and upload that computed raster as if it were full-GPU viewing. Software WGPU must remain visibly identified.

Multiple submissions, ping-pong buffers, GPU reductions, and tiles are valid resident execution. Device memory residency must not be equated with one command buffer, one binding, or physical VRAM counters. Logical transfer accounting follows [telemetry metrics](../telemetry/metrics/spec.md#actual-gpu-work).

Cancellation belongs to the render operation, not the shared device. Retained entry points bind the pipeline's shared atomic signal to a derived backend; observation and batch derivations preserve that signal without changing device identity or cancelling sibling renders. Bounded work checks the signal before submission and propagates `operation cancelled` as an ordinary error. Cancellation must not panic or publish a partial result.

Render display preparation and export saving conversion must derive their backend with the owning request's cancellation signal, including when observation is disabled. Each viewer composition must own a separate signal. A new viewer request or worker shutdown must cancel active composition before waiting for the worker. Completed fit data may remain reusable, but incomplete presentations must not replace accepted pixels. A new operation must replace or clear an inherited signal without resetting another operation's token.

A full-source reduction must retain the specified source population before crop or resize. GPU reductions must preserve the reference measurement law and supply device-side scanner or exposure interpretation. Only bounded control results and validation flags may return to host orchestration.

Retained RGB images carry full dimensions and absolute pixel-indexed device pages. Upload and final materialization traverse those pages; intermediate extraction and output assembly copy page intersections on the device. `SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES` may lower the physical binding limit to exercise the same paging route. Full-source exposure metering gathers reflected source support from those pages, streams bounded binary64 vertical blocks one preview row at a time, and preserves vertical-then-horizontal tap order and the reference 256-pixel preview cap. The complete preview luminance grid feeds the existing global pattern reduction; only the final eight-byte EV returns to the host. The preview grid and fixed reduction coefficients remain the metering minimum working set.

## Pipe assembly

The stage sequence is carried by an internal pipe. Each numerical stage is a node: a `Gpu` node dispatches a registered shader over device buffers and never reads host pixels; a `Cpu` node wraps the CPU reference stage for a CPU-selected operation. Node selection happens at assembly time from the resolved parameters and selected backend. A GPU-selected operation must not assemble a `Cpu` image-stage node. An inactive feature does not enter the pipe, create a pass-through node, or appear in decline accounting. Fixed stage ordering, image-domain transitions, and the runtime's ordering ownership are preserved exactly; only which nodes exist is dynamic. Compute pipelines are created lazily per shader identity and held for the process lifetime; device buffers are reused through the pipe's pools and a cross-frame cache, and buffer lifetime must respect command-completion rules.

## Selected-backend execution

Automatic selection may select the complete CPU path only when no usable hardware GPU is available under [backend selection](spec.md#backend-selection). After GPU selection, a missing kernel, unsupported parameter, precision limitation, resource limit, or device failure must fail the operation before publication. This rule applies to automatic and explicit GPU requests. The operation must not insert a CPU fence or restart on CPU. A user may explicitly retry with CPU.

Gaussian blur has no CPU fallback exception for small sigma or wide support on a selected GPU operation. It must preserve the reference kernel on the device or fail explicitly. A reduction node, for example metering, may return only bounded control results and validation flags to host orchestration; its image-dependent interpretation remains on the device.

A missing GPU implementation or a required CPU intermediate is an execution limitation, not a reason to select another photographic route or hide CPU arithmetic in a fence. Workflow and output-gamut selection must retain the accepted controls. A supported configuration must gain a complete GPU implementation to satisfy acceptance.

## Numerical alignment

Each stage must define its reference input and output domains and its permitted arithmetic differences. The CPU f64 model is the oracle; CPU native-f32 distinguishes host rounding from algorithm changes. A GPU representation must identify any binary16 composition, f32 arithmetic, integer arithmetic, or extended-precision computation that affects its result.

The executor must preserve per-channel coefficients, normalization, wavelength grids, kernel support, interpolation continuity, spectral missing-value policy, extrapolation, and unclipped headroom. An implementation may fuse linear operations only after stage and integrated evidence proves agreement under the frozen budget. A nonlinear operation must not move across a spatial filter or domain transform.

Retained RGB color conversion follows the CPU `convert_rgb` law: decode the source transfer when requested, apply the source-to-destination matrix exactly once, then encode the destination transfer when requested. When separate device passes implement that sequence, an encoding pass that also accepts a matrix must receive identity after the conversion matrix has already run. Preserve signed and out-of-gamut samples without clipping; neither storage partitioning nor observation binding changes the conversion.

Spatial implementation must retain the same convolution law, support and boundary extension. CPU IIR and GPU FIR must not be assumed equivalent because both are called Gaussian. They must have measured agreement over the declared parameter domain or use an aligned implementation. DIR exponential tails and channel-specific halation must not be approximated away. Device limits must be handled through exact decomposition, bounded processing, or explicit failure.

Floating tolerances must be frozen per stage and complete scenario using the [acceptance rules](spec.md#acceptance). They must protect endpoints, dark-region behavior, signed samples, neutral preservation, continuity, optical profiles, and spatial correlation, not only average final RGB error. File-encoding tolerance must remain separate from floating computation tolerance.

A device without native f64 does not waive reference-sensitive arithmetic. The GPU implementation must retain the decisions its owning engine requires, in that engine's declared arithmetic or in validated integer or extended-precision arithmetic, or fail explicitly. A capability limitation is not permission for a CPU image-stage fallback or an approximate model. No universal f64 shader-support requirement is added to WGPU.

An accepted CPU algorithm must not be changed merely to make GPU results pass. If an alternative numerical implementation also changes CPU behavior, it must preserve the frozen reference under the same domain budgets. A changed photographic model requires a separately approved contract change.

## Stochastic alignment

Each grain or glare owner retains its existing generator, random distributions, seed interpretation, stream ordering or coordinate identity, and sampling domain. Alignment must preserve those rules rather than replace them with a convenient parallel generator. Different engines must not share a new generic noise approximation.

For stream-based sampling, GPU parallelization must map logical random draws to the same reference stream positions. It must preserve the reference consumption of variable-length rejection sampling; scheduling must not shift draws to different samples. For coordinate-based sampling, the same global coordinates, layer and channel identity, seed, and footprint must address the same random values, unless the owning engine defines its appearance as independent of the reference's individual draws, in which case the address must still be a pure function of those coordinates. Chunking, crop, queue order, and workgroup size must not reseed or redefine a realization.

For identical reference conditioning values, uniform random words, integer counters, count-law branches, Poisson and binomial decisions, and other model-defined discrete choices must match the reference exactly where the owning algorithm defines them. Where a decision quantity is transcendental, the owning engine's design must define the decision-agreement budget that stands in for unattainable identity: the measured device error, the reference's own rounding noise, the demonstrated disagreement rate, and the evidence that establishes each. An owning engine whose appearance model does not depend on draw identity may instead define the device's arithmetic precision and its random-word source, provided the law itself -- branches, boundary, rejection test, and tail -- is the reference's, provided its statistical evidence is established against a control build that replaces the law, and provided the design states both. Dye grain is such an engine. An unqualified claim of identical decisions on hardware without native f64 is not admissible, and neither is a budget widened after a measurement. Reference f64 parameters must not be narrowed before a stochastic decision when the owning engine forbids it. The GPU must preserve the reference conditioning needed for those decisions; native-f32 host conditioning is a separately identified precision comparison, not a substitute oracle. Floating formation, accumulation and readout must satisfy the owning engine's certified domain budgets in addition to the shared alignment checks.

Mean, variance, spectrum, correlation, and photographic grain appearance checks must supplement sample and discrete-decision checks. Equal statistics with a different count law, clipped tail, conditional allocation, or grain population must fail algorithm alignment. Source-library parity and CPU/GPU alignment are distinct comparisons with identified references.

## Bounded execution

The GPU backend must validate buffer arithmetic, binding sizes, workgroup limits, storage requirements and the minimum working set before allocating known workloads. It must plan bounded device resources for both small images and full photographic frames. Resource planning must not rely on a full microscopic grain canvas when the owning field algorithm permits bounded readout.

Spatial tiles must include the context required by the actual kernel and route. Grain tiles must retain absolute film coordinates and actual rounded output footprints. Global measurements must use the complete input once and remain invariant under tile selection. Intermediate GPU storage may be reused only after its commands complete.

Spektra raster storage windows preserve the original flattened pixel identity for fast statistics. Sequential MT19937 retains each plane's generator across windows and completes the complete-source Poisson walk before the binomial walk; a storage boundary never reseeds a plane. Multiplicative grain USM reduces complete-source mass before applying normalization. Procedural grain gathers its original-coordinate conditioning samples on the device, retains the original dimensions in the resolution model, and carries tap accumulation rather than shortening support when a spatial footprint exceeds a binding. Image, layer, Poisson-seed, and mass-reduction transports are physically paged; no complete-source raster or complete layer stack must fit one physical GPU buffer.

A tile boundary must not change random assignment, endpoint policy, source conditioning, spectral aperture readout, or optical order. Whole-frame and tiled outputs must pass identical budgets. Data-dependent iteration must preserve reference stopping and error policy and must terminate within the owning algorithm's bounds.

Cancellation must be checked before additional chunks, immediately before each additional submission or readback, and before publication. This includes histogram pages, saving-conversion pages, and viewer preparation and presentation pages. An in-flight GPU pass may finish. A readback that has already entered the driver's blocking wait cannot be interrupted; cancellation must stop the next submission or stage after that wait returns. Results from cancelled, failed, or superseded operations must not enter retained output or destination files. Releasing an operation must not invalidate resources still referenced by submitted work.

## Verification ownership

Stage owners must supply domain and discrete-invariant comparisons. Runtime owners must prove integrated route and parameter composition. Frontends must prove real operations, saving attribution and stale-result behavior. The backend must prove actual device execution, resource bounds and transfer accounting. Telemetry must preserve its existing wire meanings and distinguish host orchestration from CPU image arithmetic.

Acceptance evidence must join these scopes through immutable input, parameter, data, executable and adapter identities. A legacy `gpu_resident` report covers its historical fused-film scope; it must not be reinterpreted as evidence for RAW, preparation, display or saving coverage. Missing or truncated observations must remain insufficient evidence. This design does not add a telemetry schema or rename existing execution enums.

Production cutover requires the [complete acceptance](spec.md#acceptance), not successful shader compilation or a new interface with unimplemented stages. Tests and retained evidence must cover plausible output and policy failures. Scaffolds, CPU pass-through implementations labeled GPU, and mixed-stage execution must not satisfy the full-GPU target.
