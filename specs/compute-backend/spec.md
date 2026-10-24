# Compute backend

## Contract

The compute backend abstraction MUST be a workspace crate of its own. Domain code that evaluates film and scanner models on the CPU MUST NOT require a device toolkit to compile.

A consumer of the backend abstraction MUST be able to name the trait, its default CPU implementation, and the shared observation vocabulary without naming any device crate. A consumer that requires device execution MUST name the device crate directly.

## Crate ownership

The workspace MUST own exactly one definition of the `ComputeBackend` trait. That definition MUST live in `spektrafilm-backend`. The WGPU crate MUST implement the trait and MUST NOT declare a second one.

`spektrafilm-backend` MUST own the default CPU implementations of every trait method. The WGPU crate MUST override the methods it executes on the device. A selected GPU MUST report an unavailable image operation rather than inherit CPU image execution.

A parameter record that a trait signature references MUST live with the trait, not with a device implementation. `DyeGpuParams`, `FilmChainParams`, `FrontPass`, the `*GpuParams` records, and the Dye table records hold only plain data — scalars, arrays, and borrowed slices — so they MUST live in `spektrafilm-backend`. The `Gpu` in a name states which pass consumes the record, not what it contains.

A device uniform layout, a bind group, and a pipeline stay with the device crate.

A domain crate MUST be able to build a device parameter record without naming the device crate. `spektrafilm-core` constructs `DyeGpuParams`, `DyeReadoutGrid`, and `DyeChannelParts` in production code, so those records MUST be reachable without a device import.

`spektrafilm-backend` MUST own the shared observation vocabulary: observation context, observation kinds, CPU reasons, and the scope binding helpers. This vocabulary MUST remain available below `spektrafilm-core`, so model and backend code MUST NOT acquire a dependency on core to observe their work. This preserves the existing [telemetry boundary](../telemetry/design.md).

`spektrafilm-gpu` MUST own device resources, kernel dispatch, device error handling, synchronization, and the fused film chain pipe. It MUST NOT own the trait, the CPU defaults, or the observation vocabulary.

## Image-stage failure propagation

Image-stage backend operations expose dispatch and resource failures through `Result`, including color conversion, transfer functions, Gaussian filtering, table and LUT interpolation, spectral integration, and density interpolation. Core stages and model wrappers propagate those errors to the render boundary without publishing an image.

An optional device hook returning `None` permits the CPU reference only when CPU was selected initially. After WGPU selection, either explicit or automatic, an unavailable operation or resource decline is a named operation error; it MUST NOT invoke the CPU reference, panic, or defer the failure through shared state.

Initial automatic selection MAY still choose CPU when no usable adapter exists.

Observation collection records execution; it does not select execution. Disabling telemetry MUST NOT bypass Gaussian, diffusion, or LUT device work. Injected per-device failures cover telemetry-off and observed requests and require an error with no CPU executor or output readback.


## Dependency direction

`spektrafilm-model` MUST depend on `spektrafilm-backend`. It MUST NOT depend on `spektrafilm-gpu` for production code. A test that requires a device adapter MAY keep `spektrafilm-gpu` as a dev-dependency, because a parity test MUST compare the CPU reference against the device implementation.

`spektrafilm-backend` MUST NOT depend on `spektrafilm-gpu`, on `spektrafilm-model`, or on `spektrafilm-core`.

`spektrafilm-core` MAY depend on `spektrafilm-gpu`, because it owns route selection across CPU and GPU execution and MUST know what the device can do.

A device-free build MUST be possible: a consumer that requires only the CPU reference implementation and the abstraction MUST compile and link without a device toolkit.

## Behavior preservation

The crate reorganization preserves numerical behavior. The image-stage failure contract above additionally requires fallible return types and named errors where an infallible API previously hid device failure.

Output pixels MUST be identical for identical inputs and parameters on both the CPU and GPU routes. The CPU reference implementation MUST keep its arithmetic, iteration order, and precision. Telemetry observation names, fields, units, and counter definitions MUST NOT change.

Public function names and parameters remain stable; image-stage return values carry operation errors.

A test that exercises the CPU route MUST NOT require a device adapter. A test that exercises the GPU route MUST still require one, and a missing adapter MUST remain a hard error rather than a silent skip.

## Acceptance

A dependency audit MUST confirm the boundary. `cargo tree -p spektrafilm-model` MUST NOT contain `wgpu` or any of its transitive device crates. `cargo tree -p spektrafilm-backend` MUST NOT contain a device toolkit.

No crate outside `spektrafilm-gpu` MAY reach a device toolkit except through a direct `spektrafilm-gpu` dependency.

The defect this removes: a domain model crate MUST NOT pull a device toolkit only to reach the CPU reference implementation.

The full workspace test suite MUST pass. CPU and GPU parity tests preserve the same numerical reference. Telemetry collection MUST leave output pixels unchanged.

Failure regressions MUST distinguish initial CPU selection from selected-WGPU failure, and failed device requests MUST neither create output nor replace an existing published image.

A build that excludes the device crate from the dependency graph MUST compile, MUST link, and MUST pass its tests. This build MUST NOT require an adapter.

Initial route selection remains unchanged. An automatic request selects the device backend when an adapter is available and CPU otherwise. An explicit device request fails when no adapter is available.

After either kind of WGPU selection, a missing device operation or resource failure is an error, never permission to run CPU image stages.
