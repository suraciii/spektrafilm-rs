# Error contract

## Contract

A caller MUST be able to tell what failed without parsing human prose, when the caller needs to act on the failure. A caller that only reports the failure MUST be able to print it unchanged.

Failure results MUST carry a stable category alongside the human message. The category MUST be an enum, not a string, so a caller branches on it exhaustively and a new variant breaks the build at the branch site.

The message MUST remain the authoritative human text. It MUST be printable verbatim and MUST NOT be reformatted by the type.

## Category stability

A category MUST be stable across releases. A caller MAY match on it, so renaming or removing a variant is a breaking change to that caller.

A category MUST NOT encode a call site. `InvalidParam` MUST mean the same thing wherever it appears. A category that identifies one function's failure is a call-site code, not a category, and MUST NOT be added.

A category MUST NOT encode recovery policy. Whether a failure is retryable, fatal, or publishable is the owning contract's decision, not the error type's. The type states what failed; the contract states what happens next.

## Reporting

A report MUST be able to emit a failure category without the message when the message contains user data. Default telemetry reports already exclude absolute paths, filenames, and parameter snapshots; a category makes that exclusion possible for the error itself.

A failure record in a report MUST use the same category vocabulary as the in-process type. A second string code table MUST NOT appear.

## Acceptance

Every existing failure message MUST remain byte-identical after the migration. The workspace test suite MUST pass with its error assertions unchanged.

Every boundary function that returns `Result<T, String>` today MUST return the typed result after the migration. No boundary MAY return a bare `String` failure.

A caller that branches on a category MUST compile exhaustively. Adding a variant MUST produce a compile error at each branch that does not handle it, which is the property the migration buys.

A device-free build MUST be able to name the error type. The type MUST NOT require a device toolkit.

## Precondition

The migration MUST NOT start until a consumer that branches on a category exists. No consumer branches on one today: the CLI prints and exits, the GUI prints the message, and the benchmark owns its own typed error. Starting without that consumer would produce a category set with no exhaustive branch to protect, which is speculation rather than a contract.

The compute backend boundary migration is the prerequisite, because it decides which crate owns the error type. The error type MUST move with the abstraction, not stay behind in a crate that domain code MUST NOT depend on.
