# spektrafilm-rs

![banner](docs/card.jpg)

A Rust port of [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm) — a spectral simulator for analogue colour film and the print-and-scan chain.

The spectral chain (RGB → film dye density → enlarger illuminant → print paper → scanner RGB) is migrated against pinned Python spektrafilm 0.3.4 commit `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`. CPU export uses f64; GPU preview uses f32 with explicit CPU routing for effects without a faithful GPU implementation. Numeric agreement is assessed with per-scenario budgets, rather than universal bit identity.

---

## What it does

- **Spectral pipeline.** Hanatos2025 RGB→raw spectral upsampling with its full sensitivity adaptation (camera UV/IR band-pass filters with reference-illuminant normalization, erf4 band-pass window, poly4 log-exposure surface, spectral Gaussian blur), full 81-wavelength film/print/scanner spectral integration, density-curve interpolation, halation, DIR couplers, grain (bit-exact numpy `MT19937` port), glare, output CCTF encoding.
- **Interactive preview** through wgpu (Metal on macOS), with CPU stages for exact optical diffusion and grain sampling. Frame rate depends on image size, controls and hardware.
- **Reference export** on the CPU at f64. Historical bare-chain evidence and applicable comparison budgets are recorded in [baseline evidence](docs/parity/baseline_evidence.md); fresh integrated comparisons are required for migration acceptance.
- **Decoupled preview + export.** GUI uses f32 GPU for iteration, then shells out to the f64 CPU binary for the final write. The export runs in a worker thread with a cancel button and proper child-process lifecycle.
- **Profiles bundled.** 30+ film and paper profiles in `data/profiles/` — Kodak Gold/Portra/Ektar, Fuji Velvia/Provia, Kodak Endura papers, Fuji Crystal Archive papers.

## Build

Requires Rust stable (≥ 1.88), a C++17 compiler, pkg-config, OpenImageIO and Exiv2 development libraries. The locked `image` dependency requires Rust 1.88. Image I/O uses the same native libraries as Python 0.3.4, preserving float samples and EXIF/IPTC/XMP. On Debian/Ubuntu install `libopenimageio-dev libexiv2-dev libopenblas-dev`; Linux links the installed OpenBLAS library. On macOS use `brew install openimageio exiv2 pkg-config`. Windows packaging uses a matching MSYS2 UCRT64 native toolchain and pkg-config dependencies. The package scripts collect native runtime libraries; the build fails explicitly when required development libraries are absent.

```bash
git clone <this-repo> && cd spektrafilm-rs

# GUI (wgpu/Metal preview, eframe)
cargo build --release -p spektrafilm-gui

# f32 CLI — fast batch processor, defaults to GPU backend
cargo build --release -p spektrafilm-cli

# f32 CLI with experimental native CUDA backend option
cargo build --release -p spektrafilm-cli --features spektrafilm-gpu/cuda-backend

# f64 CLI — reference precision (CPU only; WGSL has no f64)
cargo build --release --features precision-f64 -p spektrafilm-cli --bin spektrafilm-f64

# Helper used by the GUI's Export button — bit-identical RAW decode
cargo build --release -p spektrafilm-cli --bin decode_raw_gui
```

This produces:

- `target/release/spektrafilm-gui` — desktop GUI;
- `target/release/spektrafilm` — fast f32 CLI;
- `target/release/spektrafilm-f64` — CPU reference exporter;
- `target/release/decode_raw_gui` — RAW decoder used by GUI export.

RAW decoding requires native **LibRaw ≥ 0.22.0**, **Lensfun**, **Exiv2**, and **GLib** development packages plus a C++17 compiler and `pkg-config`. On Debian/Ubuntu install `libraw-dev liblensfun-dev libexiv2-dev libglib2.0-dev pkg-config`; if the distribution supplies LibRaw 0.21, build the official 0.22.0 release into a local prefix and prepend its `lib/pkgconfig` to `PKG_CONFIG_PATH` (and its `lib` to the runtime library search path). On macOS use `brew install libraw lensfun exiv2 glib pkg-config`. Windows builds need a matching native toolchain and these libraries built for it (for example through MSYS2 UCRT64); set `PKG_CONFIG_PATH` to their `.pc` directories. Cross compilation requires target-specific pkg-config paths and libraries. Missing dependencies or older LibRaw fail the build explicitly; measured Kodak DC50 pixels differ on LibRaw 0.21.5, while 0.22.0 matches the pinned rawpy decoder exactly.

