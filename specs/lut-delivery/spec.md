# LUT delivery

## Contract

- LUT delivery builds a deterministic transform from the selected base workflow, color settings, and runtime controls. Optional [magazine finishing](../magazine-finishing/spec.md) is finishing behavior, not a photographic-print calibration claim.
- A LUT bake contains no image-specific spatial effects or stochastic grain. The bundle records any controls neutralized for baking.
- The bundle may contain canonical LUT stages, contiguous collapsed sub-chains, machine-readable metadata, OCIO wiring, and QA results.
- LUT domains, ranges, color spaces, stock selections, and wire constants are part of the bundle contract. A consumer must not cross-chain files from different bundles.
- Target validation rejects unsupported inputs, outputs, topology, resolution, or artifact references before baking.
- The bundle records the bake parameters and provenance needed to reproduce or audit its output.
- Lattice computation and reference alignment follow [GPU execution](../gpu-execution/spec.md). Artifact assembly, reference QA, and file publication retain their delivery ownership and must not imply a different lattice computation precision.
- Bundles with active magazine color must record the base workflow and finishing definition independently. They must use an explicit `magazine_` final-stage role so they cannot be mistaken for photographic-print LUT stages.
- Existing material exposure, preflash, and development must be evaluated only on routes containing those stages. No digital-response layer or new scan-reference model is added. The bundle retains physical film-density and print-density wire meanings.
- [Print exposure composition](../output-mix/design.md#print-exposure-composition) must share its main/preflash filtration and exposure-gain derivation with image rendering. Uniform preflash is pointwise and LUT-bakeable; active spatial propagation is not. Baking must neutralize and record optical diffusion and scanner blur under the existing spatial-effect rule, not move either after nonlinear development or interpretation. A neutralized bake does not claim equivalence to active spatial rendering.
- Output mix below 1 must be supported only by a complete one-LUT source-to-output bake with the prepared source branch expressed in the same transport and geometry. Segmented topologies must reject that selection before baking because their scan-stage input does not carry the source branch. The baker must not silently reset output amount to 1. Image-specific geometry remains neutralized under the existing LUT contract.
- Magazine appearance must run once at its existing post-scan finishing boundary. The final RGB stage retains its `magazine_` role and records its definition independently of material stages. Neither a segmented consumer nor a collapsed complete transform may reapply Magazine appearance. Overall output mix follows Magazine appearance in linear output RGB; a complete one-LUT mixed transform must retain appearance and amount provenance even though intermediate nodes are collapsed.
- Existing output gamut algorithms remain unchanged except for standalone lightness retirement under [Output boundary and retirement](../output-mix/spec.md#output-boundary-and-retirement). A bake must not introduce a replacement final RGB response, output lightness stage, or Finished RGB output-processing switch.
