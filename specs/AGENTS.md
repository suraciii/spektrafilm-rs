# Specification Instructions

- `spec.md` defines target product behavior and is the source of truth.
- `design.md` defines target system design, boundaries, and invariants.
- Write durable target state only. Do not record plans, research, gaps, progress, logs, or test output.
- Use top-level capability directories by default; `simulation/` and `delivery/` are optional contexts, not required grouping layers.
- Add one nested level only for an independent variant or capability with its own behavior and design, such as `film-grain/v1/` and `film-grain/v2/`.
- Give a parent directory `spec.md` or `design.md` only when it owns a real shared contract; otherwise keep capabilities as siblings.
- Keep one fact in one authoritative document; link instead of duplicating rules.
- Keep transient material outside the repository, normally under `/data/workspaces/spektrafilm-project-context/`.
- Put durable non-contract conclusions in the relevant Issue/PR.
- Do not use `docs/` or implementation notes as a competing product or design authority.
