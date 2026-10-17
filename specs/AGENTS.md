# Specification Instructions

## Structure

```text
specs/
├── VISION.md              # Product vision (optional)
├── AGENTS.md              # This file
└── {topic}/
    ├── VISION.md          # Feature vision (optional)
    ├── spec.md            # Required — product behavior
    ├── design.md          # Optional — boundaries, invariants
    └── {sub-topic}/       # Same shape, nested on demand
```

## Rules

- Follow [documentation language rules](../eng/documentation-language.md).
- Write durable target state only.
- Keep one fact in one authoritative document; link instead of duplicating rules.
- `spec.md` states product behavior at the appropriate level of abstraction. Use cases, GWT, and implementation checks belong in tests.
- Domain terms are defined in [CONTEXT.md](../CONTEXT.md). Product concepts are defined in the owning spec.
- Higher layers constrain lower layers. Never the reverse.
- Keep transient material outside the repository, under `/data/workspaces/spektrafilm-project-context/`.
- Put durable non-contract conclusions in the relevant Issue/PR.
- Do not use `docs/` as a competing product or design authority.
- **Authority** (optional) — declare the owning module and cross-capability dependencies at the top of a spec. Prevents spec/code drift.
- **Laws** (optional) — end a spec with numbered invariants that are directly testable. Complements Acceptance by stating what must always hold.

## Context Layers

- **Product Vision** ([VISION.md](VISION.md)) → why the product exists.
- **Domain Language** ([CONTEXT.md](../CONTEXT.md)) → what each term is.
- **Spec Documents** (`spec.md` / `design.md`) → what the product does.
- **Code** → the implementation.