The RAW build links the LibRaw artifact selected by `pkg-config` through a unique build-only name, preserving that selection even when another native dependency adds system library directories first. Packages distribute the library's actual runtime SONAME (for LibRaw 0.22.0 on Linux, `libraw.so.24`), rather than the build-only name.
Image IO additionally requires OpenImageIO and Imath development headers. OIIO installations that forward to external fmt headers also need fmt development headers and `fmt.pc` on `PKG_CONFIG_PATH`; Homebrew supplies this dependency. Windows CI builds checksum-pinned LibRaw 0.22.0 with `scripts/build_windows_libraw.sh` into a separate UCRT64 prefix and packages its DLLs and licenses alongside system native dependencies.


Distribute the native shared libraries and their transitive dependencies with the CLI, GUI, and `decode_raw_gui`, together with the Lensfun XML database. Preserve Lensfun's default database lookup location or deploy it in the platform's standard data directory. LibRaw is dual LGPL 2.1/CDDL 1.0, Lensfun's library is LGPL 3.0 (database CC BY-SA 3.0), Exiv2 is GPL 2.0 or later, and GLib is LGPL 2.1 or later; include applicable license texts when packaging. The application remains GPL 3.0.

On Linux x64, install `patchelf libxkbcommon-x11-0` and run `scripts/package_linux_app.sh` after building the release CLI, f64 exporter, GUI and RAW helper. `BUILD_TARGET_DIR` selects Cargo's output directory and `DIST_DIR` selects the generated package directory. The script copies the actual recursive ELF dependencies, the GUI's dynamically loaded xkbcommon providers, profiles/ICC files, Lensfun XML and native license notices; it verifies loader resolution and relative `$ORIGIN` library paths before creating a `.tar.gz`. Extract the entire archive and launch the wrappers in `bin/`; they locate bundled data and libraries relative to their own location. The host supplies glibc, the display server and graphics drivers. Windows and macOS use `scripts/package_windows_app.ps1` and `scripts/package_macos_app.sh` respectively.

The release workflow runs workspace tests with `precision-f64` and `scripts/parity/package_smoke.py` on each platform before uploading the package. The smoke runner requires installed CLI, f64 exporter, RAW helper and data paths. It exercises generated float/integer files, retained EXIF/IPTC/XMP/ICC, invalid-input/depth errors, a SHA-verified Kodak RAW fixture, and delivered ZIP LUT/OCIO/QA artifacts. With `--gui`, the external native driver operates the real window and file dialogs, verifies state restoration across restart, compares input/output rasters, saves float output, observes the bundled f64 child, and checks Cancel/close cleanup. Linux uses Xvfb; Windows/macOS require an interactive desktop, and macOS requires Accessibility/Automation/Screen Recording authorization. Driver and OCR dependencies are listed in [scripts/parity/README.md](scripts/parity/README.md). CI retains screenshots, observations and command logs, including failed runs.

`spektrafilm-raw` returns linear ACES2065-1 pixels after LibRaw camera orientation, optional Lensfun correction, and white balance. Camera WB (`as-shot`) is the default; `daylight` uses LibRaw's daylight base, while `tungsten` and `custom` adapt that base using the reference CIE D/Kang 2002 whitepoints and Von Kries/CAT02 transform. Custom temperature must be finite in 1667–25000 K and custom tint finite; absent custom temperature is an error. Lensfun applies vignetting before combined channel geometry resampling with bilinear interpolation and nearest boundary extension, and preserves unchanged pixels/empty lens summary when camera or lens matching fails.

