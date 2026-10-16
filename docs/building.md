# Building, packaging, and verification

This page describes native build requirements, build commands, packaging, and per-user installation.

## Requirements

The build requires Rust stable (≥ 1.88), a C++17 compiler, pkg-config, OpenImageIO and Exiv2 development libraries. The locked `image` dependency requires Rust 1.88. Image I/O uses the same native libraries as Python 0.3.4. It preserves float samples and EXIF/IPTC/XMP.

On Debian/Ubuntu, install `libopenimageio-dev libimath-dev libexiv2-dev libopenblas-dev`. Linux links the installed OpenBLAS library. On macOS, use `brew install openimageio exiv2 pkg-config`. Windows packaging uses a matching MSYS2 UCRT64 native toolchain and pkg-config dependencies. The package scripts collect native runtime libraries. The build fails explicitly when required development libraries are absent.

RAW decoding requires native **LibRaw ≥ 0.22.0**, **Lensfun**, **Exiv2**, and **GLib** development packages plus a C++17 compiler and `pkg-config`. On Debian/Ubuntu, install `libraw-dev liblensfun-dev libexiv2-dev libglib2.0-dev pkg-config`. If the distribution supplies LibRaw 0.21, build the official 0.22.0 release into a local prefix. Prepend its `lib/pkgconfig` to `PKG_CONFIG_PATH` and its `lib` to the runtime library search path. On macOS, use `brew install libraw lensfun exiv2 glib pkg-config`. Windows builds need a matching native toolchain and these libraries built for it, for example through MSYS2 UCRT64. Set `PKG_CONFIG_PATH` to their `.pc` directories. Cross compilation requires target-specific pkg-config paths and libraries. Missing dependencies or older LibRaw fail the build explicitly.

Image IO additionally requires OpenImageIO and Imath development headers. OIIO installations that forward to external fmt headers also need the fmt development package and `fmt.pc` on `PKG_CONFIG_PATH`. Homebrew supplies this dependency. Windows builds LibRaw 0.22.0 with `scripts/build_windows_libraw.sh`. Packages include the DLLs and licenses alongside system native dependencies.

## Build commands

```bash
git clone <this-repo> && cd spektrafilm-rs

# GUI (wgpu/Metal preview, eframe)
cargo build --release -p spektrafilm-gui

# f32 CLI — fast batch processor, defaults to WGPU
cargo build --release -p spektrafilm-cli

# f64 CLI — reference precision (CPU only; WGSL has no f64)
cargo build --release --features precision-f64 -p spektrafilm-cli --bin spektrafilm-f64

# instrument CLI — the research command surface at f64 reference precision
cargo build --release --features precision-f64 -p spektrafilm-cli --bin spektrafilm-lab

# Optional RAW decoding helper using the shared native decoder
cargo build --release -p spektrafilm-cli --bin decode_raw_gui
```

This produces:

- `target/release/spektrafilm-gui` — desktop GUI.
- `target/release/spektrafilm` — fast f32 CLI.
- `target/release/spektrafilm-f64` — CPU reference exporter.
- `target/release/spektrafilm-lab` — the instrument command surface (f64 reference precision; shipped in packages).
- `target/release/decode_raw_gui` — standalone RAW decoding helper.

The `just` command surface also covers packaging and per-user installation:

```bash
just build
just check
just test
just test-f64
just test-fast
just ci-pr
just ci
just package
just install
```

## Packaging and installation

Packages must distribute the native shared libraries and their transitive dependencies with the CLI, GUI, `decode_raw_gui`, and the instrument binary `spektrafilm-lab`. They must also include the Lensfun XML database. Preserve Lensfun's default database lookup location or deploy it in the platform's standard data directory.

LibRaw is dual LGPL 2.1/CDDL 1.0. Lensfun's library is LGPL 3.0, and its database is CC BY-SA 3.0. Exiv2 is GPL 2.0 or later. GLib is LGPL 2.1 or later. Packages must include applicable license texts. The application remains GPL 3.0.

On Linux x64, install `patchelf libxkbcommon-x11-0` and run `scripts/package_linux_app.sh` after `just build`. This builds the CLI, f64 exporter, GUI, RAW helper, and the instrument. `BUILD_TARGET_DIR` selects Cargo's output directory. `DIST_DIR` selects the generated package directory. The script copies recursive ELF dependencies, LittleCMS, the GUI's dynamically loaded xkbcommon providers, profiles/ICC files, Lensfun XML, and native license notices. It verifies loader resolution and relative `$ORIGIN` library paths before creating a `.tar.gz`.

Extract the entire archive and launch the wrappers in `bin/`. They locate bundled data and libraries relative to their own location. The host supplies glibc, the display server, and graphics drivers. Windows and macOS use `scripts/package_windows_app.ps1` and `scripts/package_macos_app.sh`.

`just package` creates a self-contained directory and archive under `dist/`. `just install` installs without administrator privileges. On Unix-like systems, it uses `$XDG_DATA_HOME/spektrafilm`, with `~/.local/share/spektrafilm` as the default. macOS uses `~/Library/Application Support/Spektrafilm`. It creates command links under `~/.local/bin`. On Windows, it installs under `%LOCALAPPDATA%\Spektrafilm`. Set `SPEKTRAFILM_INSTALL_ROOT` to override the application directory. The Windows install directory is not added to `PATH` automatically.

`just package-macos-app` creates the native macOS `.app` bundle. `just package-windows-aio` creates the existing Windows self-contained AIO executable.

## Verification gates

The default and integration branch is `main-0.3.4`; pull requests require the `PR lightweight` check, which runs `just ci-pr` through `just verify-pr`. Pushes to `main-0.3.4` and manual dispatch run the `Main CI` workflow: the `Main full verification` job runs the full gate through `just verify`, and the three-platform `Build packages` job then produces nightly packages as workflow artifacts. Version tags and manual dispatch run the `Release` workflow: a gate job verifies that the tag sits on `main-0.3.4` and matches the workspace version, and the shared three-platform build attaches the packages to the GitHub Release.

## Package and GUI verification

See [scripts/parity/README.md](../scripts/parity/README.md) for release package smoke, headless GUI checks, and native GUI acceptance.

## See also

- [Documentation index](README.md)
- [CLI usage](cli.md)
- [GUI usage](gui.md)
- [RAW decoding](raw.md)
- [Appearance benchmark](benchmark.md)
- [Parity and acceptance evidence](parity.md)
- [Test architecture](../eng/testing.md)
- [Native package and GUI acceptance tools](../scripts/parity/README.md)
