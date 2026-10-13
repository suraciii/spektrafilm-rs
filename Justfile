set dotenv-load := false

# Show the available project commands when no recipe is supplied.
default:
    @just --list


# Install the pinned official native Fotufilm compiler dependency.
install-halide prefix:
    bash scripts/install_halide.sh "{{prefix}}"

# Build the portable default (wgpu/WGSL) release binaries.
build: build-gui build-cli build-f64 decode-raw build-benchmark

build-gui:
    cargo build --locked --release -p spektrafilm-gui

build-cli:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm

build-f64:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm-f64 --features precision-f64

decode-raw:
    cargo build --locked --release -p spektrafilm-cli --bin decode_raw_gui

build-benchmark:
    cargo build --locked --release -p spektrafilm-benchmark --bin spektrafilm-bench --bin spektrafilm-bench-fixtures

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

# Run the workspace test suite with the default f32 configuration.
test:
    cargo test --locked --workspace

# Run the workspace test suite in the f64 reference configuration.
test-f64:
    cargo test --locked --workspace --features precision-f64

# Run measured deterministic logic and isolated file tests in both precisions.
test-fast: (test-fast-precision "") (test-fast-precision "--features precision-f64")

# Positive module selection keeps new rendering/device tests in main by default.
[private]
test-fast-precision precision_args:
    cargo test --locked -p spektrafilm-math --lib {{precision_args}}
    cargo test --locked -p spektrafilm-model --lib {{precision_args}} -- density_curves:: illuminants:: couplers:: glare:: grain::v1:: grain::fotufilm:: diffusion::
    cargo test --locked -p spektrafilm-core --lib {{precision_args}} -- params::sources:: params::tests:: params::validation:: params_builder:: pipeline_calibration::tests:: resizing:: scan_receiver:: suggest:: stages::scanning::setup:: stages::scanning::coupled:: stages::scanning::finish:: magazine_print_color:: input_gamut:: gamut_compression:: lut_formats:: lut_ocio:: lut_delivery:: --skip process_params::
    cargo test --locked -p spektrafilm-core --test telemetry {{precision_args}}
    cargo test --locked -p spektrafilm-gui --bin spektrafilm-gui {{precision_args}} -- state::tests:: numeric::tests::

# Run the native appearance benchmark regressions.
benchmark-test:
    cargo test --locked -p spektrafilm-benchmark

# Exercise the real GUI App and CPU workers without a desktop or OCR.
# Optimize the WASI interpreter so original preparation fits worker deadlines.
gui-headless:
    cargo test --release --locked -p spektrafilm-gui --features precision-f64 headless::

# Render focused GUI layouts offscreen; requires a usable WGPU adapter.
gui-layout:
    cargo test --locked -p spektrafilm-gui --features precision-f64,gui-snapshots layout_snapshots::

# Reject Python bytecode and cache directories in the Git index.
check-python-cache:
    python3 scripts/check_python_cache.py


# Compile tracked Python sources in memory without creating bytecode caches.
check-python:
    python3 -c 'import pathlib, subprocess; paths = subprocess.check_output(["git", "ls-files", "-z", "--", "*.py"]).split(b"\0"); [compile(pathlib.Path(path.decode()).read_bytes(), path.decode(), "exec") for path in paths if path]'

# Pure math regressions need no native libraries, GPU, or renderer.
test-pr:
    cargo test --locked -p spektrafilm-math --lib
    cargo test --locked -p spektrafilm-math --lib --features precision-f64
# Required PR gate; workspace compilation and native suites run after merge.
ci-pr: check-python-cache check-python fmt test-pr

# Full local and main-branch gate, including slow rendering and GPU tests.
ci: check-python-cache check-python fmt check check-f64 clippy test test-f64 benchmark-test
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
