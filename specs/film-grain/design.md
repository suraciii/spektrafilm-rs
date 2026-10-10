# Film grain design

- The shared grain parameter object selects one engine and carries its controls; fields for inactive engines are ignored. V3's planned controls are defined in its [design](v3/design.md#parameters).
- Profile resolution happens before rendering: named V2 profiles provide defaults, and `custom` applies explicit overrides. Validation rejects unknown profiles, non-finite values, and out-of-range controls.
- V1 owns the density-domain sampler. V2 owns the encoded-RGB procedural model. Neither engine calls the other or shares an approximation layer.
- Backend selection follows the engine contract. V1 uses the faithful CPU stage; V2 may use the resident WGPU implementation when the complete resident chain is eligible.
- The caller receives one grain result in the engine's declared domain. Color conversion and transfer encoding are not duplicated around the effect.
- Grain must remain a stage of the film chain, not a separate public pipeline or generic effect graph.
- V3 owns the conditional film-coordinate dye field and area readout. Its [design](v3/design.md) defines parameter resolution, full-film geometry, count statistics, CPU routing, and numerical certification. Its [product contract](v3/spec.md) defines supported conditions and photographic acceptance.
