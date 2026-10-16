# Compute backend boundary design

## Ownership

The workspace must separate the compute backend abstraction from its device implementations. The abstraction owns the `ComputeBackend` trait, its default CPU implementations, and the shared observation vocabulary. A device implementation owns device resources, kernel dispatch, device error handling, and synchronization. This separation keeps the abstraction available to domain code without pulling a device toolkit into its dependency graph.

[GPU execution](../gpu-execution/design.md) already fixes this split at the module level: the CPU implementation owns reference evaluation, and the WGPU backend owns device resources, kernel dispatch, device error handling, and synchronization. This document carries that same split down to the crate boundary, so a consumer cannot reach device code through an import that only needed the reference implementation.

The abstraction must not depend on any device toolkit. A device implementation must depend on the abstraction, never the reverse. Domain code that consumes the abstraction must not acquire a transitive device dependency.

[GPU execution](../gpu-execution/design.md) owns which stages run on which processor. [Telemetry](../telemetry/design.md) owns observation semantics. This document owns only where each piece of code lives and what it may import.

## Crate boundary

The workspace must contain these crates for compute:

| Crate | Owns | Must not own |
|-------|------|--------------|
| `spektrafilm-math` | scalar and image primitives, color spaces | backend abstraction |
| `spektrafilm-backend` | `ComputeBackend` trait, default CPU implementations, telemetry vocabulary, timing, observation binding, device-independent parameter records | device resources, shader code, device error handling |
| `spektrafilm-gpu` | the WGPU backend, device buffers, kernels, device error handling, the fused film chain pipe | the trait definition, CPU default implementations, telemetry vocabulary, device-independent parameter records |
| `spektrafilm-core` | runtime, stage ordering, operation validation, report assembly | device resources |
| `spektrafilm-model` | film and scanner domain models | device resources, direct adapter selection |

`spektrafilm-backend` must be the only owner of the `ComputeBackend` trait. `spektrafilm-gpu` must implement that trait for its device backend and must not redefine it.

`spektrafilm-backend` must not enable a device feature by default. `spektrafilm-gpu` must remain the crate that enables the device toolkit. A build that selects only `spektrafilm-backend` must not compile any device code.

## Dependency direction

Dependencies must point in one direction only:

```mermaid
graph TD
    math[spektrafilm-math]
    backend[spektrafilm-backend]
    gpu[spektrafilm-gpu]
    core[spektrafilm-core]
    model[spektrafilm-model]
    raw[spektrafilm-raw]
    cli[spektrafilm-cli]
    gui[spektrafilm-gui]
    bench[spektrafilm-benchmark]

    backend --> math
    gpu --> backend
    gpu --> math
    model --> backend
    model --> math
    core --> backend
    core --> gpu
    core --> math
    core --> model
    raw --> math
    cli --> core
    cli --> gpu
    cli --> math
    cli --> raw
    gui --> core
    gui --> gpu
    gui --> math
    gui --> model
    gui --> raw
    bench --> math
```

`spektrafilm-benchmark` depends on `spektrafilm-math` only, so it is unaffected by the boundary.

`spektrafilm-cli` and `spektrafilm-gui` keep their direct `spektrafilm-gpu` dependency, because both select a device backend through `select_backend`.

`spektrafilm-model` must depend on `spektrafilm-backend` and must not depend on `spektrafilm-gpu`. This removes the inversion where domain code required a device crate only to reach the CPU reference implementation.

`spektrafilm-core` may depend on both `spektrafilm-backend` and `spektrafilm-gpu`, because it owns route selection across CPU and GPU execution. A frontend that never selects a device must not require `spektrafilm-gpu`.

## What moves

These items move from `spektrafilm-gpu` to `spektrafilm-backend` without behavior change:

- the `ComputeBackend` trait and its default implementations
- `CpuBackend` and its CPU spectral implementations
- the `telemetry` module and its observation vocabulary
- the `timing` module
- `observed_backend`, `bind_backend`, and `bind_cpu_fallback`
- `FilmChainParams`, `FrontPass`, and the `*GpuParams` records
- `GrainV2GpuParams`, `CpuStageFn`, and `Lut3D`
- `DyeGpuParams` and its type chain: `DyeReadoutGrid`, `DyeChannelTablesGpu`, `DyeChannelTableGpu`, and `DyeChannelParts`

`DyeGpuParams` currently lives under `wgpu_backend` but holds only plain data: channel tables, a cell size, taps, a readout grid, a seed, a correlation, and an expectation flag. No field is a device type. `spektrafilm-core` already constructs it directly. It is a misplaced device-independent parameter record, and the trait cannot move until it does.

Its type chain is also device-free, but those types share a file with `LayerPass`, which encodes device passes. Moving them requires splitting `dye_layers.rs` so the plain records leave and the device passes stay. This split is the first concrete step of the migration, not a later cleanup.

