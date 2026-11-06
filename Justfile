set dotenv-load := false

# Show the available project commands when no recipe is supplied.
default:
    @just --list

# Build the portable default (wgpu/WGSL) release binaries plus the lab instrument binary.
build: build-gui build-cli build-f64 build-lab build-bench decode-raw

build-gui:
    cargo build --locked --release -p spektrafilm-gui

build-cli:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm

build-f64:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm-f64 --features precision-f64

# Build the lab instrument binary. f64 reference precision; shipped in packages.
build-lab:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm-lab --features precision-f64

# Build the benchmark's offline visual-report binary. It owns the photo report
# command and shares no endpoint with the instrument.
build-bench:
    cargo build --locked --release -p spektrafilm-benchmark

decode-raw:
    cargo build --locked --release -p spektrafilm-cli --bin decode_raw_gui

# Check formatting without modifying files.
fmt:
    cargo fmt --all -- --check

# Type-check every workspace target with default features.
check:
    cargo check --locked --workspace --all-targets

# Type-check every workspace target with f64 precision.
check-f64:
    cargo check --locked --workspace --all-targets --features precision-f64

# Lint the workspace; any first-party warning fails. The vendored wgpu-hal patch
# under vendor/ is a local path dependency and keeps its own warnings.
clippy:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Lint the f64 reference configuration. A separate recipe rather than a feature
# flag on `clippy`, because `--features` changes which code compiles: lints that
# fire only under `precision-f64` are invisible to the f32 pass, and vice versa.
clippy-f64:
    cargo clippy --locked --workspace --all-targets --features precision-f64 -- -D warnings

# Run the workspace test suite with the default f32 configuration.
test:
    cargo test --locked --workspace

# Run the workspace test suite in the f64 reference configuration.
test-f64:
    cargo test --locked --workspace --features precision-f64

# Both suites under the thread and codegen limits the main CI gate sets. Local
# `test`/`test-fast` use every core and debug info, so their wall time says
# nothing about the 45-minute CI budget; run this to predict it.
test-ci:
    #!/usr/bin/env bash
    set -euo pipefail
    export OPENBLAS_NUM_THREADS=1 RAYON_NUM_THREADS=2
    export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
    just test
    just test-ci-tail

# Run audited GPU host logic in both precisions; device acquisition is an error.
test-gpu-logic: (test-gpu-logic-precision "") (test-gpu-logic-precision "--features precision-f64")

[private]
test-gpu-logic-precision precision_args:
    SPEKTRAFILM_TEST_NO_DEVICE=1 cargo test --locked -p spektrafilm-gpu --lib {{precision_args}} -- logic_tests::

# Run the full suite on the software Vulkan adapter. This matches CI's adapter
# and avoids the workstation amdgpu context loss (`radv/amdgpu: The CS has been
# cancelled because the context is lost`), which turns every device call into a
# multi-second fence wait and eventually aborts the run.
[unix]
test-software:
    VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo test --locked --workspace

# Run measured deterministic logic and isolated file tests in both precisions.
test-fast: (test-fast-group "") (test-fast-group "--features precision-f64")

# Full f32 workspace already covers the default fast selections and benchmark.
# Device-free acquisition policy is an additional contract in both precisions.
test-ci-tail: (test-gpu-logic-precision "") (test-fast-group "--features precision-f64")

[private]
test-fast-group precision_args: (test-gpu-logic-precision precision_args) (test-fast-precision precision_args)

# Positive module selection keeps new rendering/device tests in main by default.
[private]
test-fast-precision precision_args:
    cargo test --locked -p spektrafilm-math --lib {{precision_args}}
    cargo test --locked -p spektrafilm-model --lib {{precision_args}} -- density_curves:: illuminants:: couplers:: glare:: grain::spektra:: grain::fotufilm:: diffusion::
    cargo test --locked -p spektrafilm-core --lib {{precision_args}} -- params::sources:: params::tests:: params::validation:: params_builder:: pipeline_calibration::tests:: resizing:: suggest:: cmyk:: input_gamut:: gamut_compression:: lut_formats:: lut_ocio:: lut_delivery:: --skip process_params::
    cargo test --locked -p spektrafilm-core --test telemetry {{precision_args}}
    cargo test --locked -p spektrafilm-gui --bin spektrafilm-gui {{precision_args}} -- state::tests:: numeric::tests::

# Run the native appearance benchmark regressions.
benchmark-test:
    cargo test --locked -p spektrafilm-benchmark

