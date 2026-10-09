# LUT delivery design

- `BundleBuilder` owns target validation, parameter normalization, lattice construction, and artifact assembly.
- LUT construction uses the core calibrated spectral runtime and f64 arithmetic. Shared film stages are built once when several print targets share them.
- Printing and scanning use one authoritative LUT-grid layout and coordinate convention. Each stage owns its physical conversion and output interpretation.
- Transport artifacts carry their role, domain, range, color space, and wire metadata. OCIO and documentation are generated from the same bundle metadata.
- QA evaluates the emitted bundle and is stored as delivery metadata; it does not redefine the film-chain contract.