`DyeChannelParts` and `DyeChannelTablesGpu::pack` must move with them. `spektrafilm-core` constructs `DyeChannelParts` in production code at `stages/grain_v3.rs` and passes it to `pack`, which converts f64 model tables to f32 device records without touching a device. Leaving either behind would keep a device import in the domain code that builds them.

Every moving item must be free of device references before it moves. `CpuBackend` imports only `rayon`, `spektrafilm-math`, and `blas_src`. The telemetry vocabulary imports only `serde`, `tracing`, and `parking_lot`. `timing` imports no device crate. `observed_backend` names `DyeGpuParams` in its `dye_field` forwarder, and that reference disappears once the record moves. An item that cannot be made device-free must stay with the device crate and the boundary must be re-evaluated rather than force-moved.

These items stay in `spektrafilm-gpu`:

- `wgpu_backend` and every type under it, except the device-independent parameter records named above
- `WgpuBackend` and its `ComputeBackend` implementation
- the film chain pipe, its buffer pools, its pass assembly, and its shader code

`FilmChainParams` moves with the trait, because the trait's `try_run_film_chain` signature references it. Its field types — `FrontPass` and the `*GpuParams` records — are plain data: scalars, arrays, and borrowed slices. None is a device type. The `Gpu` in their names describes which pass consumes them, not what they contain. Leaving them behind would force the trait to keep a device dependency for a struct of numbers.

`test_support` must split by device: adapter acquisition for a device backend stays with that backend, and any device-independent test helper moves to `spektrafilm-backend`.

`select_backend` must stay in `spektrafilm-gpu`, because it constructs a device backend when an adapter is available. Its five call sites — four in the CLI, one in the GUI — keep their behavior, but their import path changes in step 4.

## Feature propagation

`spektrafilm-backend` must expose no device feature. `spektrafilm-gpu` must own the feature that enables the device toolkit and must keep it non-default when a consumer needs a device-free build.

A workspace member that depends on `spektrafilm-gpu` must state that dependency directly. Feature unification must not silently enable a device toolkit for a consumer that only requires `spektrafilm-backend`.

## Numerical and behavioral invariance

The move must not change any of the following:

- output pixels for any input
- the CPU reference implementation's arithmetic, iteration order, or precision
- telemetry observation names, fields, units, or counter definitions
- error message strings
- public function signatures beyond their module path

Every moved item keeps its name. Only its import path changes.

## Verification ownership

A build with only `spektrafilm-backend` in the dependency graph must compile and must not link a device toolkit.

The workspace test suite must pass unchanged after the move. Parity tests must produce identical results. Telemetry reports must be byte-identical for identical operations.

The dependency graph must be checked mechanically: no rule in this document may rely on review discipline alone.

## Risks

The `ComputeBackend` trait is referenced at about 93 call sites across four crates. Each site changes only its import path. A mechanical rename risk is a missed site, which the compiler catches.

`spektrafilm-core` keeps its `spektrafilm-gpu` dependency for two reasons: it constructs `FilmChainParams` for the device route, and it names `WgpuBackend` directly in `stages/grain_v3.rs`. This is correct: core owns route selection, and route selection requires knowledge of what the device can do.

The telemetry vocabulary moves, but its ownership rules do not. [Telemetry](../telemetry/design.md) keeps authority over observation semantics; this document only relocates the code.

## Migration order

The move must proceed in this order, because each step must compile before the next begins:

1. Create `spektrafilm-backend` with no device feature. Move the device-independent parameter records first — `DyeGpuParams` and its type chain, `GrainV2GpuParams`, `CpuStageFn`, `Lut3D`, `FilmChainParams`, `FrontPass`, and the `*GpuParams` records — because the trait signature references them. Splitting `dye_layers.rs` is part of this step, since those records share a file with device pass code. `spektrafilm-gpu` re-exports them so the tree still compiles.
2. Move the `ComputeBackend` trait and its default implementations, `CpuBackend`, telemetry, timing, and observation binding into `spektrafilm-backend`. Keep the re-exports.
3. Move `spektrafilm-model` to depend on `spektrafilm-backend` only.
4. Update `spektrafilm-core`, `spektrafilm-cli`, and `spektrafilm-gui` import paths.
5. Remove the `spektrafilm-gpu` re-exports and confirm no consumer reaches a moved item through the device crate.
6. Split `test_support` and confirm a device-free build.

A dry run of steps 1 and 2 produced a compiling `spektrafilm-backend` whose dependency tree contains no device crate. That result is the evidence this order is viable; it is not a claim that later steps are equally cheap.

Re-exports exist only to keep intermediate states compiling. Step 5 must delete them, so the final tree has exactly one path to each moved item.
