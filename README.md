# spektrafilm-rs

![banner](docs/card.jpg)

A Rust port of [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm) — a spectral simulator for analogue colour film and the print-and-scan chain.

The spectral chain (RGB → film dye density → enlarger illuminant → print paper → scanner RGB) is migrated against pinned Python spektrafilm 0.3.4 commit `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`. CPU export uses f64; GPU preview uses f32 with explicit CPU routing for effects without a faithful GPU implementation. Numeric agreement is assessed with per-scenario budgets, rather than universal bit identity.

---

## What it does

- **Spectral pipeline.** Hanatos2025 RGB→raw spectral upsampling with its full sensitivity adaptation (camera UV/IR band-pass filters with reference-illuminant normalization, erf4 band-pass window, poly4 log-exposure surface, spectral Gaussian blur), full 81-wavelength film/print/scanner spectral integration, density-curve interpolation, halation, DIR couplers, grain (bit-exact numpy `MT19937` port), glare, output CCTF encoding.
- **Interactive preview** through wgpu (Metal on macOS), with CPU stages for exact optical diffusion and V1 grain sampling; V2 uses a compute shader. Frame rate depends on image size, controls and hardware.
- **Reference export** on the CPU at f64. Historical bare-chain evidence and applicable comparison budgets are recorded in [baseline evidence](docs/parity/baseline_evidence.md); fresh integrated comparisons are required for migration acceptance.
- **Selectable CPU/GPU export.** Choose `CPU (f64)` (default) or `GPU (WGPU f32)` in the GUI's **Export backend** menu. Both re-render at export resolution through `spektrafilm-f64`, independently of preview, with cancellation and atomic output publication. The choice is saved with GUI state. GPU uses f32 shaders and retains CPU stages for unsupported effects; it is not f64 reference output. An unavailable WGPU adapter is an error for explicit GPU export.
- **Profiles bundled.** 30+ film and paper profiles in `data/profiles/` — Kodak Gold/Portra/Ektar, Fuji Velvia/Provia, Kodak Endura papers, Fuji Crystal Archive papers.
- **Experimental workflow routes.** Runtime/GUI state accepts passthrough, film-scan, film-print-scan, and the three convert-film routes. Convert-film inverts the spectral scan model with bounded Gauss-Newton, supports scan illuminant/exposure/calibration controls, and can scan with or without the film base.
- **Camera taking filters.** The measured Hoya X0, X1, Y2, YA3 and R1 transmission curves are selectable in runtime params and the GUI; changing the filter invalidates the sensitivity-dependent spectral cache.
- **Selectable grain engines.** V1 remains the default emulsion model. V2 provides procedural Analogue/Noise grain with twelve format/speed profiles, Size, Amount, Shadows, Midtones, Highlights, Chroma and Film Resolution controls.

## Build

Requires Rust stable (≥ 1.88), a C++17 compiler, pkg-config, OpenImageIO and Exiv2 development libraries. The locked `image` dependency requires Rust 1.88. Image I/O uses the same native libraries as Python 0.3.4, preserving float samples and EXIF/IPTC/XMP. On Debian/Ubuntu install `libopenimageio-dev libexiv2-dev libopenblas-dev`; Linux links the installed OpenBLAS library. On macOS use `brew install openimageio exiv2 pkg-config`. Windows packaging uses a matching MSYS2 UCRT64 native toolchain and pkg-config dependencies. The package scripts collect native runtime libraries; the build fails explicitly when required development libraries are absent.

