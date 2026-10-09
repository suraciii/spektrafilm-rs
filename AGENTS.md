# Agent Instructions

## Project

SpektraFilm is a Rust spectral film simulation application with a native GUI, CLI, and LUT export.

- Read [README.md](README.md) for build, usage, and crate layout.
- `specs/` is the source of truth for target product behavior and design.
- `docs/` contains user-facing guidance; it does not define product contracts.

## Engineering

- Follow KISS and YAGNI. Reuse existing boundaries and dependencies.
- Keep each rule in one authoritative implementation.
- Preserve observable behavior, CPU f64 reference/export, WGPU f32 preview, and supported CPU fallback.
- Require measured evidence for numerical changes.

## Context

- Keep this file global and concise. Put subtree rules in scoped `AGENTS.md` files.
- Keep durable contracts in `specs/`, with one fact in one authoritative document.
- Keep plans, research, probes, logs, build artifacts, and session state outside the repository, normally under `/data/workspaces/spektrafilm-project-context/`.
- Put durable non-contract conclusions in the relevant Issue/PR. Never commit transient artifacts, even if ignored.

## Documentation language

- Write documentation in English.
- Use short sentences, active voice, American spelling, and stable terms.
- Use ASD-STE100 writing rules as a target; do not claim formal compliance.
- Keep domain terms, identifiers, field names, commands, and code symbols in their exact spelling.
- Use `must`, `may`, and `must not` for requirements, options, and prohibitions.
- State one rule per sentence.
- Write only statements that an implementation can be checked against. Delete generic motivation, repetition, and implementation tours.
- Give each fact one authoritative home. Define a term once and link to it.

## Verification

- Use the commands in `Justfile` and the dependencies documented in [README.md](README.md#build).
- Before handing off code, run `cargo fmt --check`, the relevant checks, and the relevant tests.
- Exercise changed CLI, image, and GUI behavior through the real entry point.
- Report failures and limits. Distinguish CPU, software WGPU, and hardware GPU evidence.
