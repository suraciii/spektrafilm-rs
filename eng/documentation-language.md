# Documentation language

These rules govern durable documentation in `specs/`, `docs/`, and `eng/`.

## Language

- Write active prose in English.
- Use short sentences, active voice, American spelling, and stable terms.
- Use ASD-STE100 writing rules as a target; do not claim formal compliance.
- Keep domain terms, identifiers, field names, commands, serialized values, and code symbols in their exact spelling.
- In `specs/`, use uppercase `MUST`, `MUST NOT`, `MAY`, `SHOULD`, and `SHOULD NOT` for requirements, options, prohibitions, and recommendations.
- Use lowercase `must`, `may`, and `should` in `docs/` and `eng/`.
- State one rule per sentence.
- Write only statements that an implementation can be checked against. Delete generic motivation, repetition, and implementation tours.
- Give each fact one authoritative home. Define a term once and link to it.

## Structure

- State each normative rule once in the form that makes it clearest. Use numbered steps for a linear procedure.
- Product specifications state user-visible behavior. Design specifications state boundaries, invariants, and the trade-offs they protect.
- Describe the target state. Record current implementation gaps in the owning document instead of weakening the target contract.
- Explain product and domain concepts before implementation details. Do not turn classes, methods, call chains, or storage steps into prose.

## Examples and diagrams

- Commands and executable examples must run or parse as written. Keep only examples that remove ambiguity.
- Draw a diagram only when a boundary, ownership relation, dependency, sequence, hierarchy, or state transition is clearer as a picture.
- Store diagrams as rendered text. Use adjacent prose only for constraints the diagram does not express.
- Do not use raw HTML. Use Markdown, short prose, or one concrete example.
