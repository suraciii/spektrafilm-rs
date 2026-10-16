# Error contract

## Contract

A caller must be able to tell what failed without parsing human prose, when the caller needs to act on the failure. A caller that only reports the failure must be able to print it unchanged.

Failure results must carry a stable category alongside the human message. The category must be an enum, not a string, so a caller branches on it exhaustively and a new variant breaks the build at the branch site.

The message must remain the authoritative human text. It must be printable verbatim and must not be reformatted by the type.

## Category stability

A category must be stable across releases. A caller may match on it, so renaming or removing a variant is a breaking change to that caller.

A category must not encode a call site. `InvalidParam` must mean the same thing wherever it appears. A category that identifies one function's failure is a call-site code, not a category, and must not be added.

A category must not encode recovery policy. Whether a failure is retryable, fatal, or publishable is the owning contract's decision, not the error type's. The type states what failed; the contract states what happens next.

## Reporting

A report must be able to emit a failure category without the message when the message contains user data. Default telemetry reports already exclude absolute paths, filenames, and parameter snapshots; a category makes that exclusion possible for the error itself.

A failure record in a report must use the same category vocabulary as the in-process type. A second string code table must not appear.

## Acceptance

Every existing failure message must remain byte-identical after the migration. The workspace test suite must pass with its error assertions unchanged.

Every boundary function that returns `Result<T, String>` today must return the typed result after the migration. No boundary may return a bare `String` failure.

A caller that branches on a category must compile exhaustively. Adding a variant must produce a compile error at each branch that does not handle it, which is the property the migration buys.

A device-free build must be able to name the error type. The type must not require a device toolkit.

## Precondition

The migration must not start until a consumer that branches on a category exists. No consumer branches on one today: the CLI prints and exits, the GUI prints the message, and the benchmark owns its own typed error. Starting without that consumer would produce a category set with no exhaustive branch to protect, which is speculation rather than a contract.

The compute backend boundary migration is the prerequisite, because it decides which crate owns the error type. The error type must move with the abstraction, not stay behind in a crate that domain code must not depend on.
