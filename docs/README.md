# Documentation

These pages explain how to build, run and evaluate SpektraFilm. They describe the product as it is used; they do not define product contracts.

Contracts live in [../specs/](../specs/). Domain terms live in [../CONTEXT.md](../CONTEXT.md). Engineering practices live in [../eng/](../eng/).

## Guides

- [Building, packaging, and verification](building.md) — dependencies, build commands, package scripts, install locations, and which gate runs when.
- [Simulation pipeline and profiles](pipeline.md) — the spectral chain, selectable upsampling methods, camera filters, and shipped profiles.
- [GUI guide](gui.md) — controls, Preview, Scan, Save, Export, states, and diagnostics.
- [CLI guide](cli.md) — commands, formats, parameter overrides, geometry, color management, and reports.
- [Scanning and negative conversion](scanning.md) — scan output modes, scanner controls, film-scan points and the CineStill approximation.
- [Presets](presets.md) — look and section documents, the user library and `--set` overrides.
- [Grain engines](grain.md) — Spektra, Procedural, Dye and Fotufilm Film, with their controls.
- [RAW decoding](raw.md) — the shared LibRaw and Lensfun decoder, white balance modes, and the RAW helper.
- [Profile fitting](profile-fitting.md) — deriving surface terms and conversion tone from reference patch data.
- [Measurement](measurement.md) — the instrument's stimulus documents, point-wise readings, reading comparison, and verification.
- [LUT delivery](luts.md) — the bundle format, the LUT baker, OCIO output and QA artifacts.
- [Appearance benchmark](benchmark.md) — rendering version-comparison evidence and the offline review.
- [Parity and backends](parity.md) — the pinned Python target, measurement budgets, and backend/fallback behavior.
- [Attribution](attribution.md) — upstream sources, third-party components, and their license terms.

## Evidence

- [Baseline parity evidence and budgets](parity/baseline_evidence.md) — reference pins and measurement budgets.
- [Parity matrix](parity/parity_matrix.md) — the field and asset inventory with its machine-readable source.
- [Python ↔ Rust harness](../scripts/parity/README.md) — the differential runner and its acceptance scopes.