```bash
git clone <this-repo> && cd spektrafilm-rs

# GUI (wgpu/Metal preview, eframe)
cargo build --release -p spektrafilm-gui

# f32 CLI — fast batch processor, defaults to WGPU
cargo build --release -p spektrafilm-cli


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
The release workflow intentionally invokes package smoke without `--gui`.
Therefore its package status covers CLI/RAW/IO/LUT/OCIO/QA and provenance, not
native desktop interaction. GUI evidence is maintained separately and is not
silently counted as a release pass.

The Linux native GUI acceptance run against the debug GUI and f64 exporter under Xvfb passed the five-tab shell, native dialogs, Preview/Scan, quarter-turn pipeline rotation, exact 100/200/400% zoom (192/384/768 px bounds), Paper back/watermark, interpolation, gray canvas, white border, Reveal/Crossfade frames, float inspection, profile selection, ProPhoto RGB input/Display P3 output, state/startup round trips, RAW ACES2065-1 loading and Cancel/close cleanup. Viewer-only changes produced zero decoded Save/Export pixel differences; rotated export input matched NumPy exactly and retained EXIF/IPTC/XMP with normalized orientation/dimensions. The complete records and executable hashes are embedded under `gui_evidence` in `docs/parity/parity_matrix.json`; local smoke at Rust `8bdd7fe` and CI workflow `37226896580` provide the current Linux evidence. The same workflow passed Windows x64 package smoke. macOS remains incomplete: its real native run reaches the viewer transition tests but currently fails the Reveal-frame assertion; this is reported as unverified rather than hidden.

Quarter-turn actions rotate the pipeline input and preserve source metadata. The export worker stages rotated pixels as a 32-bit TIFF; the metadata writer normalizes orientation and dimensions. Cancellation is checked before and after staging, so a cancelled export does not launch the CPU child. A native TIFF write cannot be interrupted; closing during that write waits for it to finish and removes the temporary file.

`spektrafilm-raw` returns linear ACES2065-1 pixels after LibRaw camera orientation, optional Lensfun correction, and white balance. Camera WB (`as-shot`) is the default; `daylight` uses LibRaw's daylight base, while `tungsten` and `custom` adapt that base using the reference CIE D/Kang 2002 whitepoints and Von Kries/CAT02 transform. Custom temperature must be finite in 1667–25000 K and custom tint finite; absent custom temperature is an error. Lensfun applies vignetting before combined channel geometry resampling with bilinear interpolation and nearest boundary extension, and preserves unchanged pixels/empty lens summary when camera or lens matching fails.

The shared helper accepts `decode_raw_gui input.CR2 output.tif --raw-white-balance custom --raw-temperature 5000 --raw-tint 1.05 --lens-correction` and writes **linear ACES2065-1 float TIFF**, suitable for pipeline input configured as ACES2065-1 with input CCTF decoding disabled. It no longer writes linear sRGB through rawler.

Real RAW provenance is retained in [`scripts/parity/raw_fixtures.json`](scripts/parity/raw_fixtures.json); downloaded images are not redistributed. Run `scripts/parity/raw_reference_compare.py input.KDC --decoder target/release/decode_raw_gui --source-url URL --max-error 0 --mean-error 0` in the pinned Python environment for fresh four-mode decoder/WB evidence. The exercised Kodak DC50 KDC decodes to 768×512 and matches LibRaw 0.22.0/rawpy 0.26.1 at zero max/mean error for all four WB modes. Its EXIF has no lens model, exercising unchanged no-match correction; a separately injected Canon EOS 5D Mark II / EF 24–70mm f/2.8L / 35mm f/8 EXIF fixture matches upstream Lensfun pixels and summary exactly at 64×48 and 640×480. A public `M0054341_01_00005.cr2` fixture is intrinsically damaged (verified Git blob), and both paths reject it with a data error. These measurements do not imply support for every recognised RAW suffix.

The public Canon EOS 40D sRAW CR2 fixture also matches all four modes at zero max/mean error on 1944×1296 pixels, with missing-lens correction enabled. Unlike Kodak's unit camera multipliers, this camera's recorded WB `[2341,1024,1570,1024]` exercises distinct camera and daylight output (max change 0.212146, mean change 0.007560). Run `scripts/parity/lens_reference_compare.py` for the known-lens, missing-metadata, missing-camera and missing-lens comparisons; it uses actual Exiv2 metadata and native Lensfun, requires all dependencies, and defaults to exact pixel/summary equality.

The rebuilt f64 CLI and its relocated Linux archive also passed eight fresh RAW comparisons against the pinned Python loader: all four white-balance modes on Kodak 768×512 with missing-lens correction, and Canon 1944×1296 with injected known-lens EXIF. The unbounded `rgb_in` boundary matched exactly (maximum and mean absolute error zero). The relocated RAW helper independently matched Kodak pixels exactly under a clean environment; the packaged GUI launched under Xvfb and prepared-image export preserved EXIF/IPTC/XMP and ICC bytes.

WGPU/WGSL is the only GPU backend and can be selected explicitly with `SPEKTRAFILM_BACKEND=wgpu`; CPU is the fallback when no usable adapter is available. The interactive preview uses f32 GPU arithmetic where supported and faithful CPU per-stage routing for effects without a faithful GPU implementation. Reference export remains CPU f64.
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

- **Sidebar workflow** — the Rust GUI follows the upstream sidebar tabs: **MAIN** (load, input image, profiles, exposure, crop, preview/RAW, scanner, enlarger and output), **FILM** (halation, DIR couplers, diffusion and grain), **PRINT** (glare, print curves, enlarger details/diffusion and saving color), **ADVANCED** (spectral/color controls), and **CONFIG** (state persistence and display controls). The tab row and Preview/Scan action bar remain fixed while each tab's controls scroll.
- **Open…** — load standard images or camera RAW. RAW processing uses LibRaw and exposes as-shot/daylight/tungsten/custom Kelvin+tint white balance and Lensfun correction; RAW enters the runtime as linear ACES2065-1.
- **Input image / Profiles** — choose input/output color workflow, film stock and print paper. Picking a film auto-selects its paired paper (`target_print` in the profile).
- **Sliders** — exposure, film format, halation, DIR couplers, grain, glare, scanner, enlarger and output. Changes follow the selected sidebar tab and update the GPU preview according to Auto preview.
- **Viewer controls** — `ccw rotate` and `cw rotate` physically rotate the in-memory input used by Preview, Save and f64 Export; `100%`, `200%` and `400%` map source pixels to exact device-pixel percentages; reset view returns to fit.
- **Export…** — re-runs the pipeline at the selected CPU f64 or GPU f32 backend and writes PNG/JPEG/TIFF/OpenEXR. JPEG exposes quality (1–100) and 4:4:4/4:2:0 chroma sampling; TIFF/EXR expose ZIP/none compression where supported; format-specific bit-depth rules are validated before rendering. Status bar shows elapsed time; **Cancel** kills the child cleanly. Closing the GUI mid-export also kills the child (no orphans).
- **Save…** — convert retained floating output into the independently selected saving color space and transfer encoding, then save at the selected bit depth. Viewer borders, watermark and display ICC transforms stay out of saved pixels.
- **Save state… / Load state…** — exchange the pinned Python 0.3.4 GUI JSON sections. Partial files merge into factory values; legacy input aliases and nested sections normalize to the flat upstream format. Invalid JSON, field types, selections or Rust extension versions appear in the status bar.
- **Save startup default / Restore factory default** — persist the current controls, or remove that default and restore Kodak Gold 200 + Kodak Supra Endura. Startup files live in the platform configuration directory (`SPEKTRAFILM_CONFIG_DIR` overrides it); Rust-only runtime/viewer settings use the explicit `rust.version = 1` extension. File-dialog directories persist separately.
- **Preview** — explicitly update the preview when auto-preview is disabled. Crop, spectral adaptation/blur, UV/IR, layered grain and both diffusion filters apply to the runtime, with the chosen preview long-edge limit.
- **Scan** — render the original-resolution image with spatial and stochastic effects; **Preview** uses the configured long-edge limit and preview digestion.
- **Scan-for-print** — temporarily enable scanner white/black corrections and disable print glare. Toggle again to restore all three previous values. Loading state or changing profiles clears the transient snapshot.
- **Viewer** — switch Input / Output / Paper back; choose nearest, linear, cubic, spline16, spline36, Lanczos or Blackman sampling for Output. Input retains the upstream nearest interpolation; Paper back uses spline36. The configured preview limit bounds disposable Input/Output rasters. The canvas is the pinned 18% gray (`#767676`) or black, with normalized white padding and the upstream paper watermark. Reveal and crossfade affect the disposable viewing frame; hovering reports the original floating RGB pixel.
- **Display transform / Display ICC…** — when enabled, every platform converts output-space pixels to encoded sRGB for the disposable viewer raster; raw Save/Export pixels remain in the selected output space. Windows discovers the primary display ICC profile, and an explicitly selected ICC profile is applied on every platform that supports LittleCMS. On macOS the Metal surface is tagged sRGB; without an ICC profile the encoded sRGB preview is used directly.
- **Launch state** — `spektrafilm-gui IMAGE --state GUI_STATE.json` loads a deterministic state for repeatable sessions. `SPEKTRAFILM_GUI_STATE` provides the same startup override.

