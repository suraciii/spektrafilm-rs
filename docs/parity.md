# Parity and backends

This page describes the parity target, measurement budgets, and backend behavior.

## Reference evidence

The current parity target is the pinned upstream Python **0.3.4**, commit `3bb2c2d`. See [baseline evidence](parity/baseline_evidence.md) for reference measurements and budgets. See the [parity matrix](parity/parity_matrix.md) for the field and asset inventory. The [differential harness](../scripts/parity/README.md) provides reproducible comparisons.

With grain on, the rendered grain texture diverges per pixel by design, matching NumPy's behavior. It produces the same average tone with a different grain pattern.

## Measurement budgets

Budgets apply to different measurements:

- Reference f64 arithmetic uses a **1e-6** maximum absolute-error budget against the pinned matrix.
- CPU-f32/GPU arithmetic must report its measured maximum and mean error independently. It has no approved universal budget yet.
- Stochastic appearance uses a provisional **1 % mean / 5 % standard deviation** relative-error budget, not per-pixel equality.
- The spatial **0.005** hotspot limit guards the faithful diffusion fallback. It does not approve a GPU blur approximation.

End-to-end preview and other spatial budgets in the [baseline evidence](parity/baseline_evidence.md) remain provisional until actual scenario measurements are recorded. Adapter initialization or compilation alone does not establish parity.

The executed f64 LUT-reduction runs use a **1e-5** maximum absolute-error budget. The scenario identities live in [`scripts/parity/scenarios.py`](../scripts/parity/scenarios.py) and their retained results in the [parity matrix](parity/parity_matrix.json).

## Backend selection

WGPU/WGSL is the only GPU backend. Select it explicitly with `SPEKTRAFILM_BACKEND=wgpu`. CPU is the fallback when no usable adapter is available.

GUI Preview and Scan prefer GPU execution with faithful CPU stages for unsupported effects. GUI CPU execution uses the build's native precision. The separate `spektrafilm-f64` CLI remains the CPU f64 reference. Linux GPU acceptance requires the Mesa software Vulkan adapter (`mesa-vulkan-drivers`).

Backend selection in a `precision-f64` binary defaults to genuine CPU f64 arithmetic. `SPEKTRAFILM_BACKEND=wgpu` explicitly requests f32 WGPU preview arithmetic. Backend initialization reports `precision=f32` and `reference=false`.

An unavailable WGPU adapter reports its unavailability and uses CPU. That fallback is not evidence of a GPU pass.

## Faithful fallback

Input transfer decoding, Grain Spektra, active unsupported gamut algorithms/spaces, and camera/enlarger optical diffusion use an explicitly reported faithful CPU per-stage path. Optical diffusion uses the finite sampled, normalized PSF with reflected-boundary FFT convolution on CPU to preserve the requested effect.

Requests for enlarger/scanner PCHIP spectral LUTs also select the per-stage path. This preserves the requested sampling and interpolation.

Gaussian requests exceeding the GPU's 256-pixel FIR half-width explicitly fall back to CPU blur. This includes resident halation bounce radii, DIR, glare, scanner, and camera blur. The requested radius is preserved instead of silently truncated.

Grain Procedural uses its resident GPU shader when the rest of the resident-chain requirements are met.

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
