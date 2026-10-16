# Compute backend

## Contract

The compute backend abstraction must be a workspace crate of its own. Domain code that evaluates film and scanner models on the CPU must not require a device toolkit to compile.

A consumer of the backend abstraction must be able to name the trait, its default CPU implementation, and the shared observation vocabulary without naming any device crate. A consumer that requires device execution must name the device crate directly.

## Crate ownership

The workspace must own exactly one definition of the `ComputeBackend` trait. That definition must live in `spektrafilm-backend`. The WGPU crate must implement the trait and must not declare a second one.

`spektrafilm-backend` must own the default CPU implementations of every trait method. The WGPU crate must override the methods it accelerates and must inherit the rest.

A parameter record that a trait signature references must live with the trait, not with a device implementation. `DyeGpuParams`, `FilmChainParams`, `FrontPass`, the `*GpuParams` records, and the Dye table records hold only plain data — scalars, arrays, and borrowed slices — so they must live in `spektrafilm-backend`. The `Gpu` in a name states which pass consumes the record, not what it contains. A device uniform layout, a bind group, and a pipeline stay with the device crate.

A domain crate must be able to build a device parameter record without naming the device crate. `spektrafilm-core` constructs `DyeGpuParams`, `DyeReadoutGrid`, and `DyeChannelParts` in production code, so those records must be reachable without a device import.

`spektrafilm-backend` must own the shared observation vocabulary: observation context, observation kinds, CPU reasons, and the scope binding helpers. This vocabulary must remain available below `spektrafilm-core`, so model and backend code must not acquire a dependency on core to observe their work. This preserves the existing [telemetry boundary](../telemetry/design.md).

`spektrafilm-gpu` must own device resources, kernel dispatch, device error handling, synchronization, and the fused film chain pipe. It must not own the trait, the CPU defaults, or the observation vocabulary.

## Dependency direction

`spektrafilm-model` must depend on `spektrafilm-backend`. It must not depend on `spektrafilm-gpu`.

`spektrafilm-backend` must not depend on `spektrafilm-gpu`, on `spektrafilm-model`, or on `spektrafilm-core`.

`spektrafilm-core` may depend on `spektrafilm-gpu`, because it owns route selection across CPU and GPU execution and must know what the device can do.

A device-free build must be possible: a consumer that requires only the CPU reference implementation and the abstraction must compile and link without a device toolkit.

## Behavior preservation

The reorganization must not change any observable behavior.

Output pixels must be identical for identical inputs and parameters on both the CPU and GPU routes. The CPU reference implementation must keep its arithmetic, iteration order, and precision. Telemetry observation names, fields, units, and counter definitions must not change. Error message strings must not change. Public function signatures must keep their names and parameters; only their module path may change.

A test that exercises the CPU route must not require a device adapter. A test that exercises the GPU route must still require one, and a missing adapter must remain a hard error rather than a silent skip.

## Acceptance

A dependency audit must confirm the boundary. `cargo tree -p spektrafilm-model` must not contain `wgpu` or any of its transitive device crates. `cargo tree -p spektrafilm-backend` must not contain a device toolkit. No crate outside `spektrafilm-gpu` may reach a device toolkit except through a direct `spektrafilm-gpu` dependency.

The defect this removes: a domain model crate must not pull a device toolkit only to reach the CPU reference implementation.

The full workspace test suite must pass with unchanged assertions. CPU and GPU parity tests must produce identical results before and after the move. Telemetry reports for identical operations must be identical.

A build that excludes the device crate from the dependency graph must compile, must link, and must pass its tests. This build must not require an adapter.

Route selection must remain unchanged. An automatic request must still select the device backend when an adapter is available and fall back to the CPU implementation otherwise. An explicit device request must still fail when no adapter is available, and must still fail when a required stage has no device implementation.