### CLI

```bash
# GPU export: WGPU f32 shaders, CPU stages where required
./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --backend gpu --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# CPU f64 reference export
./target/release/spektrafilm-f64 process input.ORF -o out.png \
    --backend cpu --film kodak_gold_200 --paper kodak_portra_endura --data-dir data

# Explicit JPEG quality and chroma sampling
./target/release/spektrafilm-f64 process input.tif -o out.jpg \
    --format jpeg --bit-depth 8 --jpeg-quality 95 --jpeg-subsampling 444 \
    --backend cpu --film kodak_gold_200 --scan-film --data-dir data


# Override any params via JSON (matches RuntimeParams struct)
... --params my_params.json

# List available film + paper profiles
./target/release/spektrafilm list-profiles --data-dir data
```

`process --backend cpu|gpu` overrides `SPEKTRAFILM_BACKEND`; omitting it preserves the environment/default selection. `--format` must match the output extension when supplied; otherwise the extension selects JPEG/PNG/TIFF/EXR. Bit depth defaults to 8 for JPEG/PNG and 16 for TIFF/EXR; JPEG/PNG require 8-bit, EXR requires 16- or 32-bit. JPEG defaults to quality 95 and 4:4:4; `--compression zip|none` applies to TIFF/EXR, while JPEG/PNG reject compression options. CPU precision follows the executable build: use `spektrafilm-f64` for reference exports. Output bit depth is independent of computation precision. GPU selection does not disable grain, optical effects or requested spectral LUTs to force acceleration, and software Vulkan adapters can also execute the WGPU path; speed depends on the adapter and active effects.

