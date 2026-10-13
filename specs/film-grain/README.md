# Film grain

The film grain capability owns the shared grain parameter object, engine selection, and the per-engine contracts.

- [Product specification](spec.md) — routes, selection values, and shared contract.
- [System design](design.md) — resolution, backend, and ownership boundaries.

Capabilities:

- [Spektra](v1/spec.md) — the default density-domain engine.
- [Procedural](v2/spec.md) — the encoded-RGB post-scan appearance engine.
- [Dye](v3/spec.md) — the conditional film-coordinate dye field engine.
- [Grain Dye diagnostic probes](v3/probes/spec.md) — opt-in engineering controls for the Dye model.
- [Fotufilm Film](fotufilm/spec.md) — the fitted-population residual engine.
