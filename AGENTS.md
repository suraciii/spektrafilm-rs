# Agent Instructions

## Project

SpektraFilm is a Rust spectral film simulation application with a native GUI, CLI, and LUT export.

- `specs/` — target product behavior and design.
- `docs/` — user-facing guidance.
- [CODING_STANDARDS.md](CODING_STANDARDS.md) — reviewer judgment rules.
- [eng/documentation-language.md](eng/documentation-language.md) — documentation language rules.

## Engineering

- Follow KISS and YAGNI. Reuse existing boundaries and dependencies.
- Keep each rule in one authoritative implementation.
- Preserve observable behavior and public contracts.
- Require measured evidence for numerical changes.
- Keep plans, research, probes, logs, build artifacts, and session state outside the repository, normally under `/data/workspaces/spektrafilm-project-context/`.
- Put durable non-contract conclusions in the relevant Issue/PR. Never commit transient artifacts, even if ignored.
- Write tests that catch meaningful failures. See [eng/testing.md](eng/testing.md) for test architecture and requirements.

## Verification

- Use the commands in `Justfile` and the dependencies documented in [README.md](README.md#build).
- Before handing off code, run `cargo fmt --check`, the relevant checks, and the relevant tests.
- Exercise changed CLI, image, and GUI behavior through the real entry point.
- Report failures, environment limits, and evidence scope.
