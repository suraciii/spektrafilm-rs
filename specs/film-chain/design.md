# Film chain design

- `core::runtime` is the production entry point. `Pipeline` owns stage topology and stage state.
- Stage preparation derives film curves, spectral data, scan settings, color transforms, and working pixel pitch once for the selected execution.
- `ComputeBackend` separates stage orchestration from CPU and WGPU execution. The CPU backend preserves f64 reference order; WGPU owns f32 conversion, resources, and dispatch.
- A resident WGPU chain is an optimization, not a second product contract. `ResidentDecision` records capability and fallback reasons. Unsupported stages use the faithful per-stage route.
- The private resident module owns capability decisions, backend parameter preparation, and output transfer finalization. The parent pipeline owns input preparation and per-stage fallback ordering.
- The WGPU film-chain module owns resident command encoding and its resource lifetimes. Device setup and shared dispatch infrastructure remain in the backend.
- Filming metering owns the downsample, decode, luminance measurement, and exposure calculation. The pipeline meters the full input before crop and resize.
- Private Jzazbz and CAM16 modules own their appearance-model formulas and viewing adaptation. Output compression retains destination policy and the shared table cache.
- The WGPU blur module owns standalone Gaussian blur execution and shared blur limits remain backend policy. Resident stage-specific blur dispatch remains with its owning stage.
- Working pixel pitch is carried with the image through crop and resize and is consumed by spatial stages; stages do not infer it from the current raster size.
- Color-domain transitions are explicit. The chain does not apply an implicit transfer curve or primaries conversion inside an effect.
- No generic graph or backend-specific public parameter schema is required; runtime parameters remain the single caller-facing model.
- The magazine print appearance route delegates its color contract to [Magazine print color](../magazine-print-color/design.md).
