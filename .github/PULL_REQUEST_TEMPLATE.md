## What

Brief description of the change and why.

## Related issue

Fixes #<issue>, or N/A.

## Verification

Commands run (`just ci-pr`, `just clippy`, `just test`, `just parity-backend`, `just gui-raw`) and their results. Numerical changes need measured before/after evidence; GUI changes need screenshots or headless test output; LUT changes need the exported LUT and a rendered comparison.

- [ ] `cargo fmt --check` passes
- [ ] Relevant checks and tests pass locally
- [ ] Changed CLI, image, and GUI behavior exercised through the real entry point
