# Rust benchmark delivery

## Scope and authority

This document defines the delivered implementation's scope, distribution, workspace and execution boundary, numerical implementation, and protocol identity. The [product specification](../spec.md) owns evaluation meaning and human review. The [machine interface](../../instrument/spec.md) owns commands and documents. The [measurement protocol](../metrics/spec.md) owns measurement semantics. The [system design](../../instrument/design.md) owns runtime boundaries.

The delivered implementation must provide the complete instrument as a Rust CLI. All existing command families, reference scopes, measurement families, failure states, and review capabilities must remain supported.

The benchmark must not change film simulation algorithms, add measurement families, change qualification budgets, or expand the reference corpus.

## Installation and distribution

The release must include `spektrafilm-lab` and its `fixtures` command. The `fixtures` command must retain its current options and source attribution behavior, and it must answer the [standard configuration generation](../../instrument/spec.md#standard-configuration-generation) contract by invoking the selected renderer's preset export command. Both entry points must run without Python, uv, pip, or Python packages.

Release archives must provide binaries for the platforms supported by the application's packaging workflows. Every non-system runtime dependency must be bundled or linked into the delivered executable. The release must identify required system libraries and third-party licenses. A Rust executable with C or C++ dependencies satisfies this requirement; a pure-Rust dependency graph is not required.

Help, version, and schema discovery must work before native metric libraries or renderer dependencies are loaded. The installed commands must work from a directory outside the source checkout. Optional natural-image acquisition must remain explicit. Tests must not require network acquisition of images.

## Workspace and execution boundary

The Rust implementation must use workspace boundaries selected from actual dependencies. This specification does not prescribe a new crate or module hierarchy. Its executable must continue to invoke the renderer path declared by the target document. It must not substitute a linked runtime for that renderer.

The instrument must support historical renderer executables and retained results independently of the current application's build. Compiling the benchmark must not require a GPU runtime or GUI toolkit solely to obtain color math, image decoding, or document helpers. Shared mathematical code may be reused through an existing dependency boundary when that boundary meets this constraint.

The implementation must use typed internal records for documents, measurements, artifact roles, availability, and terminal states. JSON remains the public carrier. Typed records must preserve the interface's different rules for authored and generated documents. Runtime validation remains responsible for hashes, paths, geometry, populations, and cross-document identities.

Metric adapters must return structured evidence. A report must consume that evidence without recomputing measurements. Browser JavaScript may remain embedded in the generated offline report. Browser JavaScript is not a benchmark runtime dependency.

## Numerical implementation

The implementation must use f64 for the standard protocol's native diagnostics, color transforms, Lab calculations, statistics, and spectrum normalization unless the protocol explicitly identifies another precision. Artifact storage must not imply a higher computation precision.

LDR-FLIP must keep the pinned official implementation required by the [measurement protocol](../metrics/spec.md#perceptual-spatial-difference). A narrow native bridge to the official C++ implementation is the selected integration boundary. The bridge must identify its source revision and build settings. It must use the declared input domain and pixels-per-degree settings. It must return the official error map without a palette transform. An unverified reimplementation must not replace the official FLIP implementation.

ICC conversion must continue to use an identified LittleCMS implementation with the existing transform settings. The release must not fall back to an approximate RGB transform when ICC conversion is unavailable.

Reused Rust color functions must be verified against the standard protocol before adoption. The same function name or color-space label does not establish equivalence. Verification must cover white points, chromatic adaptation, transfer functions, negative values, super-white values, and clipping boundaries.

Gaussian filtering must preserve kernel support and boundary extension. FFT measurements must preserve frequency coordinates, window definitions, masks, and normalization. Percentiles must preserve the declared interpolation method and population. Covariance and multi-seed statistics must preserve degrees of freedom, eligibility, and failure precedence.

Exact integer decoding must preserve RGB16 PNG channel codes, including filtered and interlaced inputs. PNG detail crops must preserve the generated full-image channel codes. Display conversion and resampling remain subject to the product's separate display-copy contract.

## Existing evidence and protocol identity

The Rust readers must accept supported schema-version-1 authored documents and completed Python-produced run and comparison records. They must verify the recorded identity using that record's original rules. Reading an old record must not reinterpret its implementation identity as the Rust implementation.

A report generated from an existing comparison must retain its measurement values, availability, findings, and evaluation result. Report markup and PNG compressed bytes may differ. Decoded full-image and detail channel codes must remain exact when the report copies or crops existing display artifacts.

A new comparison must have a new resolved measurement identity. That identity must include the benchmark implementation revision and relevant numerical-library versions and settings. A preserved schema version does not imply a preserved measurement identity.

A policy bound to a Python measurement identity must not qualify a new Rust comparison. The policy must be rebound to the Rust identity with reviewed calibration evidence. The benchmark must not rewrite or automatically approve an old policy. An existing comparison's recorded policy result remains historical evidence when that comparison is reported.

Run comparison must remain possible after original source files and the renderer have become unavailable, using the retained dependencies required by the interface. The implementation must not reintroduce reads from authored paths during comparison or report generation.
