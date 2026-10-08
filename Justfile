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

check:
    cargo check --locked --workspace

test:
    cargo test --locked --workspace --features precision-f64

ci: check test

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