The shared helper accepts `decode_raw_gui input.CR2 output.tif --raw-white-balance custom --raw-temperature 5000 --raw-tint 1.05 --lens-correction` and writes **linear ACES2065-1 float TIFF**, suitable for pipeline input configured as ACES2065-1 with input CCTF decoding disabled. It no longer writes linear sRGB through rawler.

Real RAW provenance is retained in [`scripts/parity/raw_fixtures.json`](scripts/parity/raw_fixtures.json); downloaded images are not redistributed. Run `scripts/parity/raw_reference_compare.py input.KDC --decoder target/release/decode_raw_gui --source-url URL --max-error 0 --mean-error 0` in the pinned Python environment for fresh four-mode decoder/WB evidence. The exercised Kodak DC50 KDC decodes to 768×512 and matches LibRaw 0.22.0/rawpy 0.26.1 at zero max/mean error for all four WB modes. Its EXIF has no lens model, exercising unchanged no-match correction; a separately injected Canon EOS 5D Mark II / EF 24–70mm f/2.8L / 35mm f/8 EXIF fixture matches upstream Lensfun pixels and summary exactly at 64×48 and 640×480. A public `M0054341_01_00005.cr2` fixture is intrinsically damaged (verified Git blob), and both paths reject it with a data error. These measurements do not imply support for every recognised RAW suffix.

The public Canon EOS 40D sRAW CR2 fixture also matches all four modes at zero max/mean error on 1944×1296 pixels, with missing-lens correction enabled. Unlike Kodak's unit camera multipliers, this camera's recorded WB `[2341,1024,1570,1024]` exercises distinct camera and daylight output (max change 0.212146, mean change 0.007560). Run `scripts/parity/lens_reference_compare.py` for the known-lens, missing-metadata, missing-camera and missing-lens comparisons; it uses actual Exiv2 metadata and native Lensfun, requires all dependencies, and defaults to exact pixel/summary equality.

The rebuilt f64 CLI and its relocated Linux archive also passed eight fresh RAW comparisons against the pinned Python loader: all four white-balance modes on Kodak 768×512 with missing-lens correction, and Canon 1944×1296 with injected known-lens EXIF. The unbounded `rgb_in` boundary matched exactly (maximum and mean absolute error zero). The relocated RAW helper independently matched Kodak pixels exactly under a clean environment; the packaged GUI launched under Xvfb and prepared-image export preserved EXIF/IPTC/XMP and ICC bytes.

WGSL is the default GPU backend and can be selected explicitly with `SPEKTRAFILM_BACKEND=wgpu`. An experimental native CUDA backend can be built with `--features spektrafilm-gpu/cuda-backend` and selected with `SPEKTRAFILM_BACKEND=cuda`. It uses CUDA 12 driver/NVRTC bindings through dynamic loading, so the NVIDIA driver and NVRTC runtime DLLs must be available. Set `SPEKTRAFILM_CUDA_DEVICE=1` (or another zero-based index) to pick a non-default CUDA device. Both GPU paths have a resident preview implementation: front pass, highlight boost, camera diffusion, camera lens blur, halation, DIR couplers, grain, density curves, enlarger diffusion, print/scan spectral reductions, glare, output gamut compression, scanner lens blur, unsharp, and one readback.
The `just` command surface also covers packaging and per-user installation:

```bash
just build
just check
just test
just ci
just package
just install
```

`just package` creates a self-contained directory and archive under `dist/`.
`just install` installs without administrator privileges. On Unix-like systems
it uses `$XDG_DATA_HOME/spektrafilm` (default:
`~/.local/share/spektrafilm`; macOS uses
`~/Library/Application Support/Spektrafilm`) and creates command links under
`~/.local/bin`. On Windows it installs under
`%LOCALAPPDATA%\Spektrafilm`; set `SPEKTRAFILM_INSTALL_ROOT` to override the
application directory. The Windows install directory is not added to `PATH`
automatically.

`just package-macos-app` creates the native macOS `.app` bundle.
`just package-windows-aio` creates the existing Windows self-contained AIO
executable.

