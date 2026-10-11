# Review standards

These checks apply when reviewing code and acceptance drivers. Build and verification commands remain in [README.md](README.md). Native acceptance setup and evidence formats remain in [scripts/parity/README.md](scripts/parity/README.md).

## Behavioral evidence

Trace the tested action to the behavior it is meant to exercise. Check that the observed result belongs to that action, input, and selected view. A previous report, unrelated repaint, or changed selection must not satisfy completion.

For an animation assertion, check that an immediate frame replacement cannot pass as a transition. For cancellation, check that the action occurs while work is in flight and that both the terminal outcome and destination-file state are observed.

When a false positive is plausible, require a negative case that would satisfy the weaker assertion. Prefer a deterministic regression for an isolated decision; use the native entry point to establish desktop behavior.

## Evidence scope

Compare the claimed scope with the selected scenarios and recorded source, executable, driver, and fixture identities. A partial run establishes only its named scenarios. A software compute adapter does not establish hardware GPU performance. Mocked platform input does not establish a native desktop pass.

## Check placement

Keep pull-request checks fast. New mandatory checks must have a measured incremental cost, including dependency preparation. Put desktop automation, real OCR, large fixture downloads, and full GUI acceptance in the separate native workflow. Changes to these paths still require relevant local or manually triggered evidence before delivery.

Enforce fixed syntax, trailer formats, forbidden artifacts, and deterministic selection rules with automated checks. Keep this document for decisions that require understanding the behavior and evidence.
