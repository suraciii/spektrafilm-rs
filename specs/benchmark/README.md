# Benchmark

The benchmark defines repeatable evidence for film-simulation changes. It serves agent-facing automation and human visual review from the same immutable render results.

- [Product specification](spec.md) — evaluation purpose, tracks, data boundaries, review behavior, and acceptance scenarios.
- [Standard configurations](standard-configurations.md) — reusable color-and-tone and full-appearance setup, frozen look files, scope limits, and acceptance.
- [System design](design.md) — ownership, execution boundaries, identity, caching, and failure invariants.
- [Measurement protocol](metrics.md) — color, tone, perceptual, grain, halation, edge, numeric-health, and calibration rules.
- [Machine interface](interface.md) — `spektrafilm-bench` commands, document contracts, artifacts, and statuses.
- [Rust delivery](rust-delivery.md) — runtime distribution, numerical agreement, resource acceptance, migration, and cutover.