## Usage

### GUI

```bash
./target/release/spektrafilm-gui [optional/path/to/image.orf]
```

- **Open…** — load standard images or camera RAW. RAW processing uses LibRaw and exposes as-shot/daylight/tungsten/custom Kelvin+tint white balance and Lensfun correction; RAW enters the runtime as linear ACES2065-1.
- **Sliders** — exposure, film format, halation, DIR couplers, grain, glare, scanner, enlarger, output. All live-updating against the GPU preview.
- **Profiles** — film stock and print paper combo boxes; picking a film auto-selects its paired paper (`target_print` in the profile).
- **Zoom** — scroll wheel or trackpad pinch over the preview (cursor-anchored), click-drag to pan, double-click to reset.
- **Export…** — re-runs the pipeline at f64 precision on the CPU and writes a PNG/TIFF/JPEG. Status bar shows elapsed time; **Cancel** kills the child cleanly. Closing the GUI mid-export also kills the child (no orphans).
- **Save…** — convert retained floating output into the independently selected saving color space and transfer encoding, then save at the selected bit depth. Viewer borders, watermark and display ICC transforms stay out of saved pixels.
- **Save state… / Load state…** — exchange the pinned Python 0.3.4 GUI JSON sections. Partial files merge into factory values; legacy input aliases and nested sections normalize to the flat upstream format. Invalid JSON, field types, selections or Rust extension versions appear in the status bar.
- **Save startup default / Restore factory default** — persist the current controls, or remove that default and restore Kodak Gold 200 + Kodak Supra Endura. Startup files live in the platform configuration directory (`SPEKTRAFILM_CONFIG_DIR` overrides it); Rust-only runtime/viewer settings use the explicit `rust.version = 1` extension. File-dialog directories persist separately.
- **Preview** — explicitly update the preview when auto-preview is disabled. Crop, spectral adaptation/blur, UV/IR, layered grain and both diffusion filters apply to the runtime, with the chosen preview long-edge limit.
- **Scan** — render the original-resolution image with spatial and stochastic effects; **Preview** uses the configured long-edge limit and preview digestion.
- **Viewer** — switch Input / Output / Paper back; choose nearest, linear, cubic, spline16, spline36, Lanczos or Blackman sampling. The canvas is the pinned 18% gray (`#767676`) or black, with normalized white padding and the upstream paper watermark. Reveal and crossfade affect the disposable viewing frame; hovering reports the original floating RGB pixel.
- **Display transform / Display ICC…** — on Windows, discover the primary display ICC profile or choose a profile and apply it only to the viewer. Without an available display transform the output is viewed in its selected output space; the input/reference is converted to encoded sRGB.
- **Launch state** — `spektrafilm-gui IMAGE --state GUI_STATE.json` loads a deterministic state for repeatable sessions. `SPEKTRAFILM_GUI_STATE` provides the same startup override.

### CLI

```bash
# f32 GPU (fast)
./target/release/spektrafilm process input.ORF -o out.png \
    --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# f32 native CUDA, when built with spektrafilm-gpu/cuda-backend
SPEKTRAFILM_BACKEND=cuda \
    ./target/release/spektrafilm process input.ORF -o out.png \
    --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# f64 CPU (reference)
SPEKTRAFILM_BACKEND=cpu \
    ./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# Override any params via JSON (matches RuntimeParams struct)
... --params my_params.json

# List available film + paper profiles
./target/release/spektrafilm list-profiles --data-dir data
```

Working geometry follows Python 0.3.4 (`3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`). In JSON, set `io.crop`, `io.crop_center: [x, y]`, `io.crop_size: [width, height]`, and `io.upscale_factor`. Center coordinates are normalized to the source axes; both size components are fractions of the source's long edge. Bounds and rounding follow the upstream NumPy slice convention, including negative-index slicing when a crop exceeds the short edge. Empty crops and nonpositive/nonfinite resize factors return errors before output is written.

