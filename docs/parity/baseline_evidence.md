# Baseline parity evidence and budgets

This file records what is **measured**, what is **preserved as historical
evidence**, and what is **explicitly unsupported** for the Python 0.3.4 ↔
Rust parity effort. Nothing here asserts universal bit identity; every
budget carries its provenance. Machine-readable state lives in
[`parity_matrix.json`](parity_matrix.json); runnable commands live in
[`../../scripts/parity/README.md`](../../scripts/parity/README.md).

The source reproductions below describe the original audited Rust commit,
not the integrated migration implementation. Current scenarios preserve
these observations under `audit_baseline` and require parity with unchanged
budgets. Fresh evidence comes only from the differential report. The pinned
venv import probe now succeeds; missing-runtime observations below remain
historical. Live asset inventory now finds all 214 Python paths: 212 are
byte-identical, the neutral filter database is a verified superset, and the
ICC README is a retained Rust-adapted document with a differing hash.

## Pins

| Side | Version | Commit |
|------|---------|--------|
| Python reference | 0.3.4 (`pyproject.toml:7`) | `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc` |
| Audited Rust baseline | — | `9dd59b0380194b93686aaa230a8bb9680aa270a4` |

There is no locally available 0.3.4 release tag; the commit **is** the pin.
The harness (`scripts/parity/run_parity.py`) verifies both sides before any
measurement and refuses to run otherwise.

Reference dependency set (from the pinned `pyproject.toml`, core runtime):
numpy~=2.4, scipy~=1.17, colour-science~=0.4.6, scikit-image~=0.26,
matplotlib~=3.10, opt-einsum~=3.4.0, numba~=0.64, OpenImageIO~=3.1.11,
pyfftw~=0.15.0, rawpy~=0.26.1, exiv2~=0.18.1, lensfunpy~=1.18.0.
Numerical reductions that matter for parity (numpy `sum`/`einsum`) use the
interpreter's built-in pairwise summation — no external BLAS is in the
reference path for the probed stages; the Rust side links `blas_src` only
for the spectra→tc_lut dgemm (see `crates/spektrafilm-core/src/spectral_service.rs:10-14`).

## Preserved historical evidence — the 4×4 bare chain

**This result is preserved as history, not re-measured here.** During the
2026-09-30 read-only migration audit, the default no-effects 4×4 reference
(Python 0.3.4 `simulate()` vs Rust f64 chain, linear sRGB in/out, CAT16,
input gamut `xy`, output gamut `cam16ucs`, print-curve models active) showed
a **maximum absolute error of ≈ 1.14 × 10⁻⁸** on f64. The Python reference
was not re-run afterwards (venv dependencies incomplete); no fresh Python
execution is claimed since.

The measured values are pinned as constants in
`crates/spektrafilm-core/src/stages/mod.rs::full_chain_matches_python_0_3_4_defaults`
(1×1 midgray + the 16-pixel grid), guarded by the **1e-6 f64 budget**
(`TOL` under `#[cfg(feature = "precision-f64")]`). That test is the shipped
regression guard for the historical measurement; the harness scenario
`bare_chain_4x4` reproduces the same input formula
(`v = 0.05 + 0.5·((i·37) mod 256)/255`) and the same parameter set, so a
fresh differential run lands on the same reference values once the venv is
complete.

## Budgets and provenance

| Budget | Target | Provenance |
|--------|--------|------------|
| `arithmetic` | max_abs ≤ 1e-6 | measured 1.14e-8 (2026-09-30 audit, f64 both sides); shipped test budget 1e-6 |
| `arithmetic_spatial` | max_abs ≤ 5e-5 | provisional — deterministic FFT/interp-heavy stages (lens blur, unsharp, DIR, diffusion); measure on first harness run |
| `lut_reduction` | max_abs ≤ 1e-5 | provisional — PCHIP LUT-reduced paths (`use_scanner_lut`); kernels claimed bit-parity; measure on first run |
| `rng_stream` | max_abs ≤ 1e-6 | seeded streams (grain seeds 0/1/2, `model/grain.py:87`); Rust ports the legacy `np.random` generators; measure on first run |
| `statistical_texture` | mean_rel ≤ 1%, std_rel ≤ 5% | moment-level budget for stochastic appearance (layered grain, glare); per-pixel equality not applicable |
| `file_fidelity` | decoded pixels within ±1 LSB | encoded-output compare (PNG-8/TIFF-16); byte equality **not** required — different encoders |
| `preview` | provisional mean_abs ≤ 2e-2 | preview approximation: Python `skimage.resize` order=1 anti-aliased vs Rust GUI rescale; owner issue #16 |

The five concerns are deliberately **separate**: arithmetic parity of the
deterministic chain, seeded RNG stream equality, statistical grain/glare
appearance, encoded file fidelity, and preview approximation. A pass in one
column never stands in for another.

