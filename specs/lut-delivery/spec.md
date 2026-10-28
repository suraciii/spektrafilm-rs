# LUT delivery

## Contract

- LUT delivery builds a deterministic transform from the selected base workflow, color settings, and runtime controls.
- Optional [CMYK ink style](../cmyk-style/spec.md) is a modeled RGB finishing behavior, not a photographic-print calibration claim.
- A LUT bake contains no image-specific spatial effects or stochastic grain.
- The bundle records any controls neutralized for baking.
- The negative frame setup (`scanner.experimental_setup`) is frame-dependent and MUST fail baking while enabled.
- It MUST NOT be silently neutralized.
- A declared fixed style is the point-wise alternative under the [lab scan contract](../film-chain/lab-scan/spec.md#point-wise-consumers-and-migration).
- The bundle MAY contain canonical LUT stages, contiguous collapsed sub-chains, machine-readable metadata, OCIO wiring, and QA results.
- LUT domains, ranges, color spaces, stock selections, and wire constants are part of the bundle contract.
- A consumer MUST NOT cross-chain files from different bundles.
- Target validation rejects unsupported inputs, outputs, topology, resolution, or artifact references before baking.
- The bundle records the bake parameters and provenance needed to reproduce or audit its output.
- Lattice computation and reference alignment follow [GPU execution](../gpu-execution/spec.md).
- Artifact assembly, reference QA, and file publication retain their delivery ownership.
- They MUST NOT imply a different lattice computation precision.
- Bundles with active CMYK ink style MUST record the base workflow and finishing definition independently.
- They MUST use an explicit `cmyk_` final-stage role so they cannot be mistaken for photographic-print LUT stages.
- Existing material exposure, preflash, and development MUST be evaluated only on routes containing those stages.
- No digital-response layer or new scan-reference model is added.
- The bundle retains physical film-density and print-density wire meanings.
- [Print exposure composition](../output-mix/design.md#print-exposure-composition) MUST share its main/preflash filtration and exposure-gain derivation with image rendering.
- Uniform preflash is pointwise and LUT-bakeable; active spatial propagation is not.
- Baking MUST neutralize and record optical diffusion and scanner blur under the existing spatial-effect rule.
- Baking MUST NOT move either after nonlinear development or interpretation.
- A neutralized bake does not claim equivalence to active spatial rendering.
- Output mix below 1 MUST be supported only by a complete one-LUT source-to-output bake with the prepared source branch expressed in the same transport and geometry.
- Segmented topologies MUST reject that selection before baking because their scan-stage input does not carry the source branch.
- The baker MUST NOT silently reset output amount to 1.
- Image-specific geometry remains neutralized under the existing LUT contract.
- CMYK ink style MUST run once at its post-scan finishing boundary.
- The final RGB stage retains its `cmyk_` role and records its definition independently of material stages.
- Neither a segmented consumer nor a collapsed complete transform MAY reapply CMYK ink style.
- Overall output mix follows CMYK ink style in linear output RGB.
- A complete one-LUT mixed transform MUST retain appearance and amount provenance even though intermediate nodes are collapsed.
- Existing output gamut algorithms remain unchanged except for standalone lightness retirement under [Output boundary and retirement](../output-mix/spec.md#output-boundary-and-retirement).
- A bake MUST NOT introduce a replacement final RGB response, output lightness stage, or separate `input`-workflow output-processing switch.
