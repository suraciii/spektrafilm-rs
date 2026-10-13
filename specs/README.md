# Product specifications

`spec.md` defines user-visible behavior. `design.md` defines system boundaries and invariants.

Capabilities:

- [Film chain](film-chain/spec.md) — spectral film, print, and scan processing.
- [GPU execution](gpu-execution/spec.md) — GPU-first full image computation, device residency, CPU reference alignment, and hardware acceptance.
- [Digital Reference scan](film-chain/digital-reference/spec.md) — common modeled reference conversion within Scan, with fixed Reference Exposure placement.
- [Output mix and material controls](output-mix/spec.md) — final control scope: reuse preflash and chemistry, retire Output lightness compression, and mix the complete result with its prepared source.
- [Magazine finishing](magazine-finishing/spec.md) — optional magazine-style RGB finishing for scanned or completed images.
- [Film grain](film-grain/spec.md) — shared grain contract and engine selection.
- [Image input](image-input/spec.md) — raster and RAW ingestion.
- [Photo export](photo-export/spec.md) — preview output, Save, and Export.
- [LUT delivery](lut-delivery/spec.md) — deterministic LUT bundles.
- [GUI workflow](gui-workflow/spec.md) — interactive control and state behavior.
- [Presets](presets/spec.md) — portable look parameters, the user library, and GUI state compatibility.
- [Profile authoring](profile-authoring/spec.md) — deriving profile and look values from reference patch data.
- [CLI](cli/spec.md) — source declarations, presets, sparse parameter overrides, seeds, discovery, dry run, help, version identity, data selection, command status, and human feedback.
- [Film simulation benchmark](benchmark/spec.md) — agent-facing version comparison, reference alignment, measurements, and visual review artifacts.
- [Telemetry](telemetry/spec.md) — operation cost and execution observations, including the report contract.

Each capability owns its contract. Cross-capability rules are stated once in the owning document and linked elsewhere.

Domain terms follow the [Film simulation language](../CONTEXT.md).
