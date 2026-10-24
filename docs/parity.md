# Parity and backends

This page describes the migration reference, measurement budgets, and backend behavior.

## Reference evidence

The migration reference was the pinned upstream Python **0.3.4**, commit `3bb2c2d`; the record is frozen. See [baseline evidence](parity/baseline_evidence.md) for reference measurements and budgets. See the [parity matrix](parity/parity_matrix.md) for the field and asset inventory. The [differential harness](../scripts/parity/README.md) provides reproducible comparisons.

With grain on, the rendered grain texture diverges per pixel by design, matching NumPy's behavior. It produces the same average tone with a different grain pattern.

## Measurement budgets

Budgets apply to different measurements:

- Reference f64 arithmetic uses a **1e-6** maximum absolute-error budget against the pinned matrix.
- CPU-f32/GPU arithmetic is measured by `just parity-backend` against the CPU
  reference over the color-space, control, fence and grain matrix. Both
  precisions use a **5e-3** maximum absolute-error budget. Measured worst cases:
  f64 grain fence **4.1e-4**, f32 controls path **1.0e-4**; the budget keeps an
  order of magnitude of headroom while still failing the 2.9e-1 input-CCTF
  double-decode regression it was introduced to catch.
- Stochastic appearance uses a provisional **1 % mean / 5 % standard deviation** relative-error budget, not per-pixel equality.
- The spatial **0.005** hotspot limit guards the faithful diffusion fallback. It does not approve a GPU blur approximation.

End-to-end preview and other spatial budgets in the [baseline evidence](parity/baseline_evidence.md) remain provisional until actual scenario measurements are recorded. Adapter initialization or compilation alone does not establish parity.

The executed f64 LUT-reduction runs use a **1e-5** maximum absolute-error budget. The scenario identities live in [`scripts/parity/scenarios.py`](../scripts/parity/scenarios.py) and their retained results in the [parity matrix](parity/parity_matrix.json).

## Backend selection

WGPU/WGSL is the only GPU backend. Select it explicitly with `SPEKTRAFILM_BACKEND=wgpu`. Automatic selection may initially choose CPU when no usable hardware GPU is available.

GUI Preview and Scan prefer GPU execution. A selected GPU operation must fail before publication when an effect cannot run on the device; it must not insert faithful CPU stages. GUI CPU execution uses the build's native precision. The separate `spektrafilm-f64` CLI remains the CPU f64 reference. Mesa software Vulkan (`mesa-vulkan-drivers`) supports portability checks, not hardware-GPU acceptance.

Backend selection in a `precision-f64` binary defaults to genuine CPU f64 arithmetic. `SPEKTRAFILM_BACKEND=wgpu` explicitly requests f32 WGPU preview arithmetic. Backend initialization reports `precision=f32` and `reference=false`.

An explicit `SPEKTRAFILM_BACKEND=wgpu` request that cannot be served — an unavailable adapter, or a build compiled without the `wgpu-backend` feature — must fail before output publication. Automatic selection without usable hardware may initially choose CPU and must report its reason. After GPU selection, either mode must fail on unsupported work or device failure without a CPU restart. An automatic CPU path is not evidence of a GPU pass.

## Selected-executor fidelity

Input transfer decoding, grain, gamut processing, optical diffusion, and selected enlarger/scanner PCHIP spectral LUTs must retain their reference algorithms on the selected executor. Missing GPU support must fail a GPU-selected operation, not select a faithful CPU per-stage path. Historical mixed-path measurements remain evidence only for the work they actually exercised.

Gaussian blur must retain its requested support and boundary behavior. Neither a small sigma nor support beyond the historical 256-pixel FIR half-width permits CPU fallback after GPU selection. Device execution must preserve the reference kernel or fail explicitly; it must not truncate the kernel to fit a device limit.

Optical diffusion resolves the reference per-channel sampled PSF on the host, then evaluates all taps on the device using binary64 accumulation. Bounded windows gather reflected global image coordinates; when the PSF itself exceeds one storage binding, tap blocks retain full support and accumulation order. Camera diffusion operates on linear exposure after highlight boost and before lens blur. Enlarger diffusion recovers and scales inner-log irradiance before convolution, then emits the outer log used by paper development. Unsupported device arithmetic fails instead of falling back to CPU. These implementation contracts are not measured parity evidence.

Generated scanner/enlarger PCHIP lattices, slopes, cell hulls, and interpolation retain binary64 precision. Scanner interpolation applies the power-of-ten reconstruction and separate CAT/RGB matrices before its f32 image output boundary, avoiding a prematurely rounded log-XYZ image. Inverse conversion excludes coefficient rows that the reference rejects as missing before device upload, retaining the original normalization, seed, and wavelength order.

Resident direct spectral scanning likewise retains binary64 wavelength coefficients, transmittance evaluation, and spectral reduction, including the reference normalized log/power-of-ten roundtrip and separate CAT/RGB transforms. Only the retained image ingress and final image output use f32 storage.

Isolated scanner-acquisition acceptance reports absolute **linear destination RGB** error against CPU acquisition from identical physical input samples; its maximum-error limit is **1e-5**. Reports distinguish retained-f32 input quantization from the CPU-f64 source and retain encoded-output differences separately. Encoded final-route measurements keep the owning end-to-end budgets; an output-transfer slope does not change the isolated acquisition units.

## See also

- [Documentation index](README.md)
- [Build and installation](building.md)
- [Pipeline](pipeline.md)
- [Grain](grain.md)
- [LUTs](luts.md)
- [Benchmark](benchmark.md)
- [Baseline evidence](parity/baseline_evidence.md)
- [Parity matrix](parity/parity_matrix.md)
- [Differential harness](../scripts/parity/README.md)
- [CPU-vs-GPU parity](../Justfile) — `just parity-backend`
