# LUT delivery design

- `BundleBuilder` owns target validation, parameter normalization, lattice construction, and artifact assembly.
- `Topology` owns canonical stage descriptors (role, taps, suffix, and print sharing). Baking, QA, and OCIO derive stage selection and magazine final-stage roles from that owner; contiguous combination recipes retain the same stage ordering.
- LUT lattice evaluation uses the core calibrated spectral runtime under [GPU execution](../gpu-execution/spec.md). CPU f64 remains its numerical reference; GPU evaluation must meet the same delivery budgets and must not claim f64 arithmetic from artifact storage. Shared film stages are built once when several print targets share them.
- Printing and scanning use one authoritative LUT-grid layout and coordinate convention. Each stage owns its physical conversion and output interpretation.
- Lattice samples reuse existing material exposure, development and scanner preparation for their route. No material-color coordinate evaluator, tonal-mask preparation or separate baseline/effective reference model is added.
- [Print exposure composition](../material-response/design.md#print-exposure-composition) shares main and preflash filter derivation, existing exposure gains and numerical floors with the image path. Spatial operations remain outside pointwise LUTs and retain declared bake neutralization.
- [Output composition](../material-response/design.md#output-composition) must place the complete-result/prepared-source blend after existing output gamut processing and optional Magazine appearance, in common linear output RGB. A scan-stage LUT cannot reconstruct the source branch. Amount below 1 therefore requires a complete one-LUT source-to-output bake that supplies the branch explicitly; target validation rejects segmented delivery before baking rather than silently resetting the amount. No standalone output lightness stage or new Finished RGB output-processing switch is added.
- Transport artifacts carry their role, domain, range, color space, and wire metadata. OCIO and documentation are generated from the same bundle metadata.
- QA evaluates the emitted bundle and is stored as delivery metadata; it does not redefine the film-chain contract.
- A QA run publishes its report documents once. Delivery registration uses the report's artifact paths without publishing the documents again.
- A private QA stimulus module owns deterministic random streams, distribution tables, and hull index selection. Scenario evaluation owns sampling order and measurement logic.
- The private convex-hull module owns hull construction and volume calculation. QA scenarios retain metric thresholds and failure interpretation.
