# Rust benchmark delivery

## Scope and authority

This document defines the Rust migration and its acceptance criteria. The [product specification](../spec.md) owns evaluation meaning and human review. The [machine interface](../interface.md) owns commands and documents. The [measurement protocol](../metrics.md) owns measurement semantics. The [system design](../design.md) owns runtime boundaries.

The delivered implementation must provide the complete benchmark as a Rust CLI. A Rust launcher that requires the Python benchmark does not satisfy this contract. All existing command families, reference scopes, measurement families, failure states, and review capabilities must remain supported.

The migration must not change film simulation algorithms, add measurement families, change qualification budgets, or expand the reference corpus. Runtime migration must not be presented as improved film fidelity.

## Installation and distribution

<<<<<<<< HEAD:specs/benchmark/rust-delivery.md
The release must include `spektrafilm-bench` and the existing `spektrafilm-bench-fixtures` entry point. The fixture entry point must retain its current options and source attribution behavior. Both entry points must run without Python, uv, pip, or Python packages.
========
The release must include `spektrafilm-bench` and the existing `spektrafilm-bench-fixtures` entry point. The fixture entry point must retain its current options and source attribution behavior, and it must answer the [standard configuration generation](../interface.md#standard-configuration-generation) contract by invoking the selected renderer's preset export command. Both entry points must run without Python, uv, pip, or Python packages.
>>>>>>>> 0e4fd168 (docs(specs): unify capability layout and contract filenames (#139)):specs/benchmark/rust-delivery/spec.md

Release archives must provide binaries for the platforms supported by the application's packaging workflows. Every non-system runtime dependency must be bundled or linked into the delivered executable. The release must identify required system libraries and third-party licenses. A Rust executable with C or C++ dependencies satisfies this requirement; a pure-Rust dependency graph is not required.

Help, version, and schema discovery must work before native metric libraries or renderer dependencies are loaded. The installed commands must work from a directory outside the source checkout. Optional natural-image acquisition must remain explicit. Tests must not require network acquisition of images.

## Workspace and execution boundary

The Rust implementation must use workspace boundaries selected from actual dependencies. This specification does not prescribe a new crate or module hierarchy. Its executable must continue to invoke the renderer path declared by the target document. It must not substitute a linked runtime for that renderer.

The benchmark must support historical renderer executables and retained results independently of the current application's build. Compiling the benchmark must not require a GPU runtime or GUI toolkit solely to obtain color math, image decoding, or document helpers. Shared mathematical code may be reused through an existing dependency boundary when that boundary meets this constraint.

The implementation must use typed internal records for documents, measurements, artifact roles, availability, and terminal states. JSON remains the public carrier. Typed records must preserve the interface's different rules for authored and generated documents. Runtime validation remains responsible for hashes, paths, geometry, populations, and cross-document identities.

Metric adapters must return structured evidence. A report must consume that evidence without recomputing measurements. Browser JavaScript may remain embedded in the generated offline report. Browser JavaScript is not a benchmark runtime dependency.

## Numerical implementation

The implementation must use f64 for the standard protocol's native diagnostics, color transforms, Lab calculations, statistics, and spectrum normalization unless the protocol explicitly identifies another precision. Artifact storage must not imply a higher computation precision.

LDR-FLIP must use the pinned [official NVlabs implementation](https://github.com/NVlabs/flip). A narrow native bridge to the official C++ implementation is the selected integration boundary. The bridge must identify its source revision and build settings. It must use the declared input domain and pixels-per-degree settings. It must return the official error map without a palette transform. The migration must not replace official FLIP with an unverified Rust reimplementation.

ICC conversion must continue to use an identified LittleCMS implementation with the existing transform settings. The release must not fall back to an approximate RGB transform when ICC conversion is unavailable.

Reused Rust color functions must be verified against the standard protocol before adoption. The same function name or color-space label does not establish equivalence. Verification must cover white points, chromatic adaptation, transfer functions, negative values, super-white values, and clipping boundaries.

Gaussian filtering must preserve kernel support and boundary extension. FFT measurements must preserve frequency coordinates, window definitions, masks, and normalization. Percentiles must preserve the declared interpolation method and population. Covariance and multi-seed statistics must preserve degrees of freedom, eligibility, and failure precedence.

Exact integer decoding must preserve RGB16 PNG channel codes, including filtered and interlaced inputs. PNG detail crops must preserve the generated full-image channel codes. Display conversion and resampling remain subject to the product's separate display-copy contract.

## Existing evidence and protocol identity

The Rust readers must accept supported schema-version-1 authored documents and completed Python-produced run and comparison records. They must verify the recorded identity using that record's original rules. Reading an old record must not reinterpret its implementation identity as the Rust implementation.

A report generated from an existing comparison must retain its measurement values, availability, findings, and evaluation result. Report markup and PNG compressed bytes may differ. Decoded full-image and detail channel codes must remain exact when the report copies or crops existing display artifacts.

A new comparison computed by Rust must have a new resolved measurement identity. That identity must include the benchmark implementation revision and relevant numerical-library versions and settings. A preserved schema version does not imply a preserved measurement identity.

A policy bound to a Python measurement identity must not qualify a new Rust comparison. The policy must be rebound to the Rust identity with reviewed calibration evidence. The benchmark must not rewrite or automatically approve an old policy. An existing comparison's recorded policy result remains historical evidence when that comparison is reported.

Run comparison must remain possible after original source files and the renderer have become unavailable, using the retained dependencies required by the interface. Migration must not reintroduce reads from authored paths during comparison or report generation.

## Numerical acceptance

Acceptance must use a pinned Python implementation and its locked dependencies as the migration reference. The reference revision must include the native PNG decoding and evidence-boundary fixes. Its source revision, dependency versions, inputs, and invocation conditions must be retained outside the repository with the acceptance evidence. Python may be used for this migration verification; it must not be required by the delivered runtime or its ordinary post-cutover checks.

Discrete results must match exactly: dimensions, coordinates, masks, sample counts, selected seeds, availability, reason codes, rule outcomes, and deterministic hotspot ordering. Tests with stable hotspot ranking must assert the selected rectangles. Tests with ties must assert the documented tie rule.

A migration acceptance budget must be frozen before evaluating the Rust candidate. Each budget must identify a metric or artifact family, its domain, units, eligible population, absolute tolerance, relative tolerance when used, and source of the bound. The bounds must derive from reference data, repeated reference computations, or the numerical sensitivity of the declared algorithm. A blanket tolerance across all metrics is not acceptable. Increasing a budget after observing a failure requires an explicit reviewed change with retained failure evidence.

Acceptance must compare full maps and curve samples as well as scalar summaries. Matching a mean or maximum alone is insufficient. Numerical agreement must not hide a changed mask, count, reference scope, clipping behavior, or unavailable measurement.

CIEDE2000 must pass every Sharma supplementary pair within the precision of the published four-decimal values. Additional tests must cover neutral inputs, zero chroma, hue wrapping, and symmetry. Official FLIP verification must compare the same pinned upstream implementation with identical inputs and settings.

Policy tests must include values at inclusive limits, immediately below limits, and immediately above limits. Exact zero differences must remain zero where the declared deterministic computation permits it. A numerical tolerance used for migration comparison must not be added to production policy evaluation.

The acceptance corpus must exercise every shipped measurement family and all four reference scopes. It must include ordinary RGB, negative and super-white float samples, RGB16 PNG, ICC references, file-backed masks, failed seeds, and failed render cases. Real CLI scenarios must verify file-carrier rejection, selected-profile confinement, protocol-level mask retention, target preview-geometry ownership, and failure reporting.

## Resource acceptance

Resource evaluation must measure the current Python implementation and the Rust candidate on the same machine, filesystem, inputs, protocol, and output requirements. It must record thread settings, warm or cold conditions, repetitions, elapsed time, peak resident memory, artifact bytes, and installed runtime size. Profiling overhead must be identified.

Evaluation must separate renderer time, comparison time, and report time. A faster renderer must not be reported as a benchmark implementation speedup. A run that omits maps, reference objectives, seeds, or report details is not comparable.

The required workloads are a small automated suite, one 6720-by-4480 color comparison with numeric/color/tone/FLIP measurements, and a complete suite with multi-seed grain, halation, edge, and reference cases. The 30 MP report must exercise fit images, full-resolution display copies, and multiple native detail crops.

The comparison engine must avoid allocating full-image representations for unselected measurement families. It must bound the lifetime of temporary buffers and reuse buffers where numerical semantics allow. Local kernels may use tiles with sufficient halo. Global populations, exact quantiles, FFTs, and official FLIP must retain their declared semantics rather than substitute tile-local approximations.

Rust comparison peak memory for the 30 MP workload must be lower than the reference implementation's peak memory. Runtime must have no unexplained regression beyond the measured variation. Release evidence must state the observed changes without promising an unmeasured speedup factor. A resource regression requires an explicit reviewed tradeoff before cutover.

## Verification and cutover

The following stages define dependencies, not reduced deliverables:

1. Freeze the Python reference, acceptance corpus, and numerical budgets. Verify the reference covers all required behaviors before using it to judge Rust.
2. Implement Rust comparison against retained runs. Verify every measurement family, reference scope, discrete result, and artifact with the frozen corpus. Measure comparison resources.
3. Complete schema discovery, validation, run orchestration, fixture generation, policy evaluation, and offline report generation. Exercise the real renderer and installed executables. Verify the product acceptance scenarios and browser interactions.
4. Publish supported-platform packages. Exercise the installed binaries without a Python environment. Verify hashes, licenses, runtime dependencies, and examples from outside the checkout.
5. Switch the documented entry points and CI to Rust. Remove the obsolete Python production entry points, package, lockfile, and CI installation. Retain the pinned migration reference in external evidence or repository history. Preserve useful research tooling only when it has an identified independent consumer.

Cutover requires the complete CLI and fixture surface, numerical and resource acceptance, supported-platform package smoke evidence, and actual offline browser verification. A partially migrated launcher or reduced metric set must not become the default.

Permanent deterministic behavioral regressions must run through the repository's existing Justfile and required CI gate. Real-renderer contract checks must use the public renderer entry point. Generated evidence, reference environments, performance probes, and migration logs must remain outside the repository. Build, usage, and package instructions must be updated together with the final cutover.

## Delivered implementation

The delivered benchmark is the Rust workspace implementation. The frozen migration budgets, cross-implementation acceptance corpus, resource evidence, and pinned Python reference remain outside the repository with the acceptance evidence. The measured migration comparison establishes observed resource behavior for that evidence run; it is not a release performance promise.
