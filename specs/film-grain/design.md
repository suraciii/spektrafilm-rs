# Film grain design

- The shared grain parameter object selects one engine and carries its controls; fields for inactive engines are ignored. Dye's controls are defined in its [design](v3/design.md#parameters); Fotufilm Film's controls are defined in its [product contract](fotufilm/spec.md#parameters).
- Profile resolution happens before rendering: named Procedural profiles provide defaults, and `custom` applies explicit overrides. Validation rejects unknown profiles, non-finite values, and out-of-range controls.
- Spektra owns the density-domain sampler. Procedural owns the encoded-RGB procedural model. Neither engine calls the other or shares an approximation layer. Fotufilm Film owns the fitted-population residual that core applies at its own declared film-development boundary.
- Backend selection follows the engine contract. Spektra uses the faithful CPU stage; Procedural may use the resident WGPU implementation when the complete resident chain is eligible.
- The caller receives one grain result in the engine's declared domain. Color conversion and transfer encoding are not duplicated around the effect.
- Grain must remain a stage of the film chain, not a separate public pipeline or generic effect graph.
- Dye owns the conditional film-coordinate dye field and area readout. Its [design](v3/design.md) defines parameter resolution, full-film geometry, count statistics, CPU routing, and numerical certification. Its [product contract](v3/spec.md) defines supported conditions and photographic acceptance.
- Fotufilm Film supplies a model that core resolves against the loaded film curve. Its [design](fotufilm/design.md) defines density adaptation, formation and calibration, complete periodic footprints, geometry, precision, and evidence; its [product contract](fotufilm/spec.md) defines parameters, dispatch, and acceptance.
