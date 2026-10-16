# GPU execution design

## Ownership

The [product contract](spec.md) owns execution policy and scope. The calibrated runtime must remain the production computation entry point. Film, scanner, grain, input, output, and viewer owners must retain their domain rules. GPU acceleration must not introduce a second photographic parameter schema or a second interpretation of those rules.

Stage preparation must produce one resolved semantic configuration for CPU and GPU. It must retain the full reference values and data identity until an explicitly defined numerical boundary. Backend-specific buffers and packed uniforms are representations of that configuration, not independent sources of defaults, calibration, or capability decisions.

The CPU implementation owns reference evaluation. The WGPU backend owns device resources, kernel dispatch, device error handling, and synchronization. The runtime owns stage ordering and complete-operation validation. Input and output adapters own file boundaries. Viewer code owns disposable presentation state, not a second color model.

The execution policy must be resolved once per operation from the accepted backend request and available adapter. Lower layers must not select CPU through trait defaults or effect-specific fallback. A GPU capability check must cover the complete operation before dispatch where the limitation is knowable. Mid-operation failures must preserve publication guarantees.

The stage sequence is carried by an internal pipe: nodes are assembled per operation from the resolved parameters, and stages without a GPU node become explicit CPU fence nodes on the automatic route under [pipe assembly](#pipe-assembly) and [fence semantics](#fence-semantics). A generic public effect graph, new backend, or backend-specific serialized model is still not required.

## Preparation boundary

The host may parse and validate profiles, resolve controls, select reference data, check dimensions, and derive bounded scalar coefficients or small matrices in f64. This host work must not evaluate image samples. Image-independent validation may use reference arithmetic to decide whether a model is well-defined.

Numerical calibration grids, spectral lookup generation, full-source statistics, spatial field formation, stochastic population preparation, and image-sized intermediate construction must have GPU implementations. CPU-generated image or grain data must not be relabeled as cached preparation to satisfy GPU coverage. A cache hit does not prove where its content was computed.

CPU and GPU preparation must derive the same calibrated model. A prepared inverse must keep the owning scanner's grid, triangulation, endpoint and conditioning rules. A coupled inverse must not become a per-pixel iterative solver. The convert-film solver must keep its bounded solve and stopping policy rather than substitute that inverse.

Cache identity must include resolved parameters, source data identities, coordinate geometry where relevant, precision representation, and device ownership. Backend selection must not mutate accepted controls. An image revision or device change must invalidate resources whose contents depend on it.

Native RAW unpacking must expose the sensor samples and metadata needed by the selected decoding algorithm. GPU numerical processing must retain that algorithm's supported sensor modes, demosaicing, corrections, orientation, white balance, and color assignment. Moving only the final ACES transform to GPU must not count as GPU RAW processing.

## Device residency

Device image ownership must represent dimensions, sample domain, color and transfer state, working pixel pitch, and lifetime. Stages must consume that representation without obtaining host pixels implicitly. CPU reference execution may use host images under the same domain contracts.

The operation must retain device ownership through input numerical preparation, film and scan work, output composition, and final saving or display conversion. Save must consume retained output in its actual storage domain without rerendering. A final host materialization for the native writer or a diagnostic output is an explicit handoff, not an internal stage boundary. The pipe adds one further explicit handoff class under [fence semantics](#fence-semantics): a fence around a `Cpu` capability node on the automatic route. Fences are assembled in advance, never appear on a full-GPU render, and are governed by the residency rule that a full-GPU render must not materialize intermediates on the host.

A viewer must consume resident image data when compute and presentation devices can share it. Separate devices may require a final presentation transfer. Presentation must not download an image for CPU resampling or ICC evaluation and upload that computed raster as if it were full-GPU viewing. Software WGPU must remain visibly identified.

Multiple submissions, ping-pong buffers, GPU reductions, and tiles are valid resident execution. Device memory residency must not be equated with one command buffer, one binding, or physical VRAM counters. Logical transfer accounting follows [telemetry metrics](../telemetry/metrics/spec.md#actual-gpu-work).

A full-source reduction must retain the specified source population before crop or resize. GPU reductions must preserve the reference measurement law and supply device-side scanner or exposure interpretation. Only bounded control results and validation flags may return to host orchestration.

## Pipe assembly

The stage sequence is carried by an internal pipe. Each numerical stage is a node: a `Gpu` node dispatches a registered shader over device buffers and never reads host pixels; a `Cpu` node wraps an existing CPU stage implementation unchanged. Node selection happens at assembly time from the resolved parameters; an inactive feature does not enter the pipe, create a pass-through node, or appear in decline accounting. Fixed stage ordering, image-domain transitions, and the runtime's ordering ownership are preserved exactly; only which nodes exist is dynamic. Compute pipelines are created lazily per shader identity and held for the process lifetime; device buffers are reused through the pipe's pools and a cross-frame cache, and buffer lifetime must respect command-completion rules.

## Fence semantics

A fence is an explicit, assembled-in-advance host materialization around a `Cpu` node: readback of the stage input, execution of the existing CPU stage, and re-upload of the stage output. A fence is a transfer optimization for the automatic-selection route, not a form of CPU fallback: the set of stages that execute on each processor is identical to the per-stage route, and the pipe only removes the redundant transfers around them, so the transfer count becomes `2 + 2 × fence_count` instead of `2 × stage_count`. An explicit GPU request must still satisfy the contract: a capability-missing `Cpu` node for any stage in the computation scope fails the operation before output publication. A fence must never appear silently on an explicit GPU route. A fence boundary introduces explicit conversion between device and CPU arithmetic whose error must fit the existing per-stage budgets; parity comparisons for fenced configurations must be re-frozen per stage because the rounding pattern changes even though no algorithm changes.

The permitted CPU fallback of small-sigma `gaussian_blur` remains an owned exception with its existing justification and telemetry. A reduction node, for example metering, may return only bounded scalar results to host orchestration; it is not an image fence.

Stages without a GPU implementation are capability declines: the automatic route assembles a `Cpu` node behind a fence, and an explicit GPU request fails until that stage gains a GPU node. A stage that consumes a CPU intermediate, such as the Dye grain field that needs developed-film density, is a semantic decline: the pipe materializes the intermediate once as an explicit tap fence and continues, with a GPU node as the owned end state. Route selection, such as Mallett execution parity, workflow route, or an unsupported output gamut, is not a decline: it remains an assembly-time route decision that selects a route without demoting one.

## Numerical alignment

Each stage must define its reference input and output domains and its permitted arithmetic differences. The CPU f64 model is the oracle; CPU native-f32 distinguishes host rounding from algorithm changes. A GPU representation must identify any binary16 composition, f32 arithmetic, integer arithmetic, or extended-precision computation that affects its result.

The executor must preserve per-channel coefficients, normalization, wavelength grids, kernel support, interpolation continuity, spectral missing-value policy, extrapolation, and unclipped headroom. An implementation may fuse linear operations only after stage and integrated evidence proves agreement under the frozen budget. A nonlinear operation must not move across a spatial filter or domain transform.

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

A tile boundary must not change random assignment, endpoint policy, source conditioning, spectral aperture readout, or optical order. Whole-frame and tiled outputs must pass identical budgets. Data-dependent iteration must preserve reference stopping and error policy and must terminate within the owning algorithm's bounds.

Cancellation must be checked before additional chunks and before publication. An in-flight GPU pass may finish. Results from cancelled, failed, or superseded operations must not enter retained output or destination files. Releasing an operation must not invalidate resources still referenced by submitted work.

## Verification ownership

Stage owners must supply domain and discrete-invariant comparisons. Runtime owners must prove integrated route and parameter composition. Frontends must prove real operations, saving attribution and stale-result behavior. The backend must prove actual device execution, resource bounds and transfer accounting. Telemetry must preserve its existing wire meanings and distinguish host orchestration from CPU image arithmetic.

Acceptance evidence must join these scopes through immutable input, parameter, data, executable and adapter identities. A legacy `gpu_resident` report covers its historical fused-film scope; it must not be reinterpreted as evidence for RAW, preparation, display or saving coverage. Missing or truncated observations must remain insufficient evidence. This design does not add a telemetry schema or rename existing execution enums.

Production cutover requires the [complete acceptance](spec.md#acceptance), not successful shader compilation or a new interface with unimplemented stages. Tests and retained evidence must cover plausible output and policy failures. Scaffolds, CPU pass-through implementations labeled GPU, and mixed-stage execution must not satisfy the full-GPU target.