Auto-exposure meters an antialiased nearest-neighbour preview of the complete source before crop and resize. Encoded preview samples are decoded for f64 luminance measurement; the resulting exposure scales source samples before geometry and filming decode, matching Python's order. Cropping preserves the source film pixel pitch; resizing divides it by the requested factor. Lens blur, halation, DIR, grain, and enlarger diffusion consume that retained pitch on CPU and resident GPU paths. Resize uses cubic B-spline interpolation with half-pixel coordinates, mirror boundaries, Gaussian antialiasing for downscale, ties-to-even dimensions, and source-range clipping. Interpolation computes in f64 before the configured image precision boundary.

Colour management follows the pinned Python 0.3.4 registry for sRGB, ProPhoto RGB,
ITU-R BT.2020, ACES2065-1, Adobe RGB (1998), Display P3 and DCI-P3.
`io.input_cctf_decoding` decodes encoded input for metering after preview sampling,
and for film exposure after source exposure scaling and geometry. RAW loaders supply
linear values. `io.output_cctf_encoding` selects the destination's own transfer function. Floating scan output retains
negative and super-white values with encoding enabled or disabled; bounded image
writers and the display preview perform their own clipping.

Input gamut compression uses `active`, `algorithm` (`xy` or `oklch`) and `knee`.
Output `algorithm` accepts `off`, `aces_rgc`, `oklch`, `oklrab`, `jzazbz` and
`cam16ucs`, with perceptual tables built for the selected destination. Unknown
spaces and algorithms fail before rendering. The GUI converts native output to
sRGB for display while preserving native values for saving. Resident GPU kernels
that clip or lack the selected colour transform route through the shared stage
implementation until equivalent GPU transforms are available.
Image output uses one writer for CLI, GUI Save and f64 Export. `process --bit-depth 8|16|32` (default 16) selects uint8/uint16/float32 TIFF or half/float32 EXR; 8-bit EXR is rejected. JPEG and PNG always write uint8, matching Python 0.3.4. Integer output clips to [0,1], scales and truncates; float TIFF/EXR preserves negative and super-white samples. The pipeline output space/CCTF controls both saved samples and ICC/color tags; saving applies no extra transfer function. Matching profiles are compiled from `data/icc`, including encoded/linear variants; linear P3 has no bundled upstream profile. Source EXIF/IPTC/XMP is copied for non-EXR output, with orientation, dimensions, software, date and color tags refreshed. Metadata read failures yield no source metadata; post-write metadata failures are reported as warnings while pixel-write failures are errors.

`image_io::convert_image` explicitly converts retained output into a different saving space/CCTF: source decoding, CAT02 white adaptation, destination encoding. It reuses the shared seven-space color math and preserves floating headroom. Apply this operation before `save` when saving settings differ from simulation settings; writer options describe the resulting pixels and never change the simulation gamut mapping.
The calibrated LUT creator uses the Python 0.3.4 spectral runtime in deterministic
`lut_mode`, including non-spatial DIR chemistry and neutral-print calibration.
It supports one through four LUTs, shared film stages across repeated `--print`
stocks, and `--combinations` for every contiguous collapsed sub-chain.

```bash
./target/release/spektrafilm lut list input
./target/release/spektrafilm lut list output
SPEKTRAFILM_BACKEND=cpu ./target/release/spektrafilm-f64 lut build \
    --film kodak_portra_400 --print kodak_portra_endura \
    --input vlog --output srgb --topology 4lut --resolution 33 \
    --combinations --out build/lut_bundles --data-dir data
./target/release/spektrafilm lut build --from bundle.toml \
    --resolution 65 --out build/lut_bundles --data-dir data
```

TOML fields match the typed `spektrafilm_core::lut_baker::BundleSpec`; supplied
CLI flags override file values. A minimal spec is:

```toml
film_profile = "kodak_portra_400"
print_profiles = ["kodak_portra_endura"]
input_color_space = "Panasonic V-Log"
output_color_space = "sRGB"
topology = "3lut"
resolution = 33
stops_above_midgray = "auto"
include_combinations = true

[input_gamut_compress]
algorithm = "xy"
knee = [0.0, 1.0, 6.0]

[output_gamut_compress]
algorithm = "cam16ucs"
knee = [0.0, 1.0, 6.0]
lightness_compression = [0.7, 1.0, 2.2]
```