# Measure CPU-reference vs GPU parity; fails on a budget breach or missing adapter.
parity-backend:
    cargo run --locked -p spektrafilm-core --example backend_parity --features precision-f64 -- data

# Exercise the real GUI App and CPU workers without a desktop or OCR.
gui-headless:
    cargo test --locked -p spektrafilm-gui --features precision-f64 headless::

# Exercise the real App RAW import, Preview/Scan and Export with the pinned
# Kodak DC50 fixture. The fixture is not redistributed; provide it explicitly.
# A missing or mismatched fixture fails the run instead of skipping it.
gui-raw:
    #!/usr/bin/env bash
    set -euo pipefail
    fixture="${SPEKTRAFILM_GUI_RAW_FIXTURE:?Set SPEKTRAFILM_GUI_RAW_FIXTURE to the pinned Kodak DC50 KDC file}"
    SPEKTRAFILM_GUI_RAW_FIXTURE="$fixture" \
        cargo test --locked -p spektrafilm-gui --features precision-f64,gui-raw headless::raw:: -- --skip xtrans_import_demosaics_and_renders_on_selected_gpu

# Import the pinned X-Trans fixture through the App with an explicit GPU.
gui-xtrans:
    #!/usr/bin/env bash
    set -euo pipefail
    fixture="${SPEKTRAFILM_GUI_XTRANS_FIXTURE:?Set SPEKTRAFILM_GUI_XTRANS_FIXTURE to the pinned _DSF9237.RAF file}"
    SPEKTRAFILM_GUI_XTRANS_FIXTURE="$fixture" \
        cargo test --locked -p spektrafilm-gui --features gui-raw xtrans_import_demosaics_and_renders_on_selected_gpu -- --nocapture

# Render focused GUI layouts and the full window offscreen; requires a usable WGPU adapter.
gui-layout:
    cargo test --locked -p spektrafilm-gui --features precision-f64,gui-snapshots layout_snapshots::

# Reject Python bytecode and cache directories in the Git index.
check-python-cache:
    python3 scripts/check_python_cache.py


# Compile tracked Python sources in memory without creating bytecode caches.
check-python:
    python3 -c 'import pathlib, subprocess; paths = subprocess.check_output(["git", "ls-files", "-z", "--", "*.py"]).split(b"\0"); [compile(pathlib.Path(path.decode()).read_bytes(), path.decode(), "exec") for path in paths if path]'

# Reject test code that builds a GPU device without holding the device lock.
check-device-lock: check-device-lock-guard check-device-lock-tests

# The guard itself. Its masker fails open when buggy, so the guard's own
# regression tests run first: a silently disabled check is worse than none.
check-device-lock-guard:
    python3 scripts/check_device_lock.py

check-device-lock-tests:
    python3 scripts/test_check_device_lock.py

# Reject broken documentation links, heading defects, and presentation violations.
check-specs: check-specs-tests
    python3 scripts/check_specs.py --strict

check-specs-tests:
    python3 scripts/test_check_specs.py

# Pure math regressions need no native libraries, GPU, or renderer.
test-pr:
    cargo test --locked -p spektrafilm-math --lib
    cargo test --locked -p spektrafilm-math --lib --features precision-f64
# Required PR gate; workspace compilation and native suites run after merge.
ci-pr: check-python-cache check-python check-device-lock check-specs fmt test-pr

# Full local and main-branch gate, including slow rendering and GPU tests.
ci: check-python-cache check-python check-device-lock check-specs fmt check clippy check-f64 clippy-f64 test test-ci-tail parity-backend

# Run the full gate and retain source provenance, output, and exit status.
verify evidence_dir:
    python3 scripts/verify.py {{quote(evidence_dir)}}

# Retain evidence for the fast PR gate using the same provenance checks.
verify-pr evidence_dir:
    python3 scripts/verify.py --gate ci-pr {{quote(evidence_dir)}}

# Create a self-contained portable directory and archive under dist/.
[unix]
package: build
    ./scripts/package_unix.sh

[windows]
package: build
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package_windows.ps1

# Install into a per-user location. Set SPEKTRAFILM_INSTALL_ROOT to override it.
[unix]
install: build
    ./scripts/install_unix.sh

[windows]
install: build
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/install_windows.ps1

# Build the native macOS .app bundle.
[macos]
package-macos-app:
    bash scripts/package_macos_app.sh

# Build the native Windows package with the existing AIO bundler.
[windows]
package-windows-aio:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package_windows_app.ps1 -IconSource assets/spektrafilm-icon.jpg -Aio
