# Test architecture

**Tests must catch meaningful failures, not serve as formalities; speed must never weaken assertions.**

**PR — `ci-pr`:** Check the workspace and run fast tests. Fast tests must be quick, deterministic, isolated, safe to run in parallel, and independent of external environment setup. New checks must add no more than 30 seconds, including preparation, compilation, linking, and execution.

**Main — `ci`:** Run the full workspace suite, including rendering, GPU, GUI workers, and statistical tests. Moving tests out of PR must preserve regression coverage.

Logic tests must use small deterministic in-memory inputs instead of unnecessary stock loading or environment setup. Persistence tests must use real IO in isolated temporary directories. Rendering and device tests must retain real execution.

The [Justfile](../Justfile) defines test selection. The [README](../README.md#build) documents dependencies and verification commands.