Working geometry follows Python 0.3.4 (`3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`). In JSON, set `io.crop`, `io.crop_center: [x, y]`, `io.crop_size: [width, height]`, and `io.upscale_factor`. Center coordinates are normalized to the source axes; both size components are fractions of the source's long edge. Bounds and rounding follow the upstream NumPy slice convention, including negative-index slicing when a crop exceeds the short edge. Empty crops and nonpositive/nonfinite resize factors return errors before output is written.

Auto-exposure meters an antialiased nearest-neighbour preview of the complete source before crop and resize. Encoded preview samples are decoded for f64 luminance measurement; the resulting exposure scales source samples before geometry and filming decode, matching Python's order. Cropping preserves the source film pixel pitch; resizing divides it by the requested factor. Lens blur, halation, and DIR consume that retained pitch on CPU and supported resident GPU paths; grain and faithful optical diffusion remain on the CPU per-stage path. Resize uses cubic B-spline interpolation with half-pixel coordinates, mirror boundaries, Gaussian antialiasing for downscale, ties-to-even dimensions, and source-range clipping. Interpolation computes in f64 before the configured image precision boundary.

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
Image output uses one writer for CLI, GUI Save and f64 Export. `process --format jpeg|png|tiff|exr` selects the container; `--bit-depth 8|16|32` selects uint8/uint16/float32 TIFF or half/float32 EXR, with JPEG/PNG restricted to uint8 and EXR rejecting 8-bit. JPEG quality is 1–100 and chroma sampling is 4:4:4 or 4:2:0; TIFF compression accepts ZIP/none and EXR uses ZIP. Integer output clips to [0,1], scales and truncates; float TIFF/EXR preserves negative and super-white samples. The pipeline output space/CCTF controls both saved samples and ICC/color tags; saving applies no extra transfer function. Matching profiles are compiled from `data/icc`, including encoded/linear variants; linear P3 has no bundled upstream profile. Source EXIF/IPTC/XMP is copied for non-EXR output, with orientation, dimensions, software, date and color tags refreshed. Metadata read failures yield no source metadat…

Explicit JPEG quality uses OpenImageIO's `CompressionQuality` attribute; `jpeg:quality` is ignored by the encoder. JPEG still uses the encoder's default chroma subsampling (4:2:0 with the tested OpenImageIO 2.5.19.1), which can soften colored grain even at quality 100. Use 16/32-bit TIFF to compare grain detail without JPEG compression; increasing JPEG quality does not remove chroma subsampling.

`image_io::convert_image` explicitly converts retained output into a different saving space/CCTF: source decoding, CAT02 white adaptation, destination encoding. It reuses the shared seven-space color math and preserves floating headroom. Apply this operation before `save` when saving settings differ from simulation settings; writer options describe the resulting pixels and never change the simulation gamut mapping.
The calibrated LUT creator uses the Python 0.3.4 spectral runtime in deterministic
`lut_mode`, including non-spatial DIR chemistry and neutral-print calibration.
It supports one through four LUTs, shared film stages across repeated `--print`
stocks, and `--combinations` for every contiguous collapsed sub-chain.


