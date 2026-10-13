# Specification Instructions

- Follow the repository's [documentation language rules](../eng/documentation-language.md).
- `spec.md` defines target product behavior and is the source of truth.
- `design.md` defines target system design, boundaries, and invariants.
- `interface.md` defines a caller-visible machine surface: commands, serialized documents, and statuses.
- `metrics.md` defines measured quantities: what is measured, how it is computed or reported, and its interpretation limits.
- These four names are the only contract filenames in `specs/`. A concern that needs its own contract becomes a capability directory with the same shape, as `film-grain/v3/` and `benchmark/standard-configurations/`. Nesting follows the same rule at each level.
- A capability directory owns `spec.md`. It adds `design.md` only when its internals need a contract.
- A parent directory keeps `spec.md` or `design.md` only when it owns a real shared contract; otherwise keep capabilities as siblings.
- Name a directory for the capability, variant, or concern it owns. Never encode one in a filename.
- Write durable target state only. Do not record plans, research, gaps, progress, logs, or test output.
- Use top-level capability directories by default; `simulation/` and `delivery/` are optional contexts, not required grouping layers.
- `specs/README.md` is the only index and lists every top-level capability; no capability adds its own `README.md`. `AGENTS.md` holds instructions. Neither file is a contract document.
- Keep one fact in one authoritative document; link instead of duplicating rules.
- Keep transient material outside the repository, normally under `/data/workspaces/spektrafilm-project-context/`.
- Put durable non-contract conclusions in the relevant Issue/PR.
- Do not use `docs/` or implementation notes as a competing product or design authority.
