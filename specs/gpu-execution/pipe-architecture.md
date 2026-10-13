# GPU pipe architecture

This document extends the [design](design.md) with the execution architecture that
carries the [contract](spec.md) over time. It defines the pipe object model, the
stage capability and fence semantics, and the migration slices from the current
fixed resident chain plus per-stage fallback. It does not change the contract,
backend selection policy, or acceptance rules.

## Motivation

Two structural costs exist today.

Transfer loss on the per-stage route. Every per-stage trait call performs one
host upload and one host readback even when both producer and consumer run on
the device. Measured on the llvmpipe reference configuration, the default
per-stage film route performs 8 uploads and 8 readbacks where the fused
resident chain performs one of each. On unified-memory devices the dominant
per-frame cost is the staged upload blit, not device computation.

Combinatorial decline list. The resident decision in
`spektrafilm-core/src/pipeline/resident.rs` (`resident_decision`) enumerates
fourteen decline reasons. Each new feature either extends the fixed fused
chain in `spektrafilm-gpu/src/wgpu_backend/film_chain.rs`, a single handwritten
14-stage sequence, or adds another decline reason that demotes the whole route
to per-stage. The cost grows as a list, not as an architecture.

## External precedent

The pipe architecture follows the model proven by the darktable OpenCL
pixelpipe. The mappings are:

- The pixelpipe owns nodes, buffers, and all host/device transfers; modules
  never manage transfers. This design gives `GpuPipe` the same ownership:
  stages receive and produce device buffers.
- Each darktable module has a GPU path (`process_cl`) and a CPU path
  (`process`) with per-module capability flags resolved before processing.
  This design uses `PipeStage::Gpu` and `PipeStage::Cpu` nodes with capability
  resolved at pipe assembly.
- A darktable module without a GPU path runs on CPU without dissolving the
  pipe. This design gives that behavior to fences: an unported stage
  materializes once, runs the existing CPU stage, and re-uploads.
- darktable compiles kernels once per module type in `init_global`. This
  design holds one `wgpu::ComputePipeline` per shader identity for the process
  lifetime.
- darktable reuses pixelpipe buffers across runs through its state-hashed
  cache. This design reuses a bounded buffer pool across frames. Hash-keyed
  stage caching is a separate later revision, not part of this design.

Two darktable mechanisms are not adopted.

ROI back-propagation and tiling are not adopted. darktable serves an
interactive darkroom viewport with regional recomputation. This product
renders fixed full-grid image chains. Tiling matters only under device memory
pressure, which remains governed by the existing bounded-execution rules.

History and per-pipe parameter commits are not adopted. darktable runs
concurrent pipes against shared module state. This product resolves one
parameter snapshot per operation, so commit-style layering adds no safety.

## Architecture

### Pipe object

`GpuPipe` lives in `spektrafilm-gpu` and is the internal execution carrier. It
owns a bounded buffer pool of ping-pong image buffers plus staged lookup and
uniform buffers, reused across frames and invalidated on image revision or
device change per the existing cache-identity rules. It owns the ordered stage
list for one operation, assembled per run from the resolved parameters. It owns
the only host transfer points: input upload, fences, and the final readback.

The pipe is internal to the WGPU backend and runtime. It is not a public
effect graph, adds no public API surface, and introduces no second parameter
schema or serialized model.

### Stage nodes

Each numerical stage is a node in the pipe. A `Gpu` node dispatches a
registered shader over device buffers and never reads host pixels. A `Cpu`
node wraps an existing CPU stage implementation unchanged.

Node selection happens at assembly time from the resolved parameters. An
inactive feature does not enter the pipe at all. It does not create a
pass-through node and does not appear in decline accounting. Fixed stage
ordering, image-domain transitions, and the runtime's ordering ownership are
preserved exactly. Only which nodes exist is dynamic.

### Fence semantics

A fence is an explicit, assembled-in-advance host materialization around a
`Cpu` node: readback of the stage input, execution of the existing CPU stage,
and re-upload of the stage output.

A fence is a transfer optimization for the automatic-selection route, not a
new form of CPU fallback. Automatic selection may assemble `Cpu` nodes for
stages without a GPU implementation. The set of stages that execute on each
processor is identical to today's per-stage route. The pipe only removes the
redundant transfers around them, so the transfer count becomes
`2 + 2 × fence_count` instead of `2 × stage_count`.

