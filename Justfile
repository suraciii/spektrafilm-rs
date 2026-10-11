set dotenv-load := false

# Show the available project commands when no recipe is supplied.
default:
    @just --list

# Build the portable default (wgpu/WGSL) release binaries.
build: build-gui build-cli build-f64 decode-raw

build-gui:
    cargo build --locked --release -p spektrafilm-gui

build-cli:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm

build-f64:
    cargo build --locked --release -p spektrafilm-cli --bin spektrafilm-f64 --features precision-f64

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

# Run the workspace test suite in the f64 reference configuration.
test:
    cargo test --locked --workspace --features precision-f64

# Reject Python bytecode and cache directories in the Git index.
check-python-cache:
    python3 scripts/check_python_cache.py

# Python syntax and clear-error lint checks (pip install -r scripts/requirements-check.txt).
check-python:
    python3 -B scripts/check_python.py
    python3 -m ruff check --no-cache scripts tests

# Offline OCR/geometry and animation regressions; only Python stdlib is required.
test-gui-driver:
    python3 -B -m unittest discover -s tests -p 'test_*.py' -v

# Full local gate. PR CI runs this same command.
ci: check-python-cache check-python test-gui-driver fmt check check-f64 clippy test

# Run the full gate and retain source provenance, output, and exit status.
verify evidence_dir:
    python3 scripts/verify.py {{quote(evidence_dir)}}

# Build all native acceptance binaries and bind them to source identity.
build-evidence evidence_dir profile="dev":
    python3 -B scripts/build_evidence.py --out-dir {{quote(evidence_dir)}} --profile {{quote(profile)}}

# Run a real desktop scenario using a build-evidence manifest (default: full).
# Linux requires Xvfb, a session D-Bus, GTK portal, and native GUI dependencies.
test-gui build_manifest evidence_dir scenario="full":
    python3 -B scripts/parity/gui_acceptance.py --build-manifest {{quote(build_manifest)}} --evidence-dir {{quote(evidence_dir)}} --scenario {{quote(scenario)}}

# Create a self-contained portable directory and archive under dist/.
[unix]
package:
    ./scripts/package_unix.sh

[windows]
package:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package_windows.ps1

# Install into a per-user location. Set SPEKTRAFILM_INSTALL_ROOT to override it.
[unix]
install:
    ./scripts/install_unix.sh

[windows]
install:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/install_windows.ps1

# Build the native macOS .app bundle.
[macos]
package-macos-app:
    bash scripts/package_macos_app.sh

# Build the native Windows package with the existing AIO bundler.
[windows]
package-windows-aio:
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/package_windows_app.ps1 -IconSource assets/spektrafilm-icon.jpg -Aio
