# Product specifications

`spec.md` defines user-visible behavior. `design.md` defines system boundaries and invariants.

Capabilities:

- [Film chain](film-chain/spec.md) — spectral film, print, and scan processing.
- [Film grain](film-grain/spec.md) — shared grain contract and engine selection.
- [Image input](image-input/spec.md) — raster and RAW ingestion.
- [Photo export](photo-export/spec.md) — preview output, Save, and Export.
- [LUT delivery](lut-delivery/spec.md) — deterministic LUT bundles.
- [GUI workflow](gui-workflow/spec.md) — interactive control and state behavior.

Each capability owns its contract. Cross-capability rules are stated once in the owning document and linked elsewhere.