An explicit GPU request must still satisfy the contract. If assembly finds a
capability-missing `Cpu` node for a stage in the computation scope, the
operation fails before output publication, exactly as it must today. A fence
never appears silently on an explicit GPU route.

The permitted CPU fallback of small-sigma `gaussian_blur` remains an owned
exception with its existing justification and telemetry.

A reduction node, for example metering, may return only bounded scalar results
to host orchestration. It is not an image fence.

### Shader and buffer registries

One `wgpu::ComputePipeline` per shader identity is created lazily and held for
the process lifetime. Per-call pipeline construction is eliminated. Device
buffers are pooled and reused. Buffer lifetime must continue to respect
command-completion rules. Intermediate storage is reused only after its
commands complete.

## Decline reclassification

The fourteen decline reasons are reclassified into three classes. This list is
authoritative for migration. `resident_decision` shrinks to the frontend
class.

Capability class, missing GPU node: Spektra grain distribution; input transfer
decoding; optical diffusion; requested spectral LUT; negative-to-positive
output; scanner film points; Langmuir chemistry; scanner receiver. Each becomes
a `Cpu` node plus fence on automatic selection. Each gains a GPU node through
its own slice. An explicit GPU request fails until that slice lands.

Semantic class, CPU intermediate dependency: the Dye grain v3 field, which
needs the developed-film density. It becomes an explicit tap fence: the pipe
materializes the density once for the CPU field solver, then continues. A GPU
field node remains the owned end state.

Frontend class, path selection rather than fallback: Mallett execution parity;
workflow route; missing resident front pass; unsupported output gamut. These
remain assembly-time route decisions. They select a route. They do not demote
one.

## Implementation slices

Slices are ordered by dependency. Each is independently mergeable and must not
regress the parity and telemetry acceptance of the merged state.

S1, pipe core: `GpuPipe` with buffer pool, stage list, fence and readback
primitives. The per-stage route is re-expressed on the pipe with unchanged
stage placement. Acceptance: the parity suite is unchanged and passing, and
transfer accounting does not increase.

S2, chain migration: the fused film chain becomes a sequence of `Gpu` nodes on
the pipe, and the fixed sequence in `film_chain.rs` is retired. Acceptance:
`mallett_gpu_resident_matches_cpu` passes and resident telemetry meanings are
unchanged.

S3, capability fences: capability-class declines become `Cpu` nodes with
fences, in priority order: Spektra grain, the default configuration, first;
input transfer decoding second; the remainder after. Acceptance: for each
unported feature the automatic route transfers at most `2 + 2 × fence_count`;
the explicit GPU route fails at assembly; decline-reason telemetry reflects
the frontend class only.

S4, pipe tail: post-scan finishing, meaning output CCTF encode, output mix,
and magazine, joins the pipe tail. Acceptance: post-readback host stage
timings are zero on the automatic GPU route.

S5, registries: shader registry and cross-frame buffer pooling. Acceptance:
second-run latency improves and transfer counts are unchanged.

S6, optional stage caching: hash-keyed stage caching for GUI preview. This
requires its own design revision under GUI ownership and is not part of this
document.

## Non-goals

No public effect-graph API, no second parameter schema, and no serialized pipe
model. No tiling or ROI propagation beyond the existing bounded-execution
rules. No change to backend selection policy, telemetry wire meanings, or the
`spektrafilm-f64` reference executable. No relaxation of algorithm alignment:
CPU nodes run the existing reference implementations, and fence boundaries
introduce f64-to-f32 conversion whose error must fit the existing per-stage
budgets.

## Acceptance

Every slice remains under the [contract acceptance](spec.md#acceptance).
Slice-specific evidence:

- Parity. The existing CPU/GPU parity suite and `mallett_gpu_resident_matches_cpu`
  must pass at each slice boundary. S2 additionally requires per-stage golden
  buffers compared through debug taps before cutover.
- Transfers. Telemetry transfer counts are the measured quantity. Claimed
  improvements must be demonstrated through the real entry point, not inferred
  from code structure.
- Policy. An explicit GPU request with any capability-missing stage must fail
  before publication. This must be demonstrated by test for each S3 migration.
- Fence-error mode. Parity comparisons for fenced configurations must be
  re-frozen per stage, because fence boundaries change the rounding pattern
  even though no algorithm changes.
