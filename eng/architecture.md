# Crate boundaries and ownership

This page records the production runtime boundary and feature ownership within the existing crates.

## Repository layout

```
crates/
  spektrafilm-math/       f64 reference math (spectral, interpolation, PCHIP, RNG)
  spektrafilm-model/      stochastic and physical models (grain, halation, DIR couplers, glare)
  spektrafilm-core/       runtime facade, pipeline orchestration, profiles, stages
  spektrafilm-gpu/        ComputeBackend trait with CPU (rayon + BLAS) and WGPU backends
  spektrafilm-shaders/    spectral WGSL shaders and standalone Metal sources
  spektrafilm-cli/        `spektrafilm`, `spektrafilm-f64`, `decode_raw_gui`, `fit` and `lut`
  spektrafilm-gui/        egui/eframe preview window
  spektrafilm-raw/        LibRaw white balance and Lensfun correction
  spektrafilm-benchmark/  benchmark CLI, fixture generator, metrics and offline review
data/
  profiles/               film and paper JSON profiles
  luts/                   Hanatos2025 spectral basis and observer data
  filters/                neutral-print enlarger filter database
  icc/                    upstream ICC assets and their notices
  presets/                preset documents compiled into the binary
  illuminants/            measured illuminant spectra
  license/                canonical spectral-data license
licenses/                 third-party license texts shipped with packages
scripts/parity/           Python 0.3.4 to Rust differential harness and acceptance runners
```

## Runtime boundary

Production image execution enters through `spektrafilm_core::runtime`.
`RuntimePhotoParams` owns the profile/data boundary.
It applies `DigestMode::PreserveUserEdits` for GUI edits or `DigestMode::ApplyStockSpecifics` for CLI/LUT construction.
`Runtime` owns the calibrated pipeline and exposes processing/taps/timings.
GUI Preview, Scan, and independent Export use this facade directly with preserved user edits.
Export owns its input and runtime snapshot.
CPU f64 reference/export, WGPU f32 execution, and explicit per-stage CPU fallback remain backend-specific below this boundary.

Shared film/print curve and spectral preparation lives in `chain_prep`.
Stages and the resident builder consume the same derivations.
Backend-specific arithmetic, resource ownership, and dispatch remain separate.
See the current P1+P2 record in the project context workspace for the execution contract and verification limits.

## Feature ownership

Feature ownership stays inside the existing crates:

- `spektrafilm-model/src/`: `grain/{v1,v2}.rs`, `halation/`, `diffusion/`, `couplers/`, and `glare/` own their models and numerical tests. `optics/` owns shared physical blur, unsharp masking, and highlight boost.
- `spektrafilm-core/src/params/`: grain, halation, diffusion, couplers, and glare each own their parameter types and feature-specific defaults. `RuntimeParams` remains the aggregate. Serialized JSON fields are unchanged.
- `spektrafilm-gpu/src/wgpu_backend/`: each effect owns its buffers, pass encoding, and dedicated WGSL. `blur/` owns Gaussian FIR weight generation and its shared uniform layout. `film_chain.rs` owns resident orchestration and its resource lifetimes. `mod.rs` retains device setup and shared dispatch.
- `spektrafilm-cli/src/process.rs` owns process arguments, invocation setup, preflight, rendering, and publication. `main.rs` retains command dispatch and shared data selection.
- `spektrafilm-core/src/lut_baker/bundle.rs` owns canonical topology stage roles, taps, suffixes, and print sharing. Baking, QA, and OCIO consume those descriptors, including magazine final-stage roles.
- `spektrafilm-core/src/pipeline_calibration.rs` applies input-gamut remapping at one boundary after method-specific spectral LUT construction. The matrix-only Mallett path stays separate.
- `spektrafilm-gui/src/main.rs` owns contiguous parameter-revision/dirty transitions, with a separate calibration-invalidating transition. State restoration retains its explicit ordering around profile and viewer updates.
- `spektrafilm-benchmark/src/documents.rs` owns authored-document validation and shared document helpers. `documents/evidence.rs` owns generated run/comparison validation. `documents/masks.rs` owns render/reference geometry-mask decoding with their distinct acceptance rules.

Canonical Rust paths include `spektrafilm_model::grain::v1`, `spektrafilm_model::grain::v2`, and `spektrafilm_core::params::grain::GrainParams`.
The former flat module paths have been removed.

## See also

- [Project context](../CONTEXT.md).
- [Pipeline](../docs/pipeline.md).
- [Parity evidence](../docs/parity.md).
- [Benchmarking](../docs/benchmark.md).
- [Film-chain specification](../specs/film-chain/spec.md).
