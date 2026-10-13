# spektrafilm-rs

![banner](docs/card.jpg)

A Rust port of [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm) — a spectral simulator for analogue colour film and the print-and-scan chain.

The spectral chain (RGB → film dye density → enlarger illuminant → print paper → scanner RGB) is migrated against pinned Python spektrafilm 0.3.4 commit `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`. The GUI uses its native-precision Runtime for Preview, Scan and Export; GPU arithmetic is f32, with CPU routing for effects without a faithful GPU implementation. The separate `spektrafilm-f64` CLI remains the CPU f64 reference. Numeric agreement is assessed with per-scenario budgets, rather than universal bit identity.

---

## What it does

- **Spectral pipeline.** Hanatos2025 RGB→raw spectral upsampling with its full sensitivity adaptation (camera UV/IR band-pass filters with reference-illuminant normalization, erf4 band-pass window, poly4 log-exposure surface, spectral Gaussian blur), full 81-wavelength film/print/scanner spectral integration, density-curve interpolation, halation, DIR couplers, grain (bit-exact numpy `MT19937` port), glare, output CCTF encoding.
- **Interactive preview** through wgpu (Metal on macOS), with CPU stages for exact optical diffusion and Spektra grain sampling; Procedural uses a compute shader. Frame rate depends on image size, controls and hardware.
- **Reference export** on the CPU at f64. Historical bare-chain evidence and applicable comparison budgets are recorded in [baseline evidence](docs/parity/baseline_evidence.md); fresh integrated comparisons are required for migration acceptance.
- **Independent GUI export.** **Export…** opens its own settings dialog, then a file chooser, and renders an immutable snapshot of the full input and simulation parameters through the GUI's Runtime worker. CPU uses the GUI build's native precision; GPU uses WGPU f32 and faithful CPU stages where required. GUI Export does not launch `spektrafilm-f64`; use that CLI separately for CPU f64 reference output. Cancellation discards work at the next boundary before atomic publication and may wait for an active computation.
- **Profiles bundled.** 30+ film and paper profiles in `data/profiles/` — Kodak Gold/Portra/Ektar, Fuji Velvia/Provia, Kodak Endura papers, Fuji Crystal Archive papers.
- **Experimental workflow routes.** Runtime/GUI state accepts passthrough, film-scan, film-print-scan, and the three convert-film routes. Direct negative-film scans expose `scanner.scan_output=direct_scan|positive_scan`; calibrated `positive_scan` is supported for direct `input > film > scan` and real negative-capture `input > convert-film > scan`, while paper, positive-film, print-paper, scan-minus-base, white/black-corrected, and active-Grain-Procedural combinations remain rejected. Convert-film inverts the spectral scan model with bounded Gauss-Newton, supports scan illuminant/exposure/calibration controls, and can reuse the calibrated positive interpretation on its direct scan route.
- **Magazine print color.** The optional finishing treatment applies a bundled RGB appearance to any supported base workflow — a scanned negative or a finished RGB input — after the base workflow and its scanner-domain effects: warm paper/highlights, compressed print contrast, cyan-biased shadows and smooth chroma compression. `magazine_print_color.active` and one appearance Strength from 0 to 1 enable it independently of the base workflow; there is no magazine route. It remains RGB-only and does not generate CMYK or halftone output.
- **Camera taking filters.** The measured Hoya X0, X1, Y2, YA3 and R1 transmission curves are selectable in runtime params and the GUI; changing the filter invalidates the sensitivity-dependent spectral cache.
- **Selectable grain engines.** Spektra is the default upstream experimental original. Procedural is derived from Dehancer and provides Analogue/Noise grain with twelve format/speed profiles, Size, Amount, Shadows, Midtones, Highlights, Chroma and Film Resolution controls; pixel parity with Dehancer is not claimed. Dye is a self-developed experimental film-coordinate dye field, opt-in, controlled by `v3_dye_support_um` and `v3_event_area_um2`. Fotufilm Film is an adapted appearance population model fitted to resolved film curves, opt-in, controlled by `film_render.grain.fotufilm.*`. It resolves on the CPU, prepares its population before the first readout of a given population identity, and reuses the prepared bank for later renders of that identity.
- **Grain Dye experiments.** Opt-in formed-cell spectral readout and compensated fixed/two-class/Gamma event-mass comparisons are available through diagnostic environment selectors. See [experiment scope](specs/film-grain/v3/experiments.md); the default field and readout remain unchanged.

`film_render.grain.engine` accepts `spektra`, `procedural`, `dye`, or `fotufilm`. Replace explicit old selectors `v1`, `v2`, and `v3` with these values respectively; old selectors are invalid. Existing `v2_*` and `v3_*` control keys, internal module paths, and diagnostic environment variable names remain unchanged.

## Build

Requires Rust stable (≥ 1.88), a C++17 compiler, pkg-config, OpenImageIO and Exiv2 development libraries. The locked `image` dependency requires Rust 1.88. Image I/O uses the same native libraries as Python 0.3.4, preserving float samples and EXIF/IPTC/XMP. On Debian/Ubuntu install `libopenimageio-dev libexiv2-dev libopenblas-dev`; Linux links the installed OpenBLAS library. On macOS use `brew install openimageio exiv2 pkg-config`. Windows packaging uses a matching MSYS2 UCRT64 native toolchain and pkg-config dependencies. The package scripts collect native runtime libraries; the build fails explicitly when required development libraries are absent.

The default and integration branch is `main-0.3.4`. Open pull requests against it; merging requires the `PR lightweight` check. PR CI runs `just ci-pr` through `just verify-pr`. Pushes to `main-0.3.4` and manual dispatch run `Main full verification` through `just verify`. The [test architecture](eng/testing.md) defines layer responsibilities, test design rules, and the PR cost budget. Main verification retains workspace build artifacts and dependencies in commit-keyed caches for PR reuse. An active main verification is not canceled by a newer push; pending runs coalesce to the newest commit.

Pushes to `main-0.3.4` also run the three-platform release workflow: workspace tests, native builds, package smoke, Linux Python parity, and LUT acceptance. Version tags and manual release dispatch retain this workflow. Release runs use one concurrency group per ref. An active release run is not canceled by a newer commit; pending runs coalesce to the newest commit.

```bash
git clone <this-repo> && cd spektrafilm-rs

# GUI (wgpu/Metal preview, eframe)
cargo build --release -p spektrafilm-gui

# f32 CLI — fast batch processor, defaults to WGPU
cargo build --release -p spektrafilm-cli


# f64 CLI — reference precision (CPU only; WGSL has no f64)
cargo build --release --features precision-f64 -p spektrafilm-cli --bin spektrafilm-f64

# Optional RAW decoding helper using the shared native decoder
cargo build --release -p spektrafilm-cli --bin decode_raw_gui
```

This produces:

- `target/release/spektrafilm-gui` — desktop GUI;
- `target/release/spektrafilm` — fast f32 CLI;
- `target/release/spektrafilm-f64` — CPU reference exporter;
- `target/release/decode_raw_gui` — standalone RAW decoding helper.

