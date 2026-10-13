# Test architecture

**Tests must catch meaningful failures, not serve as formalities; speed must never weaken assertions.**

**PR — `ci-pr`:** Check the workspace and run fast tests. Fast tests must be quick, deterministic, isolated, safe to run in parallel, and independent of external environment setup. New checks must add no more than 30 seconds, including preparation, compilation, linking, and execution.

**Main — `ci`:** Run the full workspace suite, including rendering, GPU, GUI workers, and statistical tests. Moving tests out of PR must preserve regression coverage.

**Fixtures:** A case must reuse a prepared fixture that another case in the same binary already prepared when both need the same inputs. A case must not rebuild a population, bank, or calibration that already exists for its inputs.

**Main cost:** The full gate must stay within the `Main full verification` timeout in [.github/workflows/pr.yml](../.github/workflows/pr.yml). A change that adds rendering or preparation work reports the measured wall clock of the affected suites in the pull request.

Logic tests must use small deterministic in-memory inputs instead of unnecessary stock loading or environment setup. Persistence tests must use real IO in isolated temporary directories. Rendering and device tests must retain real execution.

The [Justfile](../Justfile) defines test selection. The [README](../README.md#build) documents dependencies and verification commands.

## Long local gates

Use `just ci-pr` as the first local gate. If `just ci` reaches the local shell timeout while a workspace test is still running, run `just test` and `just test-f64` as separate commands and report each result. A partial `just ci` run is not a full-gate pass.