The experimental spectral registry ships the `hanatos2025`, `mallett2019`,
`arctic2026alpha02`, `arctic2026beta04`, `gauss-lasers`, `jakob2019`, and
`otsu2018` methods. Runtime parameters can select a method and an illuminant
(`A`, `D50`, `D55`, `D60`, `D65`, `D75`, `E`, `T`, `TH-KG3`, `TH-KG3-L`,
`K75P`, or `BB<temperature>` in the supported 1667–25000 K range). LUT builds
accept `--params runtime_params.json`, `--stops-above-midgray
<auto|native|null|STOPS>`, and the legacy additive `--exposure-ev EV`.
Schema 3 bundles record the full digested parameter tree per print, the
digest's changed values, a SHA-256 snapshot digest, and resolved input stops
and gain. `Bundle::baked_params` preserves the actual bake configuration for
QA. Grain and coupler TOML presets override controls when stock specifics are
requested; later edits use `apply_stocks_specifics=false`. All fitted density
models refresh sampled curves on load, including `sept_norm_cdfs` with
per-layer median-preserving skew parameters.
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
The CLI also resolves the packaged `../share/data` directory for `lut` and
`export-lut` when invoked outside the package directory. Profile metadata and
array dimensions are validated before construction; present but malformed
neutral-filter JSON is an error rather than an empty-database fallback.

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
registry roles fail explicitly. The default `"auto"` bridge resolves to four
stops for encoded SDR inputs and six stops for scene-referred camera-log
inputs; `native`/`null` preserves the registry's native gain. Explicit stops
use `0.18 * 2^stops / decode(1)`, and legacy `--exposure-ev` adds a deliberate
multiplier of `2^EV`. The resolved stops and gain are recorded in
`bundle.json`. PQ and HLG outputs apply the inverse midgray bridge before
encoding. Intermediate wires retain the probed log-exposure margins and
below-fog density headroom.

Params-first baking preserves runtime gamut and look controls, forces the
film-print-scan route, and clears taps and preview mode. LUT digestion disables
crop, resize and internal spectral acceleration LUTs, and neutralizes per-image
exposure, enlarger filter shifts and preflash. The snapshot's `digest_changes`
records these changes relative to an ordinary render of the same settings.

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

Backend selection in a `precision-f64` binary defaults to genuine CPU f64 arithmetic. `SPEKTRAFILM_BACKEND=wgpu` explicitly requests f32 WGPU preview arithmetic; backend initialization reports `precision=f32` and `reference=false`. An unavailable WGPU adapter reports its unavailability and uses CPU; that fallback is not evidence of a GPU pass.

Recipe `seed` values are admitted in the unsigned 32-bit range (`0..=4_294_967_295`) because the WGPU stochastic path uses 32-bit seed state; silently truncating a caller-provided 64-bit seed would violate deterministic recipe semantics.

The WGPU resident chain returns unclipped linear destination RGB when Grain V2 is inactive; the shared CPU post-scan stage applies the destination's same-space matrix roundtrip and transfer curve only for encoded output. With Grain V2 active, the GPU applies that matrix and the selected destination curve before the shared grain preparation/filter/composition passes. The resident result is already encoded; linear exports decode only the transfer on the CPU. This keeps Grain V2 in one GPU command buffer with one upload and one readback. Input transfer decoding, V1 grain, active unsupported gamut algorithms/spaces, and camera/enlarger optical diffusion use an explicitly reported faithful per-stage path. Optical diffusion uses the finite sampled, normalized PSF with reflected-boundary FFT convolution on CPU; the discarded Gaussian mixture drifted by **0.08603** on a 48-pixel black-pro-mist hotspot at spatial scale 0.05, exceeding its unchanged **0.005** regression limit.

Requests for enlarger/scanner PCHIP spectral LUTs also select the per-stage path, preserving the requested sampling and interpolation rather than substituting direct resident spectral integration. GPU shader pipeline caches and one-upload/one-readback batching remain active for supported resident configurations.

Gaussian requests exceeding the GPU's 256-pixel FIR half-width explicitly fall back to CPU blur, including resident halation bounce radii, DIR, glare, scanner, and camera blur. Grain V1 remains on the faithful CPU per-stage path; Grain V2 uses its resident GPU shader when the rest of the resident-chain requirements are met. The requested radius is preserved rather than silently truncated. After integrating the migration slices, run `cargo run -p spektrafilm-core --example backend_parity --features precision-f64 -- data` for actual WGSL resident/per-stage comparisons across seven destination spaces, encoding modes, geometry, darks/highlights, spectral/optical controls, and faithful fallback cases. It fails on unavailable adapters and prints measured maximum/mean errors against CPU with a provisional 0.005 smoke limit.