RAW decoding requires native **LibRaw ≥ 0.22.0**, **Lensfun**, **Exiv2**, and **GLib** development packages plus a C++17 compiler and `pkg-config`. On Debian/Ubuntu install `libraw-dev liblensfun-dev libexiv2-dev libglib2.0-dev pkg-config`; if the distribution supplies LibRaw 0.21, build the official 0.22.0 release into a local prefix and prepend its `lib/pkgconfig` to `PKG_CONFIG_PATH` (and its `lib` to the runtime library search path). On macOS use `brew install libraw lensfun exiv2 glib pkg-config`. Windows builds need a matching native toolchain and these libraries built for it (for example through MSYS2 UCRT64); set `PKG_CONFIG_PATH` to their `.pc` directories. Cross compilation requires target-specific pkg-config paths and libraries. Missing dependencies or older LibRaw fail the build explicitly; measured Kodak DC50 pixels differ on LibRaw 0.21.5, while 0.22.0 matches the pinned rawpy decoder exactly.

The RAW build links the LibRaw artifact selected by `pkg-config` through a unique build-only name, preserving that selection even when another native dependency adds system library directories first. Packages distribute the library's actual runtime SONAME (for LibRaw 0.22.0 on Linux, `libraw.so.24`), rather than the build-only name.
Image IO additionally requires OpenImageIO and Imath development headers. OIIO installations that forward to external fmt headers also need fmt development headers and `fmt.pc` on `PKG_CONFIG_PATH`; Homebrew supplies this dependency. Windows CI builds checksum-pinned LibRaw 0.22.0 with `scripts/build_windows_libraw.sh` into a separate UCRT64 prefix and packages its DLLs and licenses alongside system native dependencies.


Distribute the native shared libraries and their transitive dependencies with the CLI, GUI, and `decode_raw_gui`, together with the Lensfun XML database. Preserve Lensfun's default database lookup location or deploy it in the platform's standard data directory. LibRaw is dual LGPL 2.1/CDDL 1.0, Lensfun's library is LGPL 3.0 (database CC BY-SA 3.0), Exiv2 is GPL 2.0 or later, and GLib is LGPL 2.1 or later; include applicable license texts when packaging. The application remains GPL 3.0.

On Linux x64, install `patchelf libxkbcommon-x11-0` and run `scripts/package_linux_app.sh` after `just build`. This builds the CLI, f64 exporter, GUI, RAW helper, and both native benchmark tools. `BUILD_TARGET_DIR` selects Cargo's output directory and `DIST_DIR` selects the generated package directory. The script copies recursive ELF dependencies, LittleCMS, the GUI's dynamically loaded xkbcommon providers, profiles/ICC files, Lensfun XML, and native license notices. It verifies loader resolution and relative `$ORIGIN` library paths before creating a `.tar.gz`. Extract the entire archive and launch the wrappers in `bin/`; they locate bundled data and libraries relative to their own location. The host supplies glibc, the display server, and graphics drivers. Windows and macOS use `scripts/package_windows_app.ps1` and `scripts/package_macos_app.sh`.

The release workflow runs workspace tests with `precision-f64` and `scripts/parity/package_smoke.py` on each platform before uploading the package. The smoke runner requires installed CLI, f64 exporter, RAW helper and data paths. It exercises generated float/integer files, retained EXIF/IPTC/XMP/ICC, invalid-input/depth errors, a SHA-verified Kodak RAW fixture, and delivered ZIP LUT/OCIO/QA artifacts. With `--gui`, the external native driver operates the real window and file dialogs, checks state restoration and viewer behavior, compares Save/Export output, and exercises Cancel/close cleanup. Linux uses Xvfb; Windows/macOS require an interactive desktop, and macOS requires Accessibility/Automation/Screen Recording authorization. Driver and OCR dependencies are listed in [scripts/parity/README.md](scripts/parity/README.md). CI retains screenshots, observations and command logs, including failed runs.
The release workflow intentionally invokes package smoke without `--gui`.
Therefore its package status covers CLI/RAW/IO/LUT/OCIO/QA and provenance, not
native desktop interaction. GUI evidence is maintained separately and is not
silently counted as a release pass.

Package smoke also runs the installed native benchmark tools from outside the checkout. It generates fixtures, renders a color chart, compares the retained run with itself, verifies exact-zero numeric/color/FLIP measurements, validates manifests, and creates the offline report. It checks the bundled FLIP and LittleCMS licenses and records both benchmark executable hashes.

