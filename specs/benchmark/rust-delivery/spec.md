# Rust benchmark delivery

## Scope and authority

This document defines the delivered implementation's scope, distribution, workspace and execution boundary, numerical implementation, and protocol identity. The [product specification](../spec.md) owns evaluation meaning and human review. The [machine interface](../../instrument/spec.md) owns commands and documents.

The [measurement protocol](../metrics/spec.md) owns measurement semantics. The [system design](../../instrument/design.md) owns runtime boundaries.

The delivered implementation MUST provide the complete instrument as a Rust CLI. All existing command families, reference scopes, measurement families, failure states, and review capabilities MUST remain supported.

The benchmark MUST NOT change film simulation algorithms, add measurement families, change qualification budgets, or expand the reference corpus.

## Installation and distribution

The release MUST include `spektrafilm-lab` and its `fixtures` command. The `fixtures` command MUST retain its current options and source attribution behavior, and it MUST answer the [standard configuration generation](../../instrument/spec.md#standard-configuration-generation) contract by invoking the selected renderer's preset export command. The instrument and its fixture generation MUST run without Python, uv, pip, or Python packages.

The release ships the instrument's supported commands as the surface a release user gets. An experimental command ships with the binary and carries no compatibility promise.

Release archives MUST provide binaries for the platforms supported by the application's packaging workflows. Every non-system runtime dependency MUST be bundled or linked into the delivered executable. The release MUST identify required system libraries and third-party licenses.

A Rust executable with C or C++ dependencies satisfies this requirement; a pure-Rust dependency graph is not required.

Help, version, and schema discovery MUST work before native metric libraries or renderer dependencies are loaded. The installed commands MUST work from a directory outside the source checkout. Optional natural-image acquisition MUST remain explicit.

Tests MUST NOT require network acquisition of images.

## Workspace and execution boundary

The Rust implementation MUST use workspace boundaries selected from actual dependencies. This specification does not prescribe a new crate or module hierarchy. Its executable MUST continue to invoke the renderer path declared by the target document.

It MUST NOT substitute a linked runtime for that renderer.

The instrument MUST support historical renderer executables and retained results independently of the current application's build. Compiling the benchmark MUST NOT require a GPU runtime or GUI toolkit solely to obtain color math, image decoding, or document helpers. Shared mathematical code MAY be reused through an existing dependency boundary when that boundary meets this constraint.

The implementation MUST use typed internal records for documents, measurements, artifact roles, availability, and terminal states. JSON remains the public carrier. Typed records MUST preserve the interface's different rules for authored and generated documents.

Runtime validation remains responsible for hashes, paths, geometry, populations, and cross-document identities.

Metric adapters MUST return structured evidence. A report MUST consume that evidence without recomputing measurements. Browser JavaScript MAY remain embedded in the generated offline report.

Browser JavaScript is not a benchmark runtime dependency. The delivered benchmark MAY include the benchmark-owned Photo Report builder defined by [Photo Report](../photo-report/spec.md). It MUST use the existing image decoding and display-conversion boundaries, MUST NOT add a renderer dependency, and MUST remain a static offline artifact producer.

## Numerical implementation

The implementation MUST use f64 for the standard protocol's native diagnostics, color transforms, Lab calculations, statistics, and spectrum normalization unless the protocol explicitly identifies another precision. Artifact storage MUST NOT imply a higher computation precision.

LDR-FLIP MUST keep the pinned official implementation required by the [measurement protocol](../metrics/spec.md#perceptual-spatial-difference). A narrow native bridge to the official C++ implementation is the selected integration boundary. The bridge MUST identify its source revision and build settings.

It MUST use the declared input domain and pixels-per-degree settings. It MUST return the official error map without a palette transform. An unverified reimplementation MUST NOT replace the official FLIP implementation.

ICC conversion MUST continue to use an identified LittleCMS implementation with the existing transform settings. The release MUST NOT fall back to an approximate RGB transform when ICC conversion is unavailable.

Reused Rust color functions MUST be verified against the standard protocol before adoption. The same function name or color-space label does not establish equivalence. Verification MUST cover white points, chromatic adaptation, transfer functions, negative values, super-white values, and clipping boundaries.

Gaussian filtering MUST preserve kernel support and boundary extension. FFT measurements MUST preserve frequency coordinates, window definitions, masks, and normalization. Percentiles MUST preserve the declared interpolation method and population.

Covariance and multi-seed statistics MUST preserve degrees of freedom, eligibility, and failure precedence.

Exact integer decoding MUST preserve RGB16 PNG channel codes, including filtered and interlaced inputs. PNG detail crops MUST preserve the generated full-image channel codes. Display conversion and resampling remain subject to the product's separate display-copy contract.

## Existing evidence and protocol identity

The Rust readers MUST accept supported schema-version-1 authored documents and completed Python-produced run and comparison records. They MUST verify the recorded identity using that record's original rules. Reading an old record MUST NOT reinterpret its implementation identity as the Rust implementation.

A report generated from an existing comparison MUST retain its measurement values, availability, findings, and evaluation result. Report markup and PNG compressed bytes MAY differ. Decoded full-image and detail channel codes MUST remain exact when the report copies or crops existing display artifacts.

A new comparison MUST have a new resolved measurement identity. That identity MUST include the benchmark implementation revision and relevant numerical-library versions and settings. A preserved schema version does not imply a preserved measurement identity.

A policy bound to a Python measurement identity MUST NOT qualify a new Rust comparison. The policy MUST be rebound to the Rust identity with reviewed calibration evidence. The benchmark MUST NOT rewrite or automatically approve an old policy.

An existing comparison's recorded policy result remains historical evidence when that comparison is reported.

Run comparison MUST remain possible after original source files and the renderer have become unavailable, using the retained dependencies required by the interface. The implementation MUST NOT reintroduce reads from authored paths during comparison or report generation.