Canonical names and registry short tags are accepted. Disabled scene-linear
registry roles fail explicitly. `auto` headroom maps encoded SDR white to four
stops above film midgray and uses native log/HDR white-to-midgray headroom;
`--stops-above-gray` overrides the linear exposure gain. PQ and HLG outputs
apply their registry midgray gain before encoding. Intermediate wires retain
the probed log-exposure margins and below-fog density headroom in `bundle.json`.

For Rust consumers, `BundleBuilder::build` returns typed `Bundle`, `Lut`, and
`BundleMeta` values. `Lut::table` is blue-fast `[r][g][b]`, indexed by
`(r * resolution + g) * resolution + b`; format writers reorder at serialization.
`export-lut` calls the same baker for a single combined ProPhoto RGB to sRGB
transform with native input headroom.
### LUT delivery

LUT bundles carry their stock, transport, exposure, topology, wire constants and parameter snapshots in `bundle.json`, with consumer instructions, the canonical upstream `SPEKTRAFILM_LICENSE.txt`, and derivative notices. Directory and ZIP delivery use portable relative artifact paths. The only active camera target is `lumix_realtime_vlog`: Panasonic V-Log input, sRGB/Rec.709/Rec.2020 output, and the minimal `#LUMIXPHOTOSTYLE VLOG` header. Upstream field verification covers the 33³ grid; other exporter-accepted resolutions have no camera compatibility claim.

The core format API reads and writes generic CUBE, strict Lumix CUBE, Autodesk 10-bit 3DL and Hald RGB8 PNG. Hald requires a perfect-square cube resolution (N=L²), packs an L³-square image, and quantizes without a transfer curve. 3DL and Hald match NumPy ties-to-even quantization and clamp output codes. Wire file order is red-fast; baker tables remain blue-fast. OCIO and QA producers can append relative artifacts and finalize the complete bundle before ZIP creation.

`lut build --ocio-config` emits a standalone OCIO 2.4 configuration and records its relative artifact path. `--qa` runs all sixteen pinned quality scenarios for every print; `--qa-print-index N` selects one print. QA writes JSON, Markdown, offline HTML and PNG figures under `qa/` and registers every file before ZIP finalization. The command prints the measured QA PASS/FAIL result; a physical quality failure remains visible in the delivered report.

Profiles and their derived LUTs are by Andrea Volpato, licensed under **CC BY-SA 4.0**, from [the canonical spektrafilm source](https://github.com/andreavolpato/spektrafilm), pinned to `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`. Their asset license is separate from this Rust program's GPL-3.0 license. Bundles preserve the source license bytes and record migration, bake settings and quantization modifications.

## Parity

Verified against the upstream Python `spektrafilm` v0.3.2 reference on the bare-chain (no stochastic FX):

| Stage | Max diff | Mean diff | Identical pixels |
|---|---|---|---|
| `log_raw` (post-Hanatos + log10) | **8.9 × 10⁻¹⁵** (one f64 ULP) | 3.4 × 10⁻¹⁶ | — |
| Film density CMY | **1.1 × 10⁻¹⁵** | 1.7 × 10⁻¹⁶ | — |
| Print density CMY | **3.1 × 10⁻¹⁵** | 3.0 × 10⁻¹⁶ | — |
| Final PNG (8-bit) | **1 / 255** | 0.00004 / 255 | **99.9962 %** |

With grain on, the binomial sampler's rejection step is sensitive to upstream ULP shifts and the rendered grain texture diverges per pixel — this is by design (matches numpy's behaviour) and produces the same average tone with a different grain pattern.

