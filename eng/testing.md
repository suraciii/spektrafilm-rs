# Test architecture

**Tests must catch meaningful failures, not serve as formalities; speed must never weaken assertions.**

**PR — `ci-pr`:** Check tracked Python syntax, bytecode hygiene, documentation links and presentation, and workspace formatting; run the pure math unit tests in f32 and f64. This required gate must not restore the full workspace cache, install native libraries, compile the GUI/renderer, download image fixtures, or run native, rendering, GPU, browser, or platform acceptance. It uses a separate math-only cache with debug information disabled. Main refreshes that cache for subsequent PRs. New checks must add no more than 30 seconds, including preparation, compilation, linking, and execution.

**Main — `ci`:** Run workspace type checks in both precisions, Clippy, and the full workspace suite, including rendering, GPU, GUI workers, benchmark, and statistical tests. These checks run after merge and do not block PR integration. A failure is handled as a separate integration repair. Moving tests out of PR must preserve regression coverage.

**Fixtures:** A case must reuse a prepared fixture that another case in the same binary already prepared when both need the same inputs. A case must not rebuild a population, bank, or calibration that already exists for its inputs.

Shared host fixtures may retain prepared populations, CPU banks, and parameter records. Initialize them before acquiring a device guard. Reuse the same input image for both sides of a parity comparison when the inputs are identical.

Parameterized device tests should acquire one backend outside the case loop unless device creation or replacement is the behavior under test. Reuse its pipeline cache, but keep mutable buffers, cancellation, and observation state local to each case. Do not share a global backend across tests.

Prepared device banks may be reused within one test when formation parameters and sampling footprints match. Keep them on the backend that prepared them. Keep image and readout buffers local to each case. Prepared-bank reuse must not replace cold production-path acceptance.

**Built-in look references:** Each shipped look needs a current entry in `crates/spektrafilm-gui/assets/preset-thumbnails.json`. After adding or changing a look or its material dependencies, run the existing real-CPU authoring operation and the normal completeness check:

```bash
cargo test --locked -p spektrafilm-gui --bin spektrafilm-gui \
  presets::thumbnails::tests::generate_fixed_reference_thumbnails -- --exact --ignored
cargo test --locked -p spektrafilm-gui --bin spektrafilm-gui \
  presets::thumbnails::tests::distributed_bundle_contains_current_real_render_for_every_builtin_look -- --exact
```

The authoring command uses the default-f32 configuration and writes the distributed bundle from actual CPU renders. Do not replace a missing reference with another look's image or edit its fingerprint by hand. The normal completeness check remains enabled.

**Main cost:** The full gate must stay within the `Main full verification` timeout in [.github/workflows/ci.yml](../.github/workflows/ci.yml). A change that adds rendering or preparation work reports the measured wall clock of the affected suites in the pull request. Test codegen uses `[profile.test] opt-level = 2` in [Cargo.toml](../Cargo.toml); it still inherits `dev`, so debug assertions and overflow checks stay enabled and only code generation differs.

**Long local gates:** Use `just ci-pr` as the first local gate. If `just ci` reaches the local shell timeout while a workspace test is still running, run `just test` and `just test-f64` as separate commands and report each result. A partial `just ci` run is not a full-gate pass.

**A test that takes minutes has no value.** A test that costs minutes is not run, and an assertion
nobody runs protects nothing. This narrows the rule above: speed may never weaken an assertion that
runs, but an assertion that cannot be run affordably is deleted rather than quarantined. Measure the
test before adding or keeping it, and delete it if the cost is intrinsic to what it does.

Three device tests were deleted under this rule on 2026-10-30, each for the same reason: the work
that dominated them was device formation, not the property under test.

| deleted test | measured cost | what it actually asserted |
|---|---|---|
| `gpu::…::grain::fotufilm::tests::fractional_geometry_and_paged_readout_match_the_cpu_bank` | 279 s | fractional geometry and paged/contiguous sample equality on a 4x4 source |
| `gpu::…::grain::fotufilm::tests::device_multi_sublayer_readout_matches_the_host_reference` | 283 s on amdgpu, 190 s CPU on the software adapter | device/host readout parity at three raster extents |
| `gui::…::headless::viewer::gpu_gui_scan_view_transition_and_save_keep_the_accepted_device_samples` | 166 s wall on 40 s CPU | GPU/CPU rotation parity, retained-carrier declarations, scan transition, save |

