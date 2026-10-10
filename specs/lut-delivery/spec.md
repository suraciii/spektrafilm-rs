# LUT delivery

## Contract

- LUT delivery builds a deterministic transform from the selected base workflow, color settings, and runtime controls. Optional [magazine print color](../magazine-print-color/spec.md) is finishing behavior, not a photographic-print calibration claim.
- A LUT bake contains no image-specific spatial effects or stochastic grain. The bundle records any controls neutralized for baking.
- The bundle may contain canonical LUT stages, contiguous collapsed sub-chains, machine-readable metadata, OCIO wiring, and QA results.
- LUT domains, ranges, color spaces, stock selections, and wire constants are part of the bundle contract. A consumer must not cross-chain files from different bundles.
- Target validation rejects unsupported inputs, outputs, topology, resolution, or artifact references before baking.
- The bundle records the bake parameters and provenance needed to reproduce or audit its output.
- Bundles with active magazine color must record the base workflow and finishing definition independently. They must use an explicit `magazine_` final-stage role so they cannot be mistaken for photographic-print LUT stages.