The numbers above are the historical 0.3.2 session measurements. The current parity target is the pinned upstream **0.3.4** (commit `3bb2c2d`): see [`docs/parity/baseline_evidence.md`](docs/parity/baseline_evidence.md) for the preserved 4×4 bare-chain evidence (≈1.14 × 10⁻⁸ max abs, f64) and the budget table, [`docs/parity/parity_matrix.md`](docs/parity/parity_matrix.md) for the field/asset inventory, and [`scripts/parity/README.md`](scripts/parity/README.md) for the reproducible differential harness.

Backend selection in a `precision-f64` binary defaults to genuine CPU f64 arithmetic, even when GPU features are compiled in. `SPEKTRAFILM_BACKEND=wgpu` or `cuda` explicitly requests f32 preview arithmetic; backend initialization reports `precision=f32` and `reference=false`. An unavailable requested adapter reports its unavailability and uses CPU; that fallback is not evidence of a GPU pass. CUDA adapter execution remains unavailable in the migration environment.

Resident WGSL/CUDA chains return unclipped linear destination RGB. The shared CPU post-scan stage applies the destination's same-space matrix roundtrip and optional transfer curve, preserving negative values and highlights above one even when encoding is disabled. Input transfer decoding, layered grain, active unsupported gamut algorithms/spaces, and camera/enlarger optical diffusion use an explicitly reported faithful per-stage path. Optical diffusion uses the finite sampled, normalized PSF with reflected-boundary FFT convolution on CPU; the discarded Gaussian mixture drifted by **0.08603** on a 48-pixel black-pro-mist hotspot at spatial scale 0.05, exceeding its unchanged **0.005** regression limit.

Requests for enlarger/scanner PCHIP spectral LUTs also select the per-stage path, preserving the requested sampling and interpolation rather than substituting direct resident spectral integration. GPU shader pipeline caches and one-upload/one-readback batching remain active for supported resident configurations.

Gaussian requests exceeding the GPU's 256-pixel FIR half-width explicitly fall back to CPU blur, including resident halation bounce radii, DIR, grain/glare, scanner, and camera blur. The requested radius is preserved rather than silently truncated. After integrating the migration slices, run `cargo run -p spektrafilm-core --example backend_parity --features precision-f64 -- data` for actual WGSL resident/per-stage comparisons across seven destination spaces, encoding modes, geometry, darks/highlights, spectral/optical controls, and faithful fallback cases. It fails on unavailable adapters and prints measured maximum/mean errors against CPU with a provisional 0.005 smoke limit.

Budgets apply to different measurements: reference f64 arithmetic uses max absolute error **1e-6** against the pinned matrix; CPU-f32/GPU arithmetic must report its measured maximum and mean error independently and has no approved universal budget yet. Stochastic appearance uses the provisional **1% mean / 5% standard deviation** relative-error budget rather than per-pixel equality. The spatial **0.005** hotspot limit now guards the faithful diffusion fallback, not an approved GPU blur approximation. End-to-end preview and other spatial budgets in the linked evidence table remain provisional until actual scenario measurements are recorded; adapter initialization or compilation alone does not establish parity.

Grain dispatch mirrors upstream `apply_grain`: `sublayers_active` (default, matching 0.3.4) runs the layered model — the composite density is split through `density_curves_layers` into per-sublayer densities, each sublayer samples its own Poisson-binomial particle field with per-layer density maxima/fractions, particle scaling, dye-cloud blur and lognormal micro-structure — while `sublayers_active: false` keeps the single composite-density sampler (the only model with a GPU shader; the resident preview chain falls back to the CPU stages when the layered model is on so no layer control is silently ignored). `settings.use_fast_stats` switches the layered sampler to the `fast_stats` kernels like upstream; the micro-structure clumping field always uses them (upstream does too) and is pinned to a documented deterministic seed because Python draws it from numba's unseeded thread-global RNG.

Both composite and layered grain use the CPU sampler during GPU preview. The resident composite shader approximates every Poisson/binomial draw by a normal distribution, including dark/highlight pixels with low binomial variance; those valid cases require the faithful CPU distribution. Grain-active rendering explicitly selects the per-stage path rather than claiming GPU grain parity.

