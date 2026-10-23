# Error contract design

## Ownership

The [product contract](spec.md) states what a caller can rely on when an operation fails. This document states where error types live, what they carry, and what they must not become.

The error contract must not introduce a second error vocabulary. Exactly one typed error may cross a crate boundary as a failure result. Internal helper functions may keep local error types, but a boundary function must convert before returning.

[Compute backend boundary](compute-backend/design.md) owns where the backend abstraction lives. The typed error must move with the abstraction, because the trait's method signatures return it. It must not stay behind in a crate that domain code must not depend on.

The coupling is deeper than the trait's return types. `CpuStageFn` is a type alias for a closure returning `Result<ImageBuf, String>`, and `FilmChainParams` holds five `Option<CpuStageFn>` fence fields. `spektrafilm-core` keeps its own copy of the alias. Changing the error type therefore changes the alias, the parameter record's field types, and every closure that fills them. A migration plan that only rewrites `-> Result<T, String>` signatures will not compile.

## Current shape

The workspace reports failure as `Result<T, String>` at 312 sites: 255 in core, 35 in model, 22 in the device crate. No typed error exists anywhere in the workspace. An earlier `SpektraError` definition in `spektrafilm-core/src/error.rs` was removed on 2026-10-16 because no call site used it and the precondition below had not been met; it was speculation rather than a contract.

A `String` failure carries one field: human text. A caller can print it and can substring-match it. A caller cannot branch on it without parsing prose.

## What the typed error must preserve

The migration must not change any observable failure behavior.

Error message text must stay byte-identical. The typed error must forward its message verbatim through `Display`, so an existing assertion such as `assert!(err.contains("scanner.lamp"))` must keep passing without edit. Any migration step that changes a message string is a defect, not a cleanup.

Exit codes must not change. CLI failure outcomes must stay the same for the same input. A GUI dialog must show the same text.

`From` conversions must exist for every error type a boundary function already propagates, so `?` keeps working. A conversion that drops or rewrites the message is prohibited.

## What the typed error must not do

The migration must not add error handling that no caller requests. A code variant with no consumer is speculation. Each variant must exist because some caller branches on it or because a report field requires it.

The migration must not turn a `String` failure into a structured error whose fields duplicate the message. If a caller only ever prints the error, the message alone is sufficient and the extra structure is dead weight.

The migration must not change a recoverable failure into a fatal one, or the reverse. Cancellation, publication, and retry behavior must stay as specified by the owning contract.

## Consumer-driven classification

A code variant must be justified by a named consumer that branches on it. The candidate consumers are:

| Consumer | Branch it needs | Status |
|----------|-----------------|--------|
| CLI | exit-code selection | none observed; CLI prints and exits |
| GUI | dialog class or recovery affordance | none observed; GUI prints the message |
| benchmark report | stable error category in JSON | present, but benchmark owns its own typed error already |
| tests | assertion subject | present; 120 sites assert message text |

Only a consumer that appears in this table with a real branch justifies a variant. Until one does, the code set stays minimal and the message stays the contract.

The benchmark report is the one consumer with an established pattern: it already carries a typed error with a code, a message, an optional path, and an optional boundary, and it serializes all of them. That pattern is the precedent to follow if and when a report field requires a category.

## Risks

312 sites is the largest single migration in the workspace history. The comparable completed refactor moved about 2200 lines with no signature change; this one changes a return type at 70 public function signatures in core.

The risk is not the mechanical edit. It is a message drift introduced by hand at one of 243 distinct message templates, which breaks one of 120 assertions that no compiler check covers.

A staged migration must therefore keep the tree compiling and the suite green at every step. A step that requires a message edit must be rejected.