Budgets apply to different measurements: reference f64 arithmetic uses max absolute error **1e-6** against the pinned matrix; CPU-f32/GPU arithmetic must report its measured maximum and mean error independently and has no approved universal budget yet. Stochastic appearance uses the provisional **1% mean / 5% standard deviation** relative-error budget rather than per-pixel equality. The spatial **0.005** hotspot limit now guards the faithful diffusion fallback, not an approved GPU blur approximation. End-to-end preview and other spatial budgets in the linked evidence table remain provisional until actual scenario measurements are recorded; adapter initialization or compilation alone does not establish parity.

Grain V2 is selected with `film_render.grain.engine: "v2"`. `v2_profile` defaults to `"35mm250"`; presets are `8mm50`, `8mm250`, `8mm500`, `16mm50`, `16mm250`, `16mm500`, `35mm50`, `35mm250`, `35mm500`, `65mm50`, `65mm250` and `65mm500`. Presets supply the grain configuration; Amount remains adjustable through `v2_amount` (0–100). Select `"custom"` to edit `v2_film_type` (`"negative"` or `"positive"`), `v2_mode` (`"analogue"` or `"noise"`), `v2_size` (1–48), `v2_shadows`, `v2_midtones`, `v2_highlights`, `v2_chroma`, and `v2_resolution_factor` (all 0–100). Custom values omitted or null inherit `35mm250`; switching to Custom in the GUI copies the current preset and Amount. Selecting a preset restores its Amount. All numeric controls must be finite and within their stated ranges. Film Resolution 100 preserves detail; lower values increase resolution blur. Film Type selects the reference host's resolution branch. The extra `v2_resolution_type` and `v2_timer` controls have been removed; the recipe's `random_seed` supplies the static grain phase.

V2 consumes and returns native display-encoded RGB without an internal transfer curve or primaries conversion, matching Dehancer's photo `by_pass` contract. The scanner encodes its generated RGB using the selected output color space before Grain; encoded exports keep that result, while linear exports decode only the selected transfer afterward. Grain receives the same encoded image for both export choices. V1's density-domain sampling and defaults remain unchanged.

V2 restores the recovered RGBA content hash, sine permutation, gradient noise, texture sampling and half-storage boundaries. Positive Film Type uses folded Gaussian FastBlur; Negative uses the OpticalResolution kernel. CPU/WGSL share portable trigonometry, so random realizations can differ from vendor device arithmetic. Amount and tone controls use the reference effective-control polynomial, including its nonzero intercept at zero; disable Grain with `active: false` for an exact bypass. No overscan/damage-mask input or device-specific virtual-texture cap is implemented for the unmasked photo path. See [grain research notes](docs/dehancer-grain-reverse-engineering.md) for measured evidence and coverage limits.

Noise uses a resolution-scaled sampling denominator `(1 + (Size - 1) / 47) * 2.4 * max(width / 1920, height / 1080)` and half the effective Amount before composition. Noise derives its content phase from the Film Resolution result. Analogue samples the original encoded source for its virtual grain texture and composites over the Film Resolution result. A recipe seed maps through MT19937 to a repeatable phase; Dehancer's global random stream is not a recipe seed contract.

V1 grain dispatch mirrors upstream `apply_grain`: `sublayers_active` (default, matching 0.3.4) runs the layered model — the composite density is split through `density_curves_layers` into per-sublayer densities, each sublayer samples its own Poisson-binomial particle field with per-layer density maxima/fractions, particle scaling, dye-cloud blur and lognormal micro-structure — while `sublayers_active: false` keeps the single composite-density sampler. `settings.use_fast_stats` switches the layered sampler to the `fast_stats` kernels like upstream; the micro-structure clumping field always uses them (upstream does too) and is pinned to a documented deterministic seed because Python draws it from numba's unseeded thread-global RNG.

Both V1 composite and layered grain use the CPU sampler during preview and export; no resident WGPU V1 grain shader is maintained. Active V1 grain rendering selects the faithful per-stage path. The separate Grain V2 implementation remains available.