The LUT path (`use_enlarger_lut` + `use_scanner_lut`) ports Python's PCHIP 3D interpolation (`crates/spektrafilm-math/src/pchip3d.rs` ↔ `spektrafilm/utils/fast_interp_lut.py`). The executed f64 LUT-reduction scenarios passed the **1e-5** maximum absolute-error budget; the 265 delivered LUT stock/topology/transport cases passed independent Python comparisons.

## Performance

Historical pre-migration timing on a 16 MP Olympus ORF (kodak_gold_200 → kodak_portra_endura, full FX), retained for context. The migrated layered grain and CPU fallback paths require fresh timing before applying these figures to current builds:

| | Wall time | Notes |
|---|---|---|
| Python reference (numpy + numba) | 22 s | LUT enabled, default config |
| **spektrafilm-rs (f64 CPU)** | **14 s** | **35 % faster than Python** |

What gets it there:

- **PCHIP LUTs** for the spectral integrations (enlarger + scanner) — same approximation Python uses, same accuracy budget.
- **`vForce vvpow`** for `10^x` on the spectral chain — Accelerate's SIMD pow, bit-identical to libm `pow(10, x)`.
- **Accelerate BLAS dgemm** for the spectral reductions — a single `cblas_dgemm` per contraction, parallelised internally by Accelerate. (It is not safe to call concurrently from multiple threads, so the matmul is never split across rayon.)
- **Parallelised hot per-pixel loops** in the printing and scanning post-stages.

GPU preview uses f32 wgpu compute shaders (`crates/spektrafilm-shaders/wgsl/`) and optional f32 CUDA kernels. Faithful CPU stages and destination post-scan handling participate where required. Historical Apple Silicon preview timings (~250 ms at 6 MP, ~700 ms at 16 MP) describe the earlier supported chain; they are not measured performance claims for the migrated controls or CPU fallback paths.

## Layout

```
crates/
  spektrafilm-math/    f64 reference math (spectral, interp, PCHIP, RNG, vForce bindings)
  spektrafilm-model/   stochastic + physical models (grain, halation, DIR couplers, glare)
  spektrafilm-core/    pipeline orchestration, profiles, stage definitions
  spektrafilm-gpu/     ComputeBackend trait + CPU (rayon + BLAS) and wgpu backends
  spektrafilm-shaders/ WGSL / Metal / CUDA compute shaders
  spektrafilm-cli/     `spektrafilm` / `spektrafilm-f64` (process, list-profiles, lut, export-lut) + `decode_raw_gui`
  spektrafilm-gui/     egui/eframe preview (wgpu renderer, Metal-backed on macOS)
  spektrafilm-raw/     shared native LibRaw white balance and Lensfun correction
data/
  profiles/            film + paper JSON profiles (spectral sensitivities, density curves, etc.)
  luts/                Hanatos2025 spectral basis + standard observer CMFs (.npy)
  filters/             neutral-print enlarger filter database
  icc/                 complete upstream ICC assets and source license notices
  license/             canonical spectral-data license
scripts/parity/        Python 0.3.4 ↔ Rust differential harness (scenarios.py, py_reference.py, run_parity.py, gen_matrix.py)
```

## Credits

Original Python implementation by Andrea Volpato — [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm). All spectral data, film/paper profiles, and pipeline architecture come from there. This port owes its existence to Andrea and its work.

The spectral-upsampling LUT (`hanatos2025_*`) is named after [Johannes Hanatos](https://github.com/hanatos), author of [vkdt](https://github.com/hanatos/vkdt), who provided the upstream Python project with the LUT files and sample code that drive the RGB → spectrum step.

PCHIP 3D LUT, MT19937 binomial sampler, and CIE 1931 observer constants are ported from numpy / scipy / scikit-image / colour-science.

Claude code for being this awesome.

## License

The Rust program is GPL-3.0; see [LICENSE](LICENSE). Andrea Volpato's spectral profiles and their LUT derivatives retain CC BY-SA 4.0; see [the canonical asset license](data/license/SPEKTRAFILM_LICENSE.txt).