Coverage that survives them: paged/contiguous equality in the `paged` and `page_views` unit tests,
fractional geometry in the host-side `Bank::apply` tests in `spektrafilm-model`, and preview/scan/save
in `headless::scanner_ui_real_preview_scan_persistence_save_and_export`. Do not re-add a device test
that prepares a Fotufilm bank to the default suite: one bank is 51 level formations, and that is
minutes on every adapter.

**Local adapter:** When the workstation's amdgpu loses its context (`radv/amdgpu: The CS has been cancelled because the context is lost`), the GPU library aborts and every device test becomes a multi-second fence wait. Force the software adapter for a deterministic local run: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo test --locked -p spektrafilm-gpu`. CI already runs on llvmpipe, so this matches CI's adapter.

**GPU host logic — `test-gpu-logic`:** Run the audited `logic_tests::` modules in `spektrafilm-gpu` in both precisions. These tests cover small host-side calculations, validation, and execution planning without creating a WGPU instance or device. `test-fast` includes this lane; `ci-pr` remains unchanged.

The recipe sets `SPEKTRAFILM_TEST_NO_DEVICE=1`. In GPU unit tests, this flag makes backend construction and device-lock acquisition fail before adapter enumeration or device work. A test that needs a device must stay outside `logic_tests::`; the flag must not turn device work into a skip or success.

This lane still compiles the GPU unit-test binary and its dependencies. It does not replace shader, device, numerical-parity, or full-render acceptance. `cargo test -p spektrafilm-gpu --lib`, `test`, and main CI retain the complete test set without ignored or feature-gated device cases.

Prefer device-free tests of production logic. Parameter validation, geometry, index ranges, resource budgets, execution plans, and small numerical boundaries should not require an adapter. Before adding a device test, check whether the regression belongs in an existing host calculation or a small private calculation shared with the production path.

Logic tests must use small deterministic in-memory inputs instead of unnecessary stock loading or environment setup. Inspect helper calls before placing a test in `logic_tests::`; the absence of device code in the test body is not enough. Test accepted and rejected boundaries, not incidental error wording, struct copies, or source-text markers.

Tests of source transformation should exercise the transformation with supplied input. Do not inspect checked-in shader text as a substitute for executing the shader's behavior.

Numerical shader tests should execute the production pipeline assembly instead of copying its concatenation into a compilation-only test.

Persistence tests must use real IO in isolated temporary directories. Shader numerics, actual bindings, queue synchronization, and device ownership require real execution. Keep those device assertions and full-render acceptance; a mock dispatch or a duplicate host implementation of a shader is not equivalent coverage.

When asserting pixel invariance, compare decoded dimensions and samples rather than container bytes that also carry write-time metadata.

For successful CLI image publication, assert decoded output, replacement behavior, supported metadata, and staging-file cleanup. Do not assert incidental progress-message wording or require warning-free stderr. Valid inactive-setting feedback can accompany a successful write.

Built-in look changes must ship with regenerated reference thumbnails. Use the existing real-CPU asset producer, `cargo test --locked -p spektrafilm-gui --bin spektrafilm-gui presets::thumbnails::tests::generate_fixed_reference_thumbnails -- --ignored --exact`, then verify the distributed bundle. Do not update fingerprints without rendering their images.

The [Justfile](../Justfile) defines test selection. The [README](../README.md#build) documents dependencies and verification commands.

## CPU and GPU are allowed to disagree

The GPU path is a separate implementation. Same-algorithm arithmetic may differ
in the last bits. An intentionally different algorithm variant needs its own
documented image-quality budget.

- Same-algorithm parity tests compare with a tolerance (relative <= 1e-5, or <= 1 LSB at 16 bits).
  A bit-exact assertion between backends is a bug in the test.
- Structural invariants stay exact: which pixels and planes a kernel writes, clamp
  ranges, untouched borders. A skipped write is not a precision difference.
- Each run is deterministic on its own device: no atomics in accumulation, no
  reliance on unordered writes, snapshot before an in-place pass.
- A numerical oracle (for example `xtrans_reference`) is a debugging aid, not a
  contract. When a deviation appears it says which step moved.
- Acceptance is end to end: one real file through both paths, compared as an image
  (PSNR and maximum deviation). Different algorithm variants need independent
  reference evidence and an explicit regression budget.
- Measure GPU and CPU performance rather than assuming a speedup. Report the
  adapter, workload, and timing scope. Backend selection follows the product
  contract; a timing result does not authorize an undeclared CPU fallback.
- Because the backends may disagree, document that toggling the GPU can change the
  output. Otherwise it comes back as a bug report.