The workspace tests include headless GUI acceptance with the real App, CPU
workers, and image writer. Run `just gui-headless` for the focused scenarios.
Run `just gui-layout` separately for offscreen layout checks with a usable WGPU
adapter. See [GUI acceptance scopes](scripts/parity/README.md#headless-gui-acceptance)
for coverage, image artifacts, and the native desktop gate retained for platform
integration and release evidence.

Historical Linux native GUI acceptance used the debug GUI and a separate f64 export child under Xvfb. Its five-tab shell, viewer, state, RAW and output records remain under `gui_evidence` in `docs/parity/parity_matrix.json`, including local Rust `8bdd7fe` and CI workflow `37226896580`. Those records describe the earlier worker and viewport implementation and do not establish acceptance of the current direct Pipeline Export, extension-based Save or full-resolution viewport. Historical Windows package smoke also passed; historical macOS native acceptance failed the Reveal-frame assertion. Fresh GUI results must identify the current executable and exercised workflow.

Quarter-turn actions rotate the in-memory pipeline input and preserve source metadata. Export captures those pixels directly together with the simulation parameters; it does not stage input for a CLI child. The metadata writer normalizes orientation and dimensions. Cancel sets a worker signal: an active Pipeline computation or native write may finish before the next cancellation boundary, where its result is discarded before publication. Closing the GUI signals cancellation and joins the worker.

`spektrafilm-raw` returns linear ACES2065-1 pixels after LibRaw camera orientation, optional Lensfun correction, and white balance. Camera WB (`as-shot`) is the default; `daylight` uses LibRaw's daylight base, while `tungsten` and `custom` adapt that base using the reference CIE D/Kang 2002 whitepoints and Von Kries/CAT02 transform. Custom temperature must be finite in 1667–25000 K and custom tint finite; absent custom temperature is an error. Lensfun applies vignetting before combined channel geometry resampling with bilinear interpolation and nearest boundary extension, and preserves unchanged pixels/empty lens summary when camera or lens matching fails.

The shared helper accepts `decode_raw_gui input.CR2 output.tif --raw-white-balance custom --raw-temperature 5000 --raw-tint 1.05 --lens-correction` and writes **linear ACES2065-1 float TIFF**, suitable for pipeline input configured as ACES2065-1 with input CCTF decoding disabled. It no longer writes linear sRGB through rawler.

Real RAW provenance is retained in [`scripts/parity/raw_fixtures.json`](scripts/parity/raw_fixtures.json); downloaded images are not redistributed. Run `scripts/parity/raw_reference_compare.py input.KDC --decoder target/release/decode_raw_gui --source-url URL --max-error 0 --mean-error 0` in the pinned Python environment for fresh four-mode decoder/WB evidence. The exercised Kodak DC50 KDC decodes to 768×512 and matches LibRaw 0.22.0/rawpy 0.26.1 at zero max/mean error for all four WB modes. Its EXIF has no lens model, exercising unchanged no-match correction; a separately injected Canon EOS 5D Mark II / EF 24–70mm f/2.8L / 35mm f/8 EXIF fixture matches upstream Lensfun pixels and summary exactly at 64×48 and 640×480. A public `M0054341_01_00005.cr2` fixture is intrinsically damaged (verified Git blob), and both paths reject it with a data error. These measurements do not imply support for every recognised RAW suffix.

The public Canon EOS 40D sRAW CR2 fixture also matches all four modes at zero max/mean error on 1944×1296 pixels, with missing-lens correction enabled. Unlike Kodak's unit camera multipliers, this camera's recorded WB `[2341,1024,1570,1024]` exercises distinct camera and daylight output (max change 0.212146, mean change 0.007560). Run `scripts/parity/lens_reference_compare.py` for the known-lens, missing-metadata, missing-camera and missing-lens comparisons; it uses actual Exiv2 metadata and native Lensfun, requires all dependencies, and defaults to exact pixel/summary equality.

The rebuilt f64 CLI and its relocated Linux archive also passed eight fresh RAW comparisons against the pinned Python loader: all four white-balance modes on Kodak 768×512 with missing-lens correction, and Canon 1944×1296 with injected known-lens EXIF. The unbounded `rgb_in` boundary matched exactly (maximum and mean absolute error zero). The relocated RAW helper independently matched Kodak pixels exactly under a clean environment. The earlier packaged GUI launched under Xvfb and its prepared-image export preserved EXIF/IPTC/XMP and ICC bytes; this is historical evidence for the earlier export implementation.

WGPU/WGSL is the only GPU backend and can be selected explicitly with `SPEKTRAFILM_BACKEND=wgpu`; CPU is the fallback when no usable adapter is available. GUI Preview and Scan prefer GPU execution with faithful CPU stages for unsupported effects. GUI CPU execution uses the build's native precision; the separate `spektrafilm-f64` CLI remains the CPU f64 reference. Linux GPU acceptance requires the Mesa software Vulkan adapter (`mesa-vulkan-drivers`).

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

Run `just verify /absolute/path/outside/the/repo/new-directory` to retain gate evidence. The directory must not exist. The runner displays and saves combined output in `gate.log`, preserves the gate's failure exit status, and writes `report.json` on success or failure. The report records commit and tree IDs, tracked changes in `before.diff` and `after.diff`, hashes of untracked files, tool versions, timestamps, and exit status. A source change during verification fails the run. The evidence covers `just ci`; native GUI, image, CLI, and package smoke evidence remains separate. Call this entry point directly so an outer shell pipeline cannot mask its exit status.

Use `just verify-pr /absolute/path/outside/the/repo/new-directory` for the fast PR gate. Its report records `just ci-pr` and uses the same source and exit-status checks as full verification. Test selection lives in the Justfile; admission rules live in the [test architecture](eng/testing.md).

`just ci` first runs `just check-python-cache`. This check rejects Git-index paths with a `__pycache__` component or a `.pyc` or `.pyo` suffix, including files added with `git add -f`. Python caches remain ignored local outputs.

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

- **Magazine print** — the checkbox next to the workflow selector applies the bundled appearance to the selected workflow; **ADVANCED** → **Magazine print color** sets Strength from 0 to 1. Strength 0 is a no-op. Photographic print controls remain separate. The `Finished RGB` workflow colour-manages an already rendered image to the output space; the film and print profile selections do not affect it.
- **Open…** — load standard images or camera RAW. RAW processing uses LibRaw and exposes as-shot/daylight/tungsten/custom Kelvin+tint white balance and Lensfun correction; RAW enters the runtime as linear ACES2065-1.
- **Input image / Profiles** — choose input/output color workflow, film stock and print paper. Picking a film auto-selects its paired paper (`target_print` in the profile).
- **Numeric fields** — click to type exposure, film format, halation, DIR couplers, grain, glare, scanner, enlarger and output values. The fields use the text cursor. Scrolling over a field changes its value by the control's step; scrolling elsewhere moves the sidebar. Valid edits follow Auto preview. Incomplete or invalid text keeps the last valid parameter value and resets when focus leaves the field.
- **Viewer controls** — `ccw rotate` and `cw rotate` physically rotate the in-memory input used by Preview, Scan and Export; Save writes the retained output of the latest render. `100%`, `200%` and `400%` map image pixels to exact device-pixel percentages; reset view returns to fit. Full-resolution Scan remains available in the viewport at these zoom levels instead of being capped by the preview long-edge limit.
- **Export…** — open independent output settings, then choose a destination. The worker renders an immutable snapshot of the full input and simulation parameters through the same GUI Runtime, independently of retained preview output. CPU uses native GUI precision; GPU uses WGPU f32 with faithful CPU stages where needed. JPEG exposes quality (1–100) and 4:4:4/4:2:0 chroma sampling, with a warning that JPEG remains lossy even at quality 100. TIFF/EXR expose their supported depth/compression settings. Export's saving color space and transfer encoding are independent of Save's current simulation saving settings. The status bar shows elapsed time. **Cancel** signals the worker and discards its result before publication at the next boundary; it may wait for active computation. Closing the GUI cancels and joins the worker.
- **Save…** — save the latest retained floating output without rerendering, using the current simulation saving color space and transfer encoding. The filename extension selects the format, with `.jpg` as the default: JPEG/PNG use 8-bit integer samples, TIFF uses 16-bit integer samples with ZIP compression, and OpenEXR uses 16-bit half samples. Save uses upstream OpenImageIO JPEG defaults rather than Export's quality/subsampling settings. Neither TIFF nor EXR forces ACES or linear encoding. Viewer borders, watermark and display ICC transforms stay out of saved pixels.
- **Save state… / Load state…** — exchange the pinned Python 0.3.4 GUI JSON sections. Partial files merge into factory values; legacy input aliases and nested sections normalize to the flat upstream format. Invalid JSON, field types, selections or Rust extension versions appear in the status bar.
- **Save current as default / Restore factory default** — persist the current controls, or restore Kodak Gold 200 + Kodak Supra Endura, RMS granularity `[5,5,5]` and upscale factor `1.0`. Saved user values, including zero RMS, survive startup and state loading. Startup files use the platform configuration directory (`SPEKTRAFILM_CONFIG_DIR` overrides it); Rust settings use `rust.version = 1`. File-dialog directories persist separately.
- **Preview** — render a reduced image using the configured preview long-edge limit and preview digestion; explicitly update it when auto-preview is disabled. Crop, spectral adaptation/blur, UV/IR, layered grain and both diffusion filters apply to the runtime.
  Preview calculation and viewer composition run in background workers. New edits replace queued work, and stale results cannot replace the latest output. The viewer retains its last frame until the current frame is ready. Automatic parameter updates skip development animations; explicit Preview and Scan retain the configured reveal or crossfade.
- **Scan** — render the full-resolution input with spatial and stochastic effects, preferring GPU execution with CPU fallback. The resulting full-resolution output is retained for Save and inspection in the viewport.
- **Scan-for-print** — temporarily enable scanner white/black corrections and disable print glare. Toggle again to restore all three previous values. Loading state or changing profiles clears the transient snapshot.
- **Viewer** — switch Input / Output / Paper back; choose nearest, linear, cubic, spline16, spline36, Lanczos or Blackman sampling for Output. Input retains the upstream nearest interpolation; Paper back uses spline36. The viewport samples the retained full-resolution image for visible detail; the preview long-edge limit controls Preview rendering, not Scan's available detail. The canvas is the pinned 18% gray (`#767676`) or black, with normalized white padding and the upstream paper watermark. Reveal and crossfade affect the disposable viewing frame; hovering reports the original floating RGB pixel.
- **Display transform / Display ICC…** — when enabled, every platform converts output-space pixels to encoded sRGB for the disposable viewer raster; Save and Export use their respective saving color and encoding settings. Windows discovers the primary display ICC profile, and an explicitly selected ICC profile is applied on every platform that supports LittleCMS. On macOS the Metal surface is tagged sRGB; without an ICC profile the encoded sRGB preview is used directly.
- **Launch state** — `spektrafilm-gui IMAGE --state GUI_STATE.json` loads a deterministic state for repeatable sessions. `SPEKTRAFILM_GUI_STATE` provides the same startup override.
- **Diagnostics** — CONFIG selects Off, Summary, or GPU timing for subsequent operations. Changing the mode leaves running operations on their captured mode. The session keeps the latest 20 collected operation reports. Select a report to inspect its phases, stages, GPU work, timing availability, and outcome. **Save report** writes local JSON without rendering. Save reports identify the retained render that supplied their pixels, including renders made with collection off or removed from history. Launching with `SPEKTRAFILM_GUI_DIAGNOSTICS_DIR` set to an existing directory publishes every completed operation's report there as a new `operation-<id>.json` and selects summary collection for the session, the way a CLI report request does.

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
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --format jpeg --bit-depth 8 --jpeg-quality 95 --jpeg-subsampling 444 \
    --backend cpu --film kodak_gold_200 --scan-film --data-dir data


# Positive scanner interpretation with explicit receiver, inverse, setup and finish
./target/release/spektrafilm-f64 process input.tif -o positive.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend cpu --film kodak_portra_400 --scan-film \
    --set scanner.receiver_profile=lab_scan_approx,scanner.positive_interpretation=coupled \
    --set scanner.auto_setup=per_frame,scanner.finish=lab_scan,scanner.finish_strength=0.75 \
    --scan-output positive_scan --data-dir data

# Inspect effective scanner selections without decoding or writing output
./target/release/spektrafilm process input.tif -o positive.tif --film kodak_portra_400 --scan-film \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --scan-output positive_scan --set scanner.receiver_profile=lab_scan_approx,scanner.finish=lab_scan --dry-run --data-dir data

# Override any params via JSON (matches RuntimeParams struct)
... --params my_params.json

# List available film + paper profiles
./target/release/spektrafilm list-profiles --data-dir data
```

`cinestill_800t_cs41_approx` is an experimental approximation of CineStill 800T in Cs41. It fits the manufacturer's [RGB sensitometry chart](https://cinestillfilm.com/blogs/news/what-makes-800t-the-original-and-only-true-800-speed-tungsten-balanced-film-for-still-photography) through the inherited Vision3 500T dye spectra and a fitted base-density correction. The [official Cs2 page](https://cinestillfilm.com/products/cs2-cine-simplified-ecn-2-bath-kit-low-contrast-motion-picture-color-negatives-for-ecp-scanning) identifies the source experiment as 21-step sensitometry. The chart specifies 3200 K tungsten, 1/50 s and a log-exposure reference of -1.535. Its density measurement standard and actual development time and temperature are unknown; Status M is a fitting assumption.

The material profile computes developed dye amounts and spectral density before printing or scanning. It does not provide measured CineStill spectral sensitivity, dye spectra, grain or halation. Its fitted CDF components are mathematical terms, not measured emulsion layers. Keep grain disabled for material comparisons, and compare halation separately. The chart reference of -1.535 defines the chosen model exposure origin; it does not establish absolute ISO or prove that the manufacturer's reference is an 18% gray exposure. Metadata records the sources, approximations and attribution. The derivative retains Andrea Volpato's CC BY-SA 4.0 asset license.

The built-in `CineStill 800T Cs41 (experimental)` look fits halation and Grain Dye controls to public lab scans of CineStill 800T from [PhotonFolly](https://photonfolly.com/analog/examples/Cinestill_800T/Cinestill_800T.html). Halation uses RGB strengths `[0.15, 0.045, 0.005]`, a 160-micrometer first sigma in all channels, and three bounces. Around lamps in the print route, this setting places the red-excess peak at 150–280 micrometers and its half-height at 330–470 micrometers. The three reviewed scans measure 240–370 and 270–530 micrometers. The previous PIXLS-derived 65-micrometer kernel peaked at 120–170 micrometers. The real lamps have unknown overexposure, and the print compresses their highlights, so the fit uses profile shape and red-to-green hue rather than peak height. Grain uses `rms_granularity = [13.1, 19.2, 24.4]` with `particle_scale_sublayers = [4.0, 0.15, 0.03]`. On seven tone-matched flat windows at the scans' film pitch, mean high-passed RGB grain RMS is within 5% of the scans, and the simulated-to-real ratio has no luminance trend. Simulated grain remains finer, with a half-autocorrelation radius of 6.1–6.2 micrometers versus 6.9–7.7. It is also more chromatic, with a chroma-to-luma ratio of 1.29–1.42 versus 0.97–1.15. None of these values is a measured CineStill PSF or granularity. Grain Dye requires a 36 mm frame, so the look keeps grain off; enable it with `camera.film_format_mm=36,film_render.grain.active=true`. Scattering, DIR couplers, glare, sharpening, and highlight boost are disabled. The look preserves the caller's camera exposure and input color interpretation. Its optional Portra Endura settings balance synthetic linear-sRGB midgray; they do not change the film material. Select the workflow separately.

The film chain normalizes spectral upsampling to the profile's reference illuminant, so a neutral input pixel means white under that light. For this tungsten profile, that light is `T` (CIE xy 0.4507, 0.4081; about 2812 K). An as-shot white-balanced input therefore hides the film's color balance. To evaluate tungsten balance, adapt the input from its capture light to `T`: decode RAW with `--raw-white-balance custom --raw-temperature 2812`; adapt rendered input with CAT02 from its known capture white. Daylight scenes then render blue, and lamps warmer than `T` render orange.

```bash
./target/release/spektrafilm-f64 process input.tif -o cinestill.png \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --preset 'CineStill 800T Cs41 (experimental)' --backend cpu --data-dir data \
    --route 'input > film > scan' --scan-output positive_scan
```

This command bypasses printing. `positive_scan` interprets the spectral scanner capture with the selected film profile's calibrated density response and a print-like logistic tone curve. The response is profile-derived, monotone, and extrapolates beyond calibration endpoints with finite endpoint slopes, so scanner-domain RGB can remain negative or above the clear reference until the normal output writer. `scanner.positive_density_ev` controls brightness in stops; `scanner.positive_balance_ev` supplies independent RGB exposure offsets; `scanner.positive_contrast` and `scanner.positive_dmax` control the tone curve. These controls apply to direct negative-film scans and the `input > convert-film > scan` route; print-paper and scan-minus-base routes retain their existing route semantics. The displayed result is an interpretation of the scanner capture, not a measurement of the film or a match guarantee for a processed reference JPEG. Hold profile, exposure, white balance, scan illuminant and scan settings fixed when comparing results.
For a real negative scanner capture on `input > convert-film > scan`, disable virtual camera metering (`camera.auto_exposure=false`) and supply input transfer/color interpretation and the matching `film_render.convert.scan_illuminant`; the positive interpreter assumes the incoming pixels already represent scanner capture space.

Film scans expose per-channel `scanner.black_point` and `scanner.white_point` for slide film and positive-interpreted negatives. Defaults `[0,0,0]` and `[1,1,1]` preserve the existing rendering. Each channel requires `0 <= black_point < white_point <= 1`. Points stretch the shared relative log-exposure range for negatives or the shared scanned-density range for slides; values outside the points continue the same map without clipping. On slides, non-default points run after scanner lens blur and before white/black correction without changing that correction's references. Paper scans and direct negative captures ignore these points. See [film-scan points](specs/film-chain/spec.md#film-scan-black-and-white-points) for the range definitions.

```bash
./target/release/spektrafilm-f64 process input.tif -o slide.tif \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --film fujifilm_provia_100f --route 'input > film > scan' --backend cpu \
    --set 'scanner.black_point=[0.05,0.05,0.05],scanner.white_point=[0.95,0.95,0.95]' \
    --data-dir data
```

The GUI shows scanner controls for the scanned medium: white/black correction for paper and slides, scan polarity and positive interpretation for negatives, and film-scan points for slides and positive-interpreted negatives. Selecting positive interpretation clears incompatible capture-correction flags. State files and scanner section presets retain point values.

The TIFF examples above use prepared linear ProPhoto RGB samples. The PNG examples below use encoded sRGB samples. Declare the actual source interpretation for your own files; see [source declarations](specs/cli-parameters/spec.md#input-declaration). RAW examples retain automatic linear ACES2065-1 declaration.

`process --backend cpu|gpu` overrides `SPEKTRAFILM_BACKEND`; omitting it preserves the environment/default selection. `--format` must match the output extension when supplied; otherwise the extension selects JPEG/PNG/TIFF/EXR. Bit depth defaults to 8 for JPEG/PNG and 16 for TIFF/EXR; JPEG/PNG require 8-bit, EXR requires 16- or 32-bit. JPEG defaults to quality 95 and 4:4:4; `--compression zip|none` applies to TIFF/EXR, while JPEG/PNG reject compression options. CPU precision follows the executable build: use `spektrafilm-f64` for reference exports. Output bit depth is independent of computation precision. GPU selection does not disable grain, optical effects or requested spectral LUTs to force acceleration, and software Vulkan adapters can also execute the WGPU path; speed depends on the adapter and active effects.

`process` and `render` accept `--diagnostics REPORT.json`. Add `--gpu-timings` to collect compute-pass timestamps when the adapter supports them. The report includes operation outcomes, repeated simulation attempts, saving attribution, actual GPU work, and explicit availability reasons. Diagnostics do not change image parameters or machine-readable render output. Report-write errors are reported separately from the image result. CLI reports never replace an existing file. Report paths must not alias input, output, or configuration files.

```bash
./target/release/spektrafilm process input.tif -o out.png \
    --film kodak_portra_400 --paper kodak_portra_endura \
    --input-color-space 'ProPhoto RGB' --input-cctf-decoding false \
    --backend gpu --diagnostics report.json --gpu-timings --data-dir data
```

See [the telemetry contract](specs/telemetry/spec.md) and [report metrics](specs/telemetry/metrics.md) for report semantics, bounded detail, and measurement limits.

`process` accepts an exact preset name or a TOML/JSON preset file through `--preset`. Repeat `--preset` to apply documents in appearance order: a look selects profiles and replaces look controls, then a section edits only its supplied controls. Repeat `--set` to apply sparse TOML/JSON files or comma-separated `PATH=VALUE` assignments after the presets. String and enum values need no JSON quotes: use `--set film_render.grain.engine=procedural` or `--set 'io.output_color_space=ProPhoto RGB'`. Use `true`/`false` for booleans, numbers for numeric controls, `[1,2,3]` for arrays, and `null` to clear optional controls. The [CLI parameter language](specs/cli-parameters/spec.md) defines editable paths and composition.

Preset names are their portable identities. Core reads schema v1 documents by discarding their old `id` and exports schema v2. Built-ins are compiled from `crates/spektrafilm-core/assets/presets`; user documents live in `<config_dir>/presets` under the shared `SPEKTRAFILM_CONFIG_DIR` policy. A name resolves in My Presets first, then Built-in; `user:` and `built-in:` select one group. Section documents contain sparse editable leaves from exactly one look section and retain omitted controls on application. Explicit JSON null edits require JSON export; TOML cannot represent those edits.

The GUI remembers the preset browser selection and layout independently of startup controls. An ordinary restart restores that metadata and recomputes the applied, modified, or missing marker; it does not apply the preset again. Loading an explicit GUI state uses that file's browser metadata. Use **Save current as default** to change startup controls. A failed preset application preserves the current controls and temporary black/white correction state.

Section files can be saved and exported without an active workflow. Their application validates the current scan configuration; CLI composition validates after explicit workflow choices. Library writes reject names already owned by another document, including concurrent writes with different letter case or carriers. Export into My Presets creates a new name; use Update to replace an existing document.

```bash
# Discover portable presets and editable grain controls
./target/release/spektrafilm preset list --source all --format text
./target/release/spektrafilm preset show 'Kodak Portra 400' --data-dir data
./target/release/spektrafilm describe --module film_render.grain --format json

# Inspect the effective configuration without decoding pixels or creating output
./target/release/spektrafilm process input.png -o output.tif \
    --input-color-space sRGB --input-cctf-decoding true \
    --preset 'Kodak Portra 400' \
    --preset 'Soft grain' \
    --set film_render.grain.engine=procedural \
    --set 'camera.exposure_compensation_ev=0.5,film_render.grain.v2_amount=25' \
    --dry-run --data-dir data

# Apply a sparse file, then an inline edit, with independent writer settings
./target/release/spektrafilm-f64 process input.png -o output.tif \
    --input-color-space sRGB --input-cctf-decoding true \
    --preset 'Kodak Portra 400' --set adjustments.toml \
    --set 'enlarger.print_exposure=1.2' --backend cpu \
    --format tiff --bit-depth 16 --compression zip \
    --saving-color-space 'ProPhoto RGB' --saving-cctf-encoding true --data-dir data
```

`--dry-run` prints effective runtime controls, writer options, profile names, and the requested backend policy. It checks the input path and supported file kind. Image decoding and compute-adapter availability remain unchecked. Saving color flags control writer conversion independently of scanner `io.output_color_space` and `io.output_cctf_encoding`.

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
Image output uses one writer for CLI, GUI Save and GUI Export. CLI `process --format jpeg|png|tiff|exr` selects the container; `--bit-depth 8|16|32` selects uint8/uint16/float32 TIFF or half/float32 EXR, with JPEG/PNG restricted to uint8 and EXR rejecting 8-bit. Explicit JPEG quality is 1–100 and chroma sampling is 4:4:4 or 4:2:0; TIFF compression accepts ZIP/none and EXR uses ZIP. Integer output clips to [0,1], scales and truncates; float TIFF/EXR preserves negative and super-white samples. The writer's color space/CCTF describes the pixels it receives and controls ICC/color tags; it applies no extra transfer function and does not force ACES or linear encoding for EXR. GUI Save converts retained output using the current simulation saving settings; GUI Export uses its independent saving settings. Matching profiles are compiled from `data/icc`, including encoded/linear variants; linear P3 has no bundled upstream profile. Source EXIF/IPTC/XMP is copied for non-EXR output, with orientation, dimensions, software, date and color tags refreshed.
Concurrent metadata operations initialize Exiv2's XMP toolkit once and protect namespace registration with its locking callback. Timestamp conversion uses thread-safe platform functions.
The single default-collapsed **MAIN → Output → Export options** area opens the independent settings dialog or cancels its active worker. Import RGB/Raw contain only import controls; bottom SAVE uses the chosen filename extension independently of Export preferences.
Legacy states without an explicit export format preserve 8-bit as PNG and 16/32-bit as TIFF before defaults and validation. Recipe values and viewer state remain intact; explicitly incompatible format/depth combinations remain errors.

Explicit JPEG Export quality uses OpenImageIO's `CompressionQuality` attribute, and Export forwards the chosen 4:4:4 or 4:2:0 chroma subsampling. Save leaves JPEG quality/subsampling to upstream OpenImageIO defaults. JPEG remains lossy even at quality 100, and 4:2:0 can soften colored grain. Use TIFF or EXR when comparing grain detail without JPEG compression; quality 100 and 4:4:4 do not make JPEG lossless.

`image_io::convert_image` explicitly converts retained output into a different saving space/CCTF: source decoding, CAT02 white adaptation, destination encoding. It reuses the shared seven-space color math and preserves floating headroom. Apply this operation before `save` when saving settings differ from simulation settings; writer options describe the resulting pixels and never change the simulation gamut mapping.
Current LUT acceptance uses experimental spectral defaults from `28bf883e1672e884307edc75852549376e13644e`. Hanatos input compression uses fixed D65 independently of the film adaptation white; an independent saturated F-Log regression covers that distinction. Historical `3bb2c2d` evidence does not validate current binaries.
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
SPEKTRAFILM_BACKEND=cpu ./target/release/spektrafilm-f64 lut build build/lut_bundles \
    --film kodak_portra_400 --print kodak_portra_endura \
    --input vlog --output srgb --topology 4lut --resolution 33 \
    --combinations --data-dir data
./target/release/spektrafilm lut build build/lut_bundles --from bundle.toml \
    --resolution 65 --data-dir data
```
Data-consuming commands select `--data-dir`, then `SPEKTRAFILM_DATA_DIR`, then
automatic executable-relative or working-directory data. Explicit selections
must exist and never fall back. Help, version, field discovery, and preset
listing without an explicit selection need no data. See the
[CLI workflow contract](specs/cli-workflow/spec.md#data-directory-selection).
Profile metadata and array dimensions are validated before construction;
present but malformed neutral-filter JSON is an error.

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

### Appearance benchmark

The external `spektrafilm-bench` CLI renders immutable evidence, compares versions, and creates an offline review. It reports color, tone, official LDR-FLIP, grain, halation, edge response, and numerical health by case and region. It preserves floating masters, full-resolution maps, curves, spectra, and per-seed observations. The [benchmark specification](specs/benchmark/README.md) defines reference scopes and qualification rules.

Build the native benchmark tools. Generate fixtures outside the repository. The optional pinned Kodak DC50 RAW has its own attribution and license in the generated `provenance.json`; the repository does not distribute that original.

```bash
just build-f64
just build-benchmark
./target/release/spektrafilm-bench-fixtures \
    --renderer ./target/release/spektrafilm-f64 --data-dir ./data \
    --out /tmp/spektrafilm-benchmark-fixtures --download-natural

./target/release/spektrafilm-bench validate --kind suite \
    --file /tmp/spektrafilm-benchmark-fixtures/complete.json
./target/release/spektrafilm-bench run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/baseline.json \
    --out /tmp/spektrafilm-benchmark-baseline
./target/release/spektrafilm-bench run \
    --suite /tmp/spektrafilm-benchmark-fixtures/complete.json \
    --target /tmp/spektrafilm-benchmark-fixtures/candidate.json \
    --out /tmp/spektrafilm-benchmark-candidate
./target/release/spektrafilm-bench compare \
    --baseline /tmp/spektrafilm-benchmark-baseline \
    --candidate /tmp/spektrafilm-benchmark-candidate \
    --out /tmp/spektrafilm-benchmark-comparison
./target/release/spektrafilm-bench report \
    --comparison /tmp/spektrafilm-benchmark-comparison \
    --out /tmp/spektrafilm-benchmark-review
just benchmark-test
```

`just benchmark-test` includes public CLI regressions for failed renders, nonfinite masters, evidence integrity, mask populations, RAW declaration applicability, policy qualification, and measurement availability. These tests use isolated retained artifacts and do not require network access or an external renderer.

Open `/tmp/spektrafilm-benchmark-review/index.html`. The human review opens one case and seed at a time, prioritizes `color_chart_30mp` when present, and keeps every case, seed, failure, and artifact reachable through the selector and folded evidence. The first view shows synchronized baseline/candidate imagery, a short retained condition summary, and a small rounded full-image metric summary; sources, references, difference maps, curves, full measurements, execution settings, findings, and detail regions stay collapsed until needed. Fixed regions, automatic hotspots, lossless crops, synchronized pan and zoom, manual A/B alternation, and split view remain available. Detail zoom supports 100%, 200%, and 400% in device pixels. Report generation reads retained evidence and needs neither a renderer nor the original run directories. Reference artifacts with unknown or denied redistribution permission remain omitted and identified in the report manifest.

The viewer opens in split mode: drag the divider on the image, use the range control, or move the focused divider with arrow keys (Shift for larger steps; Home/End for endpoints). Display copies use 16-bit RGB sRGB PNG without dither. Fit views resize floating samples before 16-bit quantization; native 100%, 200%, and 400% views switch to the full-size image, and detail crops preserve its 16-bit samples. Browser and screen precision still limit the displayed result. Preview resampling never feeds measurements.

Each parsed command emits one JSON result on stdout. Diagnostic messages use stderr. Status `2` indicates argument syntax errors; status `1` indicates validation, execution, publication, or requested gate failure. Use `schema --kind suite|target|policy|run|comparison` to discover carriers. `compare --policy FILE --gate` publishes the comparison before returning failure for a non-pass evaluation. The supplied policy is exploratory and cannot qualify a release. A calibrated policy must bind the digest emitted by suite validation and supply reviewed calibration evidence.

The generated quick and complete suites include the natural scene when acquired. Without `--natural-input` or `--download-natural`, they contain controlled probes only. The packaged scene covers shadows and texture; broader skin, foliage, mixed-light, and real film-scan coverage requires explicitly declared assets and regions. Synthetic probes and rendered reference fixtures do not establish physical film fidelity.

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
Grain Procedural float-bit noise hashes require separately rounded binary32 arithmetic. The pinned `vendor/wgpu-hal` patch disables Metal fast math and contraction only for shaders with the `spektrafilm_strict_float` marker; provenance and licenses are in `vendor/wgpu-hal/SPEKTRAFILM-PATCH.txt`.

Recipe `seed` values are admitted in the unsigned 32-bit range (`0..=4_294_967_295`) because the WGPU stochastic path uses 32-bit seed state; silently truncating a caller-provided 64-bit seed would violate deterministic recipe semantics.

The WGPU resident chain returns unclipped linear destination RGB when Grain Procedural is inactive; the shared CPU post-scan stage applies the destination's same-space matrix roundtrip and transfer curve only for encoded output. With Grain Procedural active, the GPU applies that matrix and the selected destination curve before the shared grain preparation/filter/composition passes. The resident result is already encoded; linear exports decode only the transfer on the CPU. This keeps Grain Procedural in one GPU command buffer with one upload and one readback. Input transfer decoding, Spektra grain, active unsupported gamut algorithms/spaces, and camera/enlarger optical diffusion use an explicitly reported faithful per-stage path. Optical diffusion uses the finite sampled, normalized PSF with reflected-boundary FFT convolution on CPU; the discarded Gaussian mixture drifted by **0.08603** on a 48-pixel black-pro-mist hotspot at spatial scale 0.05, exceeding its unchanged **0.005** regression limit.

Requests for enlarger/scanner PCHIP spectral LUTs also select the per-stage path, preserving the requested sampling and interpolation rather than substituting direct resident spectral integration. GPU shader pipeline caches and one-upload/one-readback batching remain active for supported resident configurations.

Gaussian requests exceeding the GPU's 256-pixel FIR half-width explicitly fall back to CPU blur, including resident halation bounce radii, DIR, glare, scanner, and camera blur. Grain Spektra remains on the faithful CPU per-stage path; Grain Procedural uses its resident GPU shader when the rest of the resident-chain requirements are met. The requested radius is preserved rather than silently truncated. After integrating the migration slices, run `cargo run -p spektrafilm-core --example backend_parity --features precision-f64 -- data` for actual WGSL resident/per-stage comparisons across seven destination spaces, encoding modes, geometry, darks/highlights, spectral/optical controls, and faithful fallback cases. It fails on unavailable adapters and prints measured maximum/mean errors against CPU with a provisional 0.005 smoke limit.

Budgets apply to different measurements: reference f64 arithmetic uses max absolute error **1e-6** against the pinned matrix; CPU-f32/GPU arithmetic must report its measured maximum and mean error independently and has no approved universal budget yet. Stochastic appearance uses the provisional **1% mean / 5% standard deviation** relative-error budget rather than per-pixel equality. The spatial **0.005** hotspot limit now guards the faithful diffusion fallback, not an approved GPU blur approximation. End-to-end preview and other spatial budgets in the linked evidence table remain provisional until actual scenario measurements are recorded; adapter initialization or compilation alone does not establish parity.

Grain Procedural is selected with `film_render.grain.engine: "procedural"`. `v2_profile` defaults to `"35mm250"`; presets are `8mm50`, `8mm250`, `8mm500`, `16mm50`, `16mm250`, `16mm500`, `35mm50`, `35mm250`, `35mm500`, `65mm50`, `65mm250` and `65mm500`. Presets supply the grain configuration; Amount remains adjustable through `v2_amount` (0–100). Select `"custom"` to edit `v2_film_type` (`"negative"` or `"positive"`), `v2_mode` (`"analogue"` or `"noise"`), `v2_size` (1–48), `v2_shadows`, `v2_midtones`, `v2_highlights`, `v2_chroma`, and `v2_resolution_factor` (all 0–100). Custom values omitted or null inherit `35mm250`; switching to Custom in the GUI copies the current preset and Amount. Selecting a preset restores its Amount. All numeric controls must be finite and within their stated ranges. Film Resolution 100 preserves detail; lower values increase resolution blur. Film Type selects the reference host's resolution branch. `v2_resolution_type` and nonzero `v2_timer` are explicit runtime controls; legacy `v2_timer: 0.0` uses the recipe seed phase.
`v2_resolution_type` (`0` OpticalResolution, `1` FastBlur) selects Film Resolution for Analogue only. Noise is the single-pass image-resolution path and ignores Film Resolution controls. `v2_timer` is a nonzero explicit phase override; legacy `0.0` derives the phase from the recipe seed.

Procedural consumes and returns native display-encoded RGB without an internal transfer curve or primaries conversion, matching Dehancer's photo `by_pass` contract. The scanner encodes its generated RGB using the selected output color space before Grain; encoded exports keep that result, while linear exports decode only the selected transfer afterward. Grain receives the same encoded image for both export choices. Spektra's density-domain sampling and defaults remain unchanged.

Procedural restores the recovered RGBA content hash, sine permutation, gradient noise, texture sampling and half-storage boundaries. Positive/Negative Film Type remains profile metadata and is separate from `resolution_type`; `resolution_type` selects folded Gaussian FastBlur (`1`) or OpticalResolution (`0`) for Analogue, while Noise is the direct image-resolution path. CPU/WGSL share portable trigonometry, so random realizations can differ from vendor device arithmetic. Amount and tone controls use the reference effective-control polynomial, including its nonzero intercept at zero; disable Grain with `active: false` for an exact bypass. No overscan/damage-mask input or device-specific virtual-texture cap is implemented for the unmasked photo path. See [grain research notes](docs/dehancer-grain-reverse-engineering.md) for measured evidence and coverage limits.
Noise is a single-pass image-resolution path and does not consume Film Resolution output. Analogue additionally samples the original encoded source for its virtual grain texture while composing over the Analogue Film Resolution result. Noise uses the resolution-scaled sampling denominator `(1 + (Size - 1) / 47) * 2.4 * max(width / 1920, height / 1080)` and half the effective Amount before composition. A recipe seed maps through MT19937 to a repeatable phase; Dehancer's global random stream is not a recipe seed contract.

Spektra grain dispatch mirrors upstream `apply_grain`: `sublayers_active` (default, matching 0.3.4) runs the layered model — the composite density is split through `density_curves_layers` into per-sublayer densities, each sublayer samples its own Poisson-binomial particle field with per-layer density maxima/fractions, particle scaling, dye-cloud blur and lognormal micro-structure — while `sublayers_active: false` keeps the single composite-density sampler. `settings.use_fast_stats` switches the layered sampler to the `fast_stats` kernels like upstream; the micro-structure clumping field always uses them (upstream does too) and is pinned to a documented deterministic seed because Python draws it from numba's unseeded thread-global RNG.

Both Spektra composite and layered grain use the CPU sampler during preview and export; no resident WGPU Spektra grain shader is maintained. Active Spektra grain rendering selects the faithful per-stage path. The separate Grain Procedural implementation remains available.

Grain Dye is selected with `film_render.grain.engine: "dye"`. It conditions a mixed film-coordinate dye field on `v3_dye_support_um` (Gaussian sigma in micrometers, default `3.5`; the field-cell pitch is `support / 2`) and realizes film-space Poisson counts with event area `v3_event_area_um2 × k_c × particle_scale_sublayers` (square micrometers, default `1.28`), using the resolved `density_min` and `uniformity` and forming every channel from the film's three sublayers. Both controls must be finite and strictly positive. The channel scale is `k_c = (rms_granularity_c / 6)²`, where `6` is the preset of the reference stock `kodak_portra_800`. It is the only per-channel strength input: Dye does not read Spektra's `particle_area_um2` or `particle_scale`, whose blue-heavy default would double a coarser blue record. The field belongs to film coordinates: the pipeline develops the full frame and the readout produces the requested region at the requested grid, so a crop selects the readout region and never restarts texture. Overlapping full-frame and cropped CMY reads therefore agree within the design's **1e-5** (f32) and **1e-10** (f64) tolerances, while a rounded output grid may change footprints and only that sampling is then comparable. Dye supports still and cine color-negative filming profiles using the global `camera.film_format_mm` exposed-image long edge. This value must be finite and strictly positive. Its existing default is `35.0`; it is not a roll-width designation. The reference validation remains a 36 mm long edge on `input > film > print > scan` with `io.scan_film = false`. Format changes alter physical footprints and workload, while crop/resize determine output dimensions. Support and event area remain in film units. Large requests can fail an explicit resource bound. The diagnostic film-scan route `input > film > scan` with `io.scan_film = true` also runs for `positive_scan` and `direct_scan`. Another chemistry or an unsupported route, fails with an actionable unsupported-condition error. Dye resolves on the CPU — the resident chain reports `FilmGrainV3FieldCpu` and the CLI/GUI report the effective backend — and preview, LUT, stochastic and spatial deactivation bypass it before allocation, while diagnostic taps, timings and dumps still exercise it. `SPEKTRAFILM_GRAIN_V3_EXPECTATION=1` renders the deterministic expectation without changing the recipe, and `SPEKTRAFILM_GRAIN_V3_CELL_UM` may set a finer certification pitch. See [the Dye specification](specs/film-grain/v3/spec.md).

When downstream spatial processing is active, Dye extends the output grid to cover the full film before those stages and extracts the requested crop afterward. Physical optical controls use the actual output pitch; scanner blur and unsharp masking remain pixel controls. See [film geometry](specs/film-grain/v3/design.md#film-geometry) for rounded and anisotropic output limits. The final Dye cell-address mapping changes the realization of prototype seeds while preserving the probability law. Dye sublayer scales must be positive; the GUI editing minimum is `0.01`.

Cine profiles such as Kodak VISION3 500T can use Dye with their existing stock controls. This availability does not establish cine photographic calibration. The current density-average readout exceeds the spectral aperture gate for the measured 500T-to-2383 condition; see [known optical limits](specs/film-grain/v3/design.md#evidence-and-limits).

Set `SPEKTRAFILM_GRAIN_V3_APERTURE_FACTOR=6` to try the [spectral aperture probe](specs/film-grain/v3/spectral-aperture.md) on an active Dye print route. It reads a finer lattice and averages linear print exposure before print development. Disable enlarger diffusion, scanner blur, unsharp mask, and print glare for this probe. The default path remains unchanged when the variable is absent. Supersampling does not establish cine calibration or replace exact fractional-overlap integration.

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
  spektrafilm-core/    runtime facade, pipeline orchestration, profiles, stage definitions
  spektrafilm-gpu/     ComputeBackend trait + CPU (rayon + BLAS) and wgpu backends
  spektrafilm-shaders/ spectral WGSL shaders and standalone Metal sources
  spektrafilm-cli/     `spektrafilm` / `spektrafilm-f64` (process, list-profiles, lut, export-lut) + `decode_raw_gui`
  spektrafilm-gui/     egui/eframe preview (wgpu renderer, Metal-backed on macOS)
  spektrafilm-raw/     shared native LibRaw white balance and Lensfun correction
  spektrafilm-benchmark/ native benchmark CLI, fixture generator, retained metrics and offline review
data/
  profiles/            film + paper JSON profiles (spectral sensitivities, density curves, etc.)
  luts/                Hanatos2025 spectral basis + standard observer CMFs (.npy)
  filters/             neutral-print enlarger filter database
  icc/                 complete upstream ICC assets and source license notices
  license/             canonical spectral-data license
scripts/parity/        Python 0.3.4 ↔ Rust differential harness (scenarios.py, py_reference.py, run_parity.py, gen_matrix.py)
```

Production image execution enters through `spektrafilm_core::runtime`: `RuntimePhotoParams` owns the profile/data boundary and applies `DigestMode::PreserveUserEdits` for GUI edits or `DigestMode::ApplyStockSpecifics` for CLI/LUT construction. `Runtime` owns the calibrated pipeline and exposes processing/taps/timings. GUI Preview, Scan and independent Export use this facade directly with preserved user edits; Export owns its input and runtime snapshot. CPU f64 reference/export, WGPU f32 execution, and explicit per-stage CPU fallback remain backend-specific below this boundary.

Shared film/print curve and spectral preparation lives in `chain_prep`; stages and the resident builder consume the same derivations. Backend-specific arithmetic, resource ownership and dispatch remain separate. See [the current P1+P2 record](GPU_UNIFY_PLAN.md#p1p2-当前实现2026-10-09) for the execution contract and verification limits.

Feature ownership stays inside the existing crates:

- `spektrafilm-model/src/`: `grain/{v1,v2}.rs`, `halation/`, `diffusion/`, `couplers/`, and `glare/` own their models and numerical tests; `optics/` owns shared physical blur, unsharp masking, and highlight boost.
- `spektrafilm-core/src/params/`: grain, halation, diffusion, couplers, and glare each own their parameter types and feature-specific defaults. `RuntimeParams` remains the aggregate; serialized JSON fields are unchanged.
- `spektrafilm-gpu/src/wgpu_backend/`: each effect owns its buffers, pass encoding, and dedicated WGSL. Shared blur infrastructure stays in `blur/`; `film_chain.rs` owns resident orchestration and its resource lifetimes, while `mod.rs` retains device setup and shared dispatch.

Canonical Rust paths include `spektrafilm_model::grain::v1`, `spektrafilm_model::grain::v2`, and `spektrafilm_core::params::grain::GrainParams`; the former flat module paths have been removed.

After integrating mainline commit `0d194c3`, the feature-directory refactor passed the workspace all-target/all-feature check, 192 existing default tests, and 193 existing f64 tests (including GUI state tests). Ten CPU f64 render/export cases cover V1 and V2 Analogue/Noise, Negative/Positive film types, and print/film-scan output. Their float TIFF pixels match that mainline baseline exactly (maximum absolute difference 0). Native Linux GUI smoke covered the extracted Film/Print panels, preset-to-Custom controls, preview rendering, and the retained export-backend selector. No new tests were added for directory structure or forwarding. Hardware GPU execution and performance were not verified by this refactor's acceptance run.

## Credits

Original Python implementation by Andrea Volpato — [andreavolpato/spektrafilm](https://github.com/andreavolpato/spektrafilm). All spectral data, film/paper profiles, and pipeline architecture come from there. This port owes its existence to Andrea and its work.

The spectral-upsampling LUT (`hanatos2025_*`) is named after [Johannes Hanatos](https://github.com/hanatos), author of [vkdt](https://github.com/hanatos/vkdt), who provided the upstream Python project with the LUT files and sample code that drive the RGB → spectrum step.

PCHIP 3D LUT, MT19937 binomial sampler, and CIE 1931 observer constants are ported from numpy / scipy / scikit-image / colour-science.

Claude code for being this awesome.

## License

The Rust program is GPL-3.0; see [LICENSE](LICENSE). Andrea Volpato's spectral profiles and their LUT derivatives retain CC BY-SA 4.0; see [the canonical asset license](data/license/SPEKTRAFILM_LICENSE.txt).

The optional LabScan scanner finish adapts Fotufilm project-authored `Sources/FotufilmCore/LabScanFinish.swift` under Apache-2.0, pinned at research revision `59d43471dbef45c85ae442e507a291fdebfb2ec5` (source SHA-256 `fd2352193a8c91232729e51cc83cf748236b3a4b3cfaed07e703f5ae0943fe73`). The approximate narrow-band receiver follows Fotufilm's `PrintPaperTables.swift` sensitivity approximation; it is not a measured or calibrated scanner device. Receiver and finish source/version metadata is retained in scanner provenance.

The optional Fotufilm Film grain engine adapts the crystal-population and periodic-aperture mechanism of Fotufilm project-authored `Sources/FotufilmCore/FilmGrain.swift` under Apache-2.0, pinned at the same research revision `59d43471dbef45c85ae442e507a291fdebfb2ec5` (source SHA-256 `76ad52a3421d6f1c3cd7c5d98aabd488dcf81c1212555a317cd650f36d067fad`). It is an authored appearance model: the resolved spectral film profile remains the authority for developed colour and tone, its grain parameters are explicitly authored effective values rather than measured stock data, and the complete-periodic readout above 96 micrometers is a documented extension of the upstream clipped sampler.