## Baseline reproductions for known gaps

Each item below is a divergence verified against source at the audited
baseline, with the harness scenario that demonstrates it and the owning
migration issue. Code references: Python paths are inside
`/home/szf/repos/spektrafilm` at the pinned commit; Rust paths are inside
this repository.

### 1. Ignored crop controls — scenario `crop_controls` (issue #7)
Python `_preprocess` runs `ResizingService.crop_and_rescale` after
auto-exposure (`runtime/pipeline.py:191-195`); `io.crop`,
`io.crop_center`, `io.crop_size` are parsed by Rust `IoParams`
(`crates/spektrafilm-core/src/params.rs:544-548`) but **never read** by any
stage — the output geometry itself differs (Python emits the cropped size).

### 2. Ignored input decoding — scenario `input_cctf_decoding` (issue #4)
`io.input_cctf_decoding=True` makes Python decode the input CCTF inside the
RGB→XYZ conversion (`_rgb_to_tc_b` → `colour.RGB_to_XYZ(...,
apply_cctf_decoding=...)`, `utils/spectral_upsampling.py:133`). Rust parses
the field but never reads it (grep over `crates/spektrafilm-core`: only
tests and the CLI's PNG/RAW auto-detect touch it).

### 3. Ignored UV/IR filters — scenario `uv_ir_filters` (issue #5)
`camera.filter_uv`/`filter_ir` trigger a band-pass filter on the film
sensitivity with white-balance normalization
(`runtime/stages/filming.py:99-104`). Rust parses the fields
(`params.rs:82-85`) and even ships the erf4 window evaluation
(`spectral_service.rs:232`), but nothing connects the camera params to it.

### 4. Ignored blur/layered-grain controls — scenarios `layered_grain`,
`preview_mode_digest` (issues #6, #3, #5)
`film_render.grain.sublayers_active` (default **True** upstream!),
`particle_scale_layers`, `blur_dye_clouds_um`, `micro_structure`,
`settings.use_fast_stats`, `settings.preview_mode` are all accepted by Rust
serde and never read. The upstream default path develops grain through
`interp_density_cmy_layers` + `apply_grain_to_density_layers`
(`model/grain.py:193-213`); only the single-curve fallback
(`sublayers_active=False`) is ported. The Hanatos adaptation blur knobs
`settings.spectral_gaussian_blur` and
`settings.apply_hanatos2025_adaptation_surface` (issue #5) are likewise
parsed and never applied when the tc LUT is built
(`crates/spektrafilm-core/src/pipeline.rs:254-287` applies only the window
variant), while Python filters the spectra LUT and multiplies the surface
correction (`utils/spectral_upsampling.py:336-386`).

### 5. Unsupported gamut fallback — scenario `gamut_output_jzazbz` (issue #4)
`OutputGamutCompress::build` treats `jzazbz`/`oklrab` and any non-sRGB
output as error-logged **identity fallback**
(`crates/spektrafilm-core/src/gamut_compression.rs:267-293`); Python 0.3.4
supports all five algorithms on any output space
(`utils/gamut_compression.py:197-216`). Similarly unknown input color
spaces silently fall back (filming.rs:516 → ProPhoto; scanning.rs:455 →
sRGB) where Python raises.

### 6. Wrong LUT constructor — CLI `export-lut` (issue #13)
`cmd_export_lut` builds its CUBE through the **simplified** non-spectral
`Pipeline::new` with `print_render.glare` and `scanner.unsharp_mask` left
active (`crates/spektrafilm-cli/src/main.rs:368-410`). Upstream LUT bakes
run the full spectral pipeline under `debug.lut_mode`
(`runtime/params_builder.py:99-118`), which forces the deterministic
per-pixel regime (spatial/stochastic off, AE off, corrections off).

### 7. Output encoding / clipping — scenario `output_encoding_srgb` (issue #4)
Rust clamps the final RGB to [0,1] on **both** branches of the encoding
switch (`crates/spektrafilm-core/src/stages/scanning.rs:370-389`); Python
0.3.4's `_apply_cctf_encoding` never clips (`runtime/stages/scanning.py:130-139`)
— the doc-comment in Rust references a `_apply_cctf_encoding_and_clip`
helper that does not exist at this pin. With gamut compression off and
out-of-gamut input, Python returns out-of-range values.

### 8. Direct-scan glare — scenario `scan_film_glare` (issue #8)
On `scan_film=True` Python passes `glare=None`
(`runtime/stages/scanning.py:54-55`) — no glare on the direct scan, ever.
Rust selects `film_render.glare` for the scan path
(`crates/spektrafilm-core/src/stages/scanning.rs:284-288`) and applies it
with a **fixed seed 42** (`scanning.rs:325-333`), diverging both in
semantics and in RNG stream.

## Integrated acceptance evidence

The pinned Python runtime has generated 68 reference scenarios. All 68 passed
the rebuilt Rust f64 differential run, including all shared profiles, seeded
grain, deterministic and stochastic preview, spatial effects, and six invalid
paper-as-film configurations rejected before producing output. The bare-chain
maximum absolute error was 4.509e-10 against the 1e-6 budget.

Independent executed checks establish:

- All 265 LUT topology, stock and transport cases passed the stored Python
  reference comparison. Generic CUBE, Lumix CUBE, 3DL and Hald PNG also passed
  independent format readers, including axis order and ties-to-even quantization.
- The rebuilt f64 CLI passed 112 QA scenario comparisons across all four LUT
  topologies and seven print selections: zero metric drifts and zero status
  mismatches at 1e-5 relative tolerance with a 1e-7 absolute floor. Upstream
  FAIL and INFO results remain unchanged. QA artifact registration, offline
  HTML references and the delivered ZIP contents passed independent checks.
- Nine seeded 256×256 grain cases matched every pixel exactly. Dark fast
  texture uses a 67,108,864-sample aggregate: mean relative errors
  [0.00007035, 0.00157755, 0.00519272] and standard-deviation relative errors
  [0.00021169, 0.00005989, 0.00015846] passed the unchanged 1%/5% budgets.
  Original smaller-sample failures are retained; this is moment evidence.
- DIR correction now retains f64 profile exposure and density curves through
  normalization and interpolation. Across 50,000 stimuli, film-density maximum
  error fell from 1.89037828502e-7 to 3.21964677141e-15 (mean 3.20310101504e-16).
  Reconstructing the former f32 tables in Python reproduced the original error;
  negative and positive film precision vectors have a pinned regression check.
- Native GUI operations passed under Xvfb: image Open, state Save/Load and
  restart, layers, float inspection, Save, actual CPU export, cancellation and
  closing during export. Cancel/close left no exporter child or temporary JSON.
- A relocated Linux package started the GUI and processed a floating TIFF
  with EXIF/IPTC/XMP/ICC preservation. Its identity export reproduced the input
  buffer exactly and its 32-bit float export of the pinned midgray spectral
  reference matched within 6.635e-9 against the 1e-6 budget. Real RAW decoding
  passed all eight
  camera/white-balance cases exactly, both before and after relocation, including
  a known Lensfun correction. The rebuilt consumers load LibRaw 0.22's actual
  `libraw.so.24`; its selected link artifact prevents system-header ABI mismatch.
  The final DIR-corrected Linux package passed the same portable image/RAW/GUI
  smoke under Xvfb with a clean environment. The fork's Release workflow is now
  active; Windows/macOS installed-workflow acceptance remains unverified.
  PR #18 remains a draft, and acceptance issues #17/#1 remain open.
- Saving into the output layer's own colour space and encoding is now a
  bit-exact copy, matching the pinned Python save guard
  (`spektrafilm_gui/controller.py:354-369`). Previously the export ran the
  4-digit IEC sRGB matrices as a round trip even when nothing needed
  converting: a 4×4 gradient probe measured up to 2.293e-5 drift between the
  saved 32-bit TIFF and the pipeline buffer. Against the pinned midgray
  reference the raw f64 buffer is 4.864e-10 off and the saved 32-bit TIFF
  6.635e-9 off; recomputing the old round trip with the same matrices gives
  9.503e-6. `crates/spektrafilm-core/tests/image_io.rs` pins the no-op and
  fails on the pre-fix code.
- The actual wgpu adapter exercised 224 space/encoding/effect routes. Maximum
  arithmetic error was 0.0001019144361 and maximum mean error 0.000004945424654
  against CPU f64. Grain used the faithful CPU sampler; its preview differences
  were measured separately (maximum 0.00083358). Repeated half-sample reflection
  now matches CPU FIR boundaries in WGSL and CUDA Gaussian kernels. CUDA was
  not executed on this host. The full workspace passed 189 tests.
- Review corrections preserve explicitly edited stock halation/DIR parameters
  across construction and updates, include input-compression activation in the
  spectral cache key, and resolve omitted endpoints from persistent taps.
  Nested print morph configuration defaults inactive, matching pinned Python.
  The corrected release CLI passed all 68 fresh runtime differential scenarios.
- EXR16 rounds retained f64 samples directly to binary16, eliminating the f32
  intermediate. Actual EXR16/EXR32/TIFF32 write/read regressions cover signed
  midpoint neighbors, tie parity, subnormals, overflow, zero, NaN and infinity.
  `1.00048828125 + 1e-10` now saves as `1.0009765625`, matching numpy float16.
- Input-compression QA shares the selected runtime compressor and respects its
  activation switch. The corrected CLI passed the original 112 pinned QA
  comparisons plus 32 additional comparisons for disabled xy and active oklch,
  with unchanged metric budgets, status semantics, offline artifact and OCIO checks.
  The integrated precision-f64 workspace passed 192 tests after these corrections.
- The new external Linux package GUI gate passed real native window and file
  chooser operations under Xvfb: standard/RAW loading and preview, input/output
  raster comparison, Auto exposure and float-depth edits, state save/load and
  restart restoration, float save, actual bundled f64 export, Cancel and closing
  in flight. Observed exporter executable hashes match installed native payloads;
  cancelled/closed exports left no output, child process or temporary JSON.
  Windows/macOS drivers exist but have no passing target-platform evidence yet.
- The first real three-platform workflow exposed Ubuntu Exiv2 auto_ptr predicate
  incompatibility, Homebrew OIIO's missing external fmt include path, and MSYS2
  LibRaw below 0.22. The fixes retain the native contracts: get() pointer checks,
  installed-header-based fmt/Imath include discovery, and checksum-pinned UCRT64
  LibRaw 0.22 build with merged native dependency/license packaging.
- Workflow 37055584083 exposed adapter workgroup limits below 1024, GLib's
  Homebrew-installed SPDX license filename, and a partially written output after
  native Cancel. Linear GPU kernels now use portable workgroups and a two-dimensional
  dispatch grid. The integrated workspace passed 192 tests after these repairs.
  The license collector includes installed SPDX-named full license texts.
- Export writes a guarded sibling staging file and publishes it atomically only
  after the UI accepts successful completion without cancellation. The rebuilt
  Linux package passed the full native GUI/RAW/LUT smoke, including real in-flight
  Cancel and close with no output, child or staged-image/state-JSON residue.
- Current calibration updates rebuild the print exposure factor when camera EV,
  print compensation/normalization or illuminant/filter calibration inputs change.
  The workspace regression compares updated pipelines with freshly constructed
  pipelines across EV -2 through +2.
- The current release CLI passed all 68 runtime differential rows in
  `/tmp/spektrafilm-current-parity/parity_report.json`. LUT artifact acceptance
  passed 112 topology QA comparisons, 32 input-compression override comparisons,
  six F-Log/F-Log2/N-Log/ProPhoto/PQ/HLG bundle comparisons, OCIO processors and
  public format/transport probes in `/tmp/spektrafilm-lut-current-3/report.json`.
  Unsupported optional OCIO mappings explicitly skip configuration emission;
  `qa_print_index` alone does not enable QA. Scenario errors remain named failed
  rows while later measurements continue. Finalization preserves the README
  Quality table, writes `bundle.json`, and creates ZIPs only when requested.
  Scientific QA failures at coarse resolution match Python's reported failures;
  acceptance verifies the results and statuses rather than requiring every LUT
  quality metric to pass.
- Profile serialization preserves missing spectral samples as JSON null and
  saves a suffixed copy without mutating the caller. Parametric curves retain
  the upstream formula. The public gamma/slope helpers use linear-cost
  not-a-knot spline construction; a seven-sample cubic smoke matched SciPy
  gamma `[3.103978836041295, 3.1039788360412963, 3.6538466942740713]`
  and slopes `[3.21, 3.21, 3.21]`. Nonlinear density-min fitting and plotting
  remain independent research APIs outside product parity.
- The final workspace passed 197 tests. The relocated portable Linux package
  passed installed image/metadata/RAW/LUT/OCIO/QA and native GUI acceptance at
  `/tmp/spektrafilm-current-package-evidence-3`. Native assertions include
  Scan-for-print force/restore/state reset, Input interpolation isolation,
  preview-limit refresh with full-resolution Export, persistent RAW lens status,
  animation, display/save isolation and actual in-flight Cancel/close cleanup.
  Windows/macOS remain governed by their CI evidence, not Linux screenshots.

Pinned Python glare uses unseeded Numba thread-local random streams. Repeating
`np.random.seed(0)` does not reproduce its pixels: the measured 512×512 repeat
difference was 0.009055880483984825. Deterministic preview digestion and
stochastic glare need separate scenarios. The production glare sampler matches
the explicitly seeded Python interpreter path within 4.78e-18; its actual JIT
mean/std comparison passed the existing statistical thresholds (relative
errors 0.000699606 and 0.00311325). These checks do not establish deterministic
global RNG continuation through the full stochastic chain.


## Regenerating the evidence

```bash
# fixtures + per-tap dumps from the pinned Python core, then Rust f64 CLI,
# then per-stage/final max+mean report (fails hard on missing prerequisites)
python3 scripts/parity/run_parity.py

# checkpoint matrix state without fake results when a prerequisite is missing
python3 scripts/parity/run_parity.py --record-unsupported

# refresh the machine-readable inventory (live asset hashing)
python3 scripts/parity/gen_matrix.py
```
