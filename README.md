# spektrafilm-rs

![banner](docs/card.jpg)

SpektraFilm simulates analog color film and the print-and-scan chain spectrally. A digital image exposes a film stock, the developed record is enlarged onto photographic paper, and a scanner reads the result. Film, paper, enlarger light and scanner response are integrated over 81 wavelengths, so a stock, a paper, or a light source changes the image through its spectral profile rather than through a color preset. Bundled profiles carry measured or fitted spectral data; the CineStill 800T Cs41 material is an experimental fit, see [scanning](docs/scanning.md).

This is a Rust port of [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm). The desktop GUI renders at the build's native precision; GPU execution is f32 WGPU with faithful CPU stages where a GPU implementation would change the result. The separate `spektrafilm-f64` CLI remains the CPU f64 reference. Numeric agreement is judged against per-scenario budgets, not universal bit identity; see [parity and backends](docs/parity.md).

## What you can do

- **Simulate a film and print look.** Pick a film stock and a print paper, then adjust exposure, film format, halation, DIR couplers, grain, glare, enlarger and scanner settings. More than 30 bundled profiles cover Kodak, Fujifilm, CineStill and other stocks. See [the simulation pipeline](docs/pipeline.md).
- **Convert a negative.** Interpret a real negative or a simulated one as a positive with the calibrated inverse. Digital Reference is a modeled conversion for a simulated negative on `input > film > scan`. See [scanning](docs/scanning.md).
- **Preview and export interactively.** One session keeps Preview, full-resolution Scan, Save and the independent Export dialog together, with state save and restore. See [the GUI guide](docs/gui.md).
- **Process in batch.** `spektrafilm` is the fast f32 processor and `spektrafilm-f64` is the CPU reference exporter; both keep floating samples in float TIFF/EXR, copy source EXIF/IPTC/XMP to non-EXR output, and write color tags or ICC profiles. See [the CLI guide](docs/cli.md).
- **Choose a grain engine.** Spektra, Procedural (Dehancer-derived), Dye and Fotufilm Film are selectable, each with its own controls and defaults. See [grain engines](docs/grain.md).
- **Deliver LUTs for a camera.** Bake deterministic bundles for Panasonic V-Log with CUBE, 3DL, Hald, OCIO and QA artifacts. See [LUT delivery](docs/luts.md).
- **Compare versions.** `spektrafilm-bench` renders immutable evidence from color, tone, FLIP, grain, halation and edge measurements. Read retained measurements through `inspect`, then use `report` for an offline visual review. See [the appearance benchmark](docs/benchmark.md).
- **Decode camera RAW.** `decode_raw_gui` shares the native LibRaw and Lensfun path with the pipeline and writes linear ACES2065-1 TIFF. See [RAW decoding](docs/raw.md).

## Build

Rust stable (≥ 1.88), a C++17 compiler, `pkg-config`, and the OpenImageIO and Exiv2 development libraries are required. Linux links the installed OpenBLAS; macOS links Accelerate. [Building, packaging, and verification](docs/building.md) lists the per-platform commands, the RAW build dependencies (LibRaw ≥ 0.22.0, Lensfun, GLib), the package scripts and the `just` targets.

```bash
git clone <this-repo> && cd spektrafilm-rs

# desktop GUI
cargo build --release -p spektrafilm-gui

# f32 CLI, GPU by default
cargo build --release -p spektrafilm-cli

# CPU f64 reference exporter
cargo build --release --features precision-f64 -p spektrafilm-cli --bin spektrafilm-f64
```

`just install` installs the built programs and their data without administrator privileges. `just package` creates a self-contained directory and archive under `dist/`.

## Quick start

Render a film stock onto a print paper:

```bash
./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --backend cpu --film kodak_gold_200 --paper kodak_portra_endura --data-dir data
```

Start the desktop GUI, optionally on an image:

```bash
./target/release/spektrafilm-gui [optional/path/to/image.orf]
```

List the shipped films, papers and presets:

```bash
./target/release/spektrafilm list-profiles --data-dir data
./target/release/spektrafilm preset list --source all --format text
```

## Documentation

[specs/](specs/README.md) holds the product contracts: what the program must do and the invariants it protects. [CONTEXT.md](CONTEXT.md) defines the film-simulation language. The guides below explain how to use the product.

- [Building, packaging, and verification](docs/building.md) — dependencies, build commands, package scripts, install locations, and which gate runs when.
- [Simulation pipeline and profiles](docs/pipeline.md) — the spectral chain, selectable upsampling methods, camera filters, and shipped profiles.
- [GUI guide](docs/gui.md) — controls, Preview, Scan, Save, Export, states, and diagnostics.
- [CLI guide](docs/cli.md) — commands, formats, parameter overrides, geometry, color management, and reports.
- [Attribution](docs/attribution.md) — upstream sources, third-party components, and their license terms.

[docs/README.md](docs/README.md) carries the complete guide list. [Crate architecture](eng/architecture.md), [testing](eng/testing.md), [documentation language](eng/documentation-language.md), and [coding standards](CODING_STANDARDS.md) cover engineering practices.

## Project status

The migration target is the pinned Python 0.3.4 reference. Measured agreement and budgets live in [baseline parity evidence](docs/parity/baseline_evidence.md), the [parity matrix](docs/parity/parity_matrix.md), and the [Python ↔ Rust differential harness](scripts/parity/README.md).

## Credits and license

SpektraFilm is a Rust port of Andrea Volpato's Python [spektrafilm](https://github.com/andreavolpato/spektrafilm); the spectral data, the film and paper profiles and the pipeline architecture come from that project. [Attribution](docs/attribution.md) records the other third-party sources and their license terms.

The Rust program is [GPL-3.0](LICENSE). The bundled spectral profiles and their LUT derivatives keep their upstream CC BY-SA 4.0 terms. [Attribution](docs/attribution.md) states both and names the canonical license file.
