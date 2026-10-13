# Test architecture

**Tests must catch meaningful failures, not serve as formalities; speed must never weaken assertions.**

**PR — `ci-pr`:** Check tracked Python syntax, bytecode hygiene, and workspace formatting; run the pure math unit tests in f32 and f64. This required gate must not restore the full workspace cache, install native libraries, compile the GUI/renderer, download image fixtures, or run native, rendering, GPU, browser, or platform acceptance. It uses a separate math-only cache with debug information disabled. Main refreshes that cache for subsequent PRs. New checks must add no more than 30 seconds, including preparation, compilation, linking, and execution.

**Main — `ci`:** Run workspace type checks in both precisions, Clippy, and the full workspace suite, including rendering, GPU, GUI workers, benchmark, and statistical tests. These checks run after merge and do not block PR integration. A failure is handled as a separate integration repair. Moving tests out of PR must preserve regression coverage.

**Fixtures:** A case must reuse a prepared fixture that another case in the same binary already prepared when both need the same inputs. A case must not rebuild a population, bank, or calibration that already exists for its inputs.

**Main cost:** The full gate must stay within the `Main full verification` timeout in [.github/workflows/pr.yml](../.github/workflows/pr.yml). A change that adds rendering or preparation work reports the measured wall clock of the affected suites in the pull request.

**Long local gates:** Use `just ci-pr` as the first local gate. If `just ci` reaches the local shell timeout while a workspace test is still running, run `just test` and `just test-f64` as separate commands and report each result. A partial `just ci` run is not a full-gate pass.

Logic tests must use small deterministic in-memory inputs instead of unnecessary stock loading or environment setup. Persistence tests must use real IO in isolated temporary directories. Rendering and device tests must retain real execution.

The [Justfile](../Justfile) defines test selection. The [README](../README.md#build) documents dependencies and verification commands.