The LUT path (`use_enlarger_lut` + `use_scanner_lut`) ports Python's PCHIP 3D interpolation (`crates/spektrafilm-math/src/pchip3d.rs` ↔ `spektrafilm/utils/fast_interp_lut.py`). The executed f64 LUT-reduction scenarios passed the **1e-5** maximum absolute-error budget; the 265 delivered LUT stock/topology/transport cases passed independent Python comparisons.

Rust profile authoring supports `profile::save_profile`, preserving JSON nulls
and leaving the source profile unchanged, and
`density_curves::parametric_density_curves_model`. The `measurement` module
provides not-a-knot cubic gamma inversion and local exposure slopes with
explicit errors for invalid samples. Python research utilities
for nonlinear toe fitting (`measure_density_min(control_plot=...)`) and
interactive plotting remain a separate developer API scope: no runtime,
GUI or LUT Creator code calls them. They are not included in the user-feature
parity claim.

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

GPU preview uses f32 WGPU compute shaders. Effect-specific shaders live beside their Rust dispatch code in `crates/spektrafilm-gpu/src/wgpu_backend/`; spectral shaders remain in `crates/spektrafilm-shaders/wgsl/spectral/`. Faithful CPU stages and destination post-scan handling participate where required. Historical Apple Silicon preview timings (~250 ms at 6 MP, ~700 ms at 16 MP) describe the earlier supported chain; they are not measured performance claims for the migrated controls or CPU fallback paths.

## Layout

```
crates/
  spektrafilm-math/    f64 reference math (spectral, interp, PCHIP, RNG, vForce bindings)
  spektrafilm-model/   stochastic + physical models (grain, halation, DIR couplers, glare)
  spektrafilm-core/    pipeline orchestration, profiles, stage definitions
  spektrafilm-gpu/     ComputeBackend trait + CPU (rayon + BLAS) and wgpu backends
  spektrafilm-shaders/ spectral WGSL shaders and standalone Metal sources
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

Feature ownership stays inside the existing crates:

- `spektrafilm-model/src/`: `grain/{v1,v2}.rs`, `halation/`, `diffusion/`, `couplers/`, and `glare/` own their models and numerical tests; `optics/` owns shared physical blur, unsharp masking, and highlight boost.
- `spektrafilm-core/src/params/`: grain, halation, diffusion, couplers, and glare each own their parameter types and feature-specific defaults. `RuntimeParams` remains the aggregate; serialized JSON fields are unchanged.
- `spektrafilm-gpu/src/wgpu_backend/`: each effect owns its buffers, pass encoding, and dedicated WGSL. Shared blur infrastructure stays in `blur/`; `mod.rs` retains device setup and film-chain orchestration.
- `spektrafilm-gui/src/panels/`: effect panels edit typed parameters and return whether controls changed; the application retains preview scheduling and persistence.

Canonical Rust paths include `spektrafilm_model::grain::v1`, `spektrafilm_model::grain::v2`, and `spektrafilm_core::params::grain::GrainParams`; the former flat module paths have been removed.

After integrating mainline commit `0d194c3`, the feature-directory refactor passed the workspace all-target/all-feature check, 192 existing default tests, and 193 existing f64 tests (including GUI state tests). Ten CPU f64 render/export cases cover V1 and V2 Analogue/Noise, Negative/Positive film types, and print/film-scan output. Their float TIFF pixels match that mainline baseline exactly (maximum absolute difference 0). Native Linux GUI smoke covered the extracted Film/Print panels, preset-to-Custom controls, preview rendering, and the retained export-backend selector. No new tests were added for directory structure or forwarding. Hardware GPU execution and performance were not verified by this refactor's acceptance run.

## Credits

Original Python implementation by Andrea Volpato — [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm). All spectral data, film/paper profiles, and pipeline architecture come from there. This port owes its existence to Andrea and its work.

The spectral-upsampling LUT (`hanatos2025_*`) is named after [Johannes Hanatos](https://github.com/hanatos), author of [vkdt](https://github.com/hanatos/vkdt), who provided the upstream Python project with the LUT files and sample code that drive the RGB → spectrum step.

PCHIP 3D LUT, MT19937 binomial sampler, and CIE 1931 observer constants are ported from numpy / scipy / scikit-image / colour-science.

Claude code for being this awesome.

## License

The Rust program is GPL-3.0; see [LICENSE](LICENSE). Andrea Volpato's spectral profiles and their LUT derivatives retain CC BY-SA 4.0; see [the canonical asset license](data/license/SPEKTRAFILM_LICENSE.txt).
