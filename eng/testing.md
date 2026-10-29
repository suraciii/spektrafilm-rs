# Test architecture

**Tests must catch meaningful failures, not serve as formalities; speed must never weaken assertions.**

**PR — `ci-pr`:** Check tracked Python syntax, bytecode hygiene, documentation links and presentation, and workspace formatting; run the pure math unit tests in f32 and f64. This required gate must not restore the full workspace cache, install native libraries, compile the GUI/renderer, download image fixtures, or run native, rendering, GPU, browser, or platform acceptance. It uses a separate math-only cache with debug information disabled. Main refreshes that cache for subsequent PRs. New checks must add no more than 30 seconds, including preparation, compilation, linking, and execution.

**Main — `ci`:** Run workspace type checks in both precisions, Clippy, and the full workspace suite, including rendering, GPU, GUI workers, benchmark, and statistical tests. These checks run after merge and do not block PR integration. A failure is handled as a separate integration repair. Moving tests out of PR must preserve regression coverage.

**Fixtures:** A case must reuse a prepared fixture that another case in the same binary already prepared when both need the same inputs. A case must not rebuild a population, bank, or calibration that already exists for its inputs.

Shared host fixtures may retain prepared populations, CPU banks, and parameter records. Initialize them before acquiring a device guard. Reuse the same input image for both sides of a parity comparison when the inputs are identical.

Parameterized device tests should acquire one backend outside the case loop unless device creation or replacement is the behavior under test. Reuse its pipeline cache, but keep mutable buffers, cancellation, and observation state local to each case. Do not share a global backend across tests.

**Main cost:** The full gate must stay within the `Main full verification` timeout in [.github/workflows/ci.yml](../.github/workflows/ci.yml). A change that adds rendering or preparation work reports the measured wall clock of the affected suites in the pull request. Test codegen uses `[profile.test] opt-level = 2` in [Cargo.toml](../Cargo.toml); it still inherits `dev`, so debug assertions and overflow checks stay enabled and only code generation differs.

**Long local gates:** Use `just ci-pr` as the first local gate. If `just ci` reaches the local shell timeout while a workspace test is still running, run `just test` and `just test-f64` as separate commands and report each result. A partial `just ci` run is not a full-gate pass.

**GPU host logic — `test-gpu-logic`:** Run the audited `logic_tests::` modules in `spektrafilm-gpu` in both precisions. These tests cover small host-side calculations, validation, and execution planning without creating a WGPU instance or device. `test-fast` includes this lane; `ci-pr` remains unchanged.

The recipe sets `SPEKTRAFILM_TEST_NO_DEVICE=1`. In GPU unit tests, this flag makes backend construction and device-lock acquisition fail before adapter enumeration or device work. A test that needs a device must stay outside `logic_tests::`; the flag must not turn device work into a skip or success.

This lane still compiles the GPU unit-test binary and its dependencies. It does not replace shader, device, numerical-parity, or full-render acceptance. `cargo test -p spektrafilm-gpu --lib`, `test`, and main CI retain the complete test set without ignored or feature-gated device cases.

Prefer device-free tests of production logic. Parameter validation, geometry, index ranges, resource budgets, execution plans, and small numerical boundaries should not require an adapter. Before adding a device test, check whether the regression belongs in an existing host calculation or a small private calculation shared with the production path.

Logic tests must use small deterministic in-memory inputs instead of unnecessary stock loading or environment setup. Inspect helper calls before placing a test in `logic_tests::`; the absence of device code in the test body is not enough. Test accepted and rejected boundaries, not incidental error wording, struct copies, or source-text markers.

Tests of source transformation should exercise the transformation with supplied input. Do not inspect checked-in shader text as a substitute for executing the shader's behavior.

Persistence tests must use real IO in isolated temporary directories. Shader numerics, actual bindings, queue synchronization, and device ownership require real execution. Keep those device assertions and full-render acceptance; a mock dispatch or a duplicate host implementation of a shader is not equivalent coverage.

The [Justfile](../Justfile) defines test selection. The [README](../README.md#build) documents dependencies and verification commands.
