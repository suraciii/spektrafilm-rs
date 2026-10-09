# Python ↔ Rust parity harness

Differential tooling against the **pinned** Python reference
`spektrafilm` 0.3.4 at commit `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`
(local checkout `/home/szf/repos/spektrafilm`) and the audited Rust
baseline `9dd59b0380194b93686aaa230a8bb9680aa270a4`. There is no 0.3.4
release tag — the commit is the pin; the harness verifies it before
measuring anything.

Files:

| File | Purpose |
|------|---------|
| `scenarios.py` | Shared scenario catalog + budgets (single source of truth) |
| `py_reference.py` | Pinned Python driver — fixtures + per-tap f64 dumps (core imports only) |
| `run_parity.py` | Orchestrator — preflight, Rust f64 CLI, per-stage/final max+mean report |
| `gen_matrix.py` | Regenerates `docs/parity/parity_matrix.json` (live asset hashing) |
| `lut_acceptance.py` | Real CLI LUT bakes, pinned QA/format comparisons, OCIO processors and delivered artifact checks |
| `package_smoke.py` | Installed image/metadata/RAW paths, LUT/OCIO/QA delivery and actual native GUI operations |
| `gui_viewer_acceptance.py` | Native viewer controls, float probes, animation frames, profile/non-sRGB paths and Save/Export isolation |
| `grain_v2_acceptance.py` | Grain V2 presets, Film Types and endpoints, CPU/GPU dispatch parity, Custom parameter inheritance and f32/f64 float TIFF render/export roundtrips |

Grain V2 acceptance: run `python3 scripts/parity/grain_v2_acceptance.py`.
The GPU comparison dispatches the actual shader across multiple workgroups for all twelve profiles in Analogue and Noise modes, with a maximum absolute error budget of 0.005 against the independent CPU implementation. A missing WGPU adapter is reported as a skip; it is not GPU evidence. Both kernels consume native encoded RGB without a hardcoded transfer. Scanner regressions cover four output spaces, film/paper scans, both modes and linear/encoded output in f32/f64; actual renders are saved and reloaded as float TIFF. Recovered Gaussian FastBlur, OpticalResolution, half storage and noise semantics are tested; vendor-device bit-exact randomness is not required.

Local verification on 2026-10-08: workspace tests passed (240 tests); native f32 WGPU and f64 GUI previews rendered, the V2 mode control was exercised, and native Save/f64 Export wrote readable TIFF files. CLI V2 processing and six f32/f64 render/export roundtrips passed. The local native dependency prefix was `/data/deps/libraw-0.22.2` (real LibRaw 0.22.2). Hardware render-node access was denied, so this establishes WGPU execution with the available adapter, not discrete-GPU performance. Strict all-features clippy stopped in unchanged math sources on existing diagnostics.

The subsequent V2 sky-stripe repair passed eight grain tests in both f32 and f64, the shader device check, profile inheritance checks, and all six render/export roundtrips. Final Noise host scale, half-effective Amount and Film Resolution behavior are included. A separate f64 CLI run exported and decoded a 768×512 synthetic sky TIFF with spatial grain residual RMS 0.00799 against grain disabled. These checks do not establish a fix for an unavailable original user photo or hardware-GPU performance.

The public-control alignment passed nine grain tests in each precision, shader device validation, preset/Custom inheritance, and ten TIFF roundtrips per precision. Four GUI state tests passed separately. Native GUI scanning exercised Positive/Noise and preset-to-Custom inheritance with editable preset Amount. The isolated display did not open the Save state dialog; native state saving is not claimed for this run. Current CLI measurements and host dispatch provenance are recorded in [the public-control alignment evidence](../../docs/dehancer-grain-reverse-engineering.md#12-公开参数对齐2026-10-08).

Evidence, budgets and provenance live in
[`docs/parity/baseline_evidence.md`](../../docs/parity/baseline_evidence.md);
the machine-readable inventory is
[`docs/parity/parity_matrix.json`](../../docs/parity/parity_matrix.json).

Pass `--gui-report path/to/gui-acceptance/observations.json` to `gen_matrix.py` to retain the executed native GUI records and their SHA256 alongside the spectral report. A failed or incomplete GUI report is rejected. Each named scenario remains scoped to its actual assertions; Linux desktop evidence does not establish Windows/macOS display behavior.

## Experimental GUI gate

`experimental_gui.py` targets upstream experimental commit
`28bf883e1672e884307edc75852549376e13644e`. The historical 0.3.4 catalog below
does not establish experimental acceptance. Build the recorded implementation
commit first, then run from a clean worktree with an unused evidence directory:

```bash
xvfb-run -a -s '-screen 0 1600x1100x24' dbus-run-session -- \
  /tmp/spektrafilm-034-venv/bin/python scripts/parity/experimental_gui.py \
  --gui target/debug/spektrafilm-gui \
  --upstream /tmp/spektrafilm-upstream-28bf \
  --evidence /tmp/spektrafilm-experimental-native
```

The Linux gate uses real X11 input, OCR, and native file choosers. It checks the
five tab section order, consolidated effect fields, fixed actions, and each
of the six Workflow selections. Every route is saved to canonical state,
rendered with PREVIEW and SCAN, saved as TIFF, and loaded/saved again to verify
state roundtrip. The report records executable, input, state, profile/preset,
observation, and screenshot hashes. Numerical upstream parity remains a
separate runtime gate; native rendering alone does not establish it.

Start D-Bus inside Xvfb so the GTK portal inherits DISPLAY. The driver performs
an initial preview before saving state to establish the output viewing layer.
It distinguishes the SCAN action from the Scan for print checkbox and observes
a saved-state status before each new SCAN completion; image dimensions in the
Rendered status are not an execution counter.

`--development-smoke` permits an uncommitted iteration and always reports
`development-smoke`, never acceptance `pass`. Missing controls, clipped labels,
failed file dialogs, incorrect state, nonfinite pixels, and changed provenance
fail explicitly. Evidence from this Linux gate does not establish Windows or
macOS behavior.

`experimental_runtime.py` runs the same deterministic 32×24 RGB fixture against
all six upstream and Rust routes. Both sides explicitly disable grain, print
glare, and auto exposure and use linear sRGB input/output. The maximum absolute difference
budget is fixed at `1e-5`; missing routes, nonfinite output, and execution errors
fail the gate. It retains each parameter snapshot, raw output, process log, and
their hashes. Use a newly built f64 CLI from the recorded commit:

```bash
SPEKTRAFILM_RS_BIN="$PWD/target/release/spektrafilm-f64" \
SPEKTRAFILM_UPSTREAM=/tmp/spektrafilm-upstream-28bf \
  /tmp/spektrafilm-034-venv/bin/python scripts/parity/experimental_runtime.py \
  --out /tmp/spektrafilm-experimental-runtime
```

The runtime gate also rejects a dirty checkout unless `--development-smoke` is
explicitly set. Its JSON status distinguishes a development run from acceptance.
Build logs must bind the binary hash to the recorded Rust commit; a commit field
alone does not prove binary provenance.



## Reference environment (one-time setup)

Python 3.13 venv with the 0.3.4 runtime stack (core only — the GUI and
LUT-creator packages are installed by the project's default extras but are
never imported by the driver; `py_reference.py` asserts this at exit):

```bash
python3.13 -m venv /tmp/spektrafilm-034-venv
/tmp/spektrafilm-034-venv/bin/pip install --upgrade pip setuptools wheel
/tmp/spektrafilm-034-venv/bin/pip install --prefer-binary \
    "numpy~=2.4" "scipy~=1.17" "colour-science~=0.4.6" "scikit-image~=0.26" \
    "matplotlib~=3.10" "opt-einsum~=3.4.0" "numba~=0.64" \
    "OpenImageIO~=3.1.11" "pyfftw~=0.15.0" "rawpy~=0.26.1" \
    "exiv2~=0.18.1" "lensfunpy~=1.18.0"
git -C /home/szf/repos/spektrafilm checkout 3bb2c2d2801ff68b92019cf1dbcbb133d60832bc
/tmp/spektrafilm-034-venv/bin/pip install --no-deps -e /home/szf/repos/spektrafilm
```

The pinned venv import probe is now usable. The integration owner builds
the f64 binary and runs the complete catalog; missing prerequisites always
produce unsupported evidence and exit 2.

Rust side (f64 precision is a cargo feature):

```bash
cargo build -p spektrafilm-cli --features precision-f64 \
    --bin spektrafilm-f64 --release
```

Override locations with `SPEKTRAFILM_PY`, `SPEKTRAFILM_PY_REPO` and
`SPEKTRAFILM_RS_BIN` if they differ from the defaults.

For CI or another checkout, `SPEKTRAFILM_RUNTIME_REPO` points the provenance
and data/work-directory checks at a separate clean exact-a191 runtime
worktree while the parity script itself remains the candidate checkout. This
keeps PR merge refs testable without weakening the exact-commit gate.


### Fresh provenance gate

Runtime evidence is accepted only for the exact Rust runtime commit
`a1910231fbd2c049c5177b539b6e7963c97f4e90`. `run_parity.py` rejects another
checkout, a dirty worktree (including untracked files), or a non-executable
CLI. Its report records the complete lowercase SHA256 of the executable.
`gen_matrix.py` re-checks that commit, requires `rust_worktree_dirty: false`,
requires a syntactically complete executable hash, and compares the hash with
the recorded binary when that path is available. It therefore rejects copied
or stale runtime reports rather than treating old rows as fresh evidence.

Keep source and evidence separate when running the clean a191 runtime
worktree. For example, with the report artifacts outside the checkout:

```bash
RUNTIME=/tmp/spektrafilm-a191-20261007
OUT=/tmp/spektrafilm-a191-parity-fresh
SPEKTRAFILM_RS_BIN=/tmp/spektrafilm-a191-target/release/spektrafilm-f64 \
  python3 "$RUNTIME/scripts/parity/run_parity.py" --out-root "$OUT"
python3 "$RUNTIME/scripts/parity/gen_matrix.py" \
  --report "$OUT/parity_report.json"
```

The runtime checkout itself must remain clean; `--out-root` is intentionally
outside it. A report from another commit, a dirty checkout, or a replaced
binary fails before matrix generation.
## Running the matrix

```bash
# full matrix (Python fixtures → Rust f64 CLI → per-stage/final report)
SPEKTRAFILM_PY=/tmp/spektrafilm-034-venv/bin/python \
SPEKTRAFILM_RS_BIN="$PWD/target/release/spektrafilm-f64" \
python3 scripts/parity/run_parity.py --out-root target/parity-fresh

# catalog only
python3 scripts/parity/run_parity.py --list

# single scenario (e.g. the preserved 4x4 bare chain)
python3 scripts/parity/run_parity.py --only bare_chain_4x4

# existing Python fixtures must live at <out-root>/fixtures/<scenario>/
python3 scripts/parity/run_parity.py --rust-only --out-root target/parity

# checkpoint state when a prerequisite is missing: writes explicit
# `unsupported` evidence rows and exits 2 — never a passing skip
python3 scripts/parity/run_parity.py --record-unsupported
```

Exit codes: `0` every current row within its unchanged budget; `1` budget
failure, missing tap or invalid run; `2` unsupported prerequisites. Original
gap descriptions live under `audit_baseline` and never excuse current drift.

Per-scenario artifacts land in `target/parity/fixtures/<scenario>/`:
For manually generated references, pass `py_reference.py --out-dir
<out-root>/fixtures`; `run_parity.py --out-root <out-root> --rust-only`
expects that exact directory. A reference root named `python` needs an
explicit `fixtures` alias before running the Rust comparison.


- `input.tif` — shared float32 linear fixture (both sides read this file);
- `py_<tap>.f64` / `rs_<tap>.f64` — little-endian f64 dumps per topology
  tap (`log_e_film`, `cmy_film`, `log_e_print`, `cmy_print`, `rgb_out`);
- `rs_params.json` — the exact Rust param overrides sent to the CLI.

Python produces taps through `SimulationPipeline.process(collect=...)`
(named topology taps, `runtime/topology.py`). Rust produces them through
env-probe hooks in the core pipeline (`SPEKTRAFILM_DUMP_FILM_LOG_RAW`,
`SPEKTRAFILM_DUMP_FILM_DENSITY`, `SPEKTRAFILM_DUMP_PRINT_LOG_RAW`,
`SPEKTRAFILM_DUMP_PRINT_DENSITY`) plus `--raw-out` for the final buffer,
with `SPEKTRAFILM_BACKEND=cpu` so the dumps are produced by the same CPU
code path the arithmetic budgets describe.

## Regenerating reference fixtures by hand

```bash
/tmp/spektrafilm-034-venv/bin/python scripts/parity/py_reference.py \
    --repo /home/szf/repos/spektrafilm \
    --out-dir target/parity/fixtures --all

# then, per scenario, the Rust side with dumps:
SPEKTRAFILM_BACKEND=cpu \
SPEKTRAFILM_DUMP_FILM_LOG_RAW=target/parity/fixtures/bare_chain_4x4/rs_log_e_film.f64 \
SPEKTRAFILM_DUMP_FILM_DENSITY=target/parity/fixtures/bare_chain_4x4/rs_cmy_film.f64 \
SPEKTRAFILM_DUMP_PRINT_LOG_RAW=target/parity/fixtures/bare_chain_4x4/rs_log_e_print.f64 \
SPEKTRAFILM_DUMP_PRINT_DENSITY=target/parity/fixtures/bare_chain_4x4/rs_cmy_print.f64 \
target/release/spektrafilm-f64 process \
    target/parity/fixtures/bare_chain_4x4/input.tif \
    -o target/parity/fixtures/bare_chain_4x4/rs_out.png \
    --film kodak_portra_400 --paper kodak_portra_endura \
    --params target/parity/fixtures/bare_chain_4x4/rs_params.json \
    --raw-out target/parity/fixtures/bare_chain_4x4/rs_rgb_out.f64 \
    --data-dir data
```

`run_parity.py` automates exactly this invocation for every scenario.

## Interpretation rules

- **expected_parity** rows must meet their budget; failures block.
- Original gap observations are retained under **audit_baseline**. Current
  migrated scenarios require parity and exceeding their budget blocks.
- **historical** rows guard the preserved 4×4 bare-chain evidence
  (≈1.14e-8 measured 2026-09-30; see `docs/parity/baseline_evidence.md`).
- Canonical Python parameter names reach strict Rust validation unchanged;
  the harness neither renames grain controls nor drops debug/tap fields.
- Stochastic appearance (layered grain, glare) is judged by the
  `statistical_texture` moment budget, not per-pixel equality.

## LUT and package acceptance

Install the pinned Python LUT creator dependencies and `opencolorio` alongside
the reference runtime. The Linux CI gate runs the full runtime catalog and:

```bash
python scripts/parity/lut_acceptance.py \
    --cli target/release/spektrafilm-f64 --data-dir data \
    --evidence-dir target/lut-acceptance
```

The LUT gate compares numeric QA metrics and the original PASS/FAIL/INFO
statuses independently. An upstream quality failure remains a failure in the
delivered QA report; acceptance requires the same result as the pinned Python
implementation. Relative tolerance is `1e-5` with a `1e-7` absolute floor.
Independent format readers and OCIO processors exercise the delivered files.
The same gate compares 32 additional diagnostic results for disabled xy and
active oklch input compression, separately from the 112 baseline results.


The package smoke command always covers installed image/metadata/RAW paths,
LUT/OCIO/QA delivery and binary provenance. Native GUI acceptance is optional:
pass `--gui` only for a real desktop/Xvfb run. The release workflow currently
omits `--gui` by design, so its three package jobs do not claim GUI coverage;
the GUI driver remains available as a separate, explicit evidence command.
When enabled, the driver operates real windows and file dialogs, records
screenshots, saves/restores state across restart, decodes RAW, saves the retained
output, observes the GUI's independent Export worker, and checks cancellation
and window-close cleanup. Linux GUI runs use Xvfb with a session bus created
inside that display; Windows/macOS require a real interactive desktop and
platform permissions.

Desktop acceptance requires Pillow, mss, pytesseract, psutil and the Tesseract
engine. Linux additionally requires openbox, xdotool, xclip, xprop, xwininfo,
zenity, ffmpeg, gdbus, and a working GTK file chooser portal on the same display.
The driver checks portal readiness before launching the GUI. Package smoke also
builds a ZIP bundle through the installed exporter, checks offline
report references and artifacts, and executes its delivered OCIO processors.
`package_smoke.py` requires `--package-root` and writes `package_report.json`
next to `observations.json`. It records the Rust HEAD, pinned reference commit,
platform, worktree status, a deterministic package-tree SHA256, each delivered
binary SHA256 and the observed scenario results. A passing smoke command without
that provenance is not publication evidence.
For a single local gate, provide the built f64 CLI and portable package:

```bash
xvfb-run -a -s '-screen 0 1600x1100x24' dbus-run-session -- python3 scripts/parity/run_all.py \
  --rust-bin target/release/spektrafilm-f64 \
  --data-dir data \
  --package-root dist/spektrafilm-linux-x64 \
  --out-root target/acceptance
```

The wrapper runs the existing runtime parity, LUT acceptance, and package smoke
checks without adding another test framework. It fails on missing prerequisites
or on the first failed check and writes command logs plus `report.json`.
The wrapper uses `SPEKTRAFILM_PY` for installed package acceptance and defaults
to the pinned reference venv; `SPEKTRAFILM_PY_REPO` defaults to the sibling
checkout. Supply both variables on CI or when using another checkout. Final
reports record the Rust HEAD, executable SHA256, platform and reference pin.

`--raw-fixture PATH` reuses the pinned Kodak KDC download when network access
is unavailable; the same required SHA256 check runs before decoding.

`raw_wb_audit.json` is the pinned decoded-buffer fixture matrix for RAW
white-balance repair issue #26. It covers Kodak KDC and Canon 40D CR2 across
`as_shot`, `daylight`, `tungsten` and `custom(5000K, tint=1.05)` with
`lens_correction=false`, records the source URLs/SHA256/dimensions and the
current verified decoder/reference provenance. The report binds the eight
passing rows to the audited Rust commit and decoder SHA256; all rows must
remain within `max_abs <= 1e-5` and `mean_abs <= 1e-6`. Download the public
fixtures, verify their hashes, then rerun `raw_reference_compare.py` to
regenerate the evidence. This gate compares decoded float buffers; visual
screenshots or PNG-clamped comparisons are not substitutes.


The package smoke exports the pinned 0.3.4 bare-chain midgray through the real
spectral assets and compares the saved 32-bit TIFF against the recorded Python
result within `1e-6`. Saving into the output layer's own colour space and
encoding is a bit-exact copy of the pipeline buffer — the pinned Python save
guard skips the transform there, and re-running the 4-digit IEC sRGB matrices
would shift pixels by ~2e-5 (`crates/spektrafilm-core/src/image_io.rs`).

## History

The previous session-scoped probes (`spektra_compare.py`, `py_stages.py`,
`py_bisect.py`, `rs_bisect.rs`) hard-coded a developer machine
(`/Users/sasha/...`, `/tmp/cmp/...`) and were superseded by this harness at
commit time. Their findings (print-stage drift root causes: neutral
filters, exposure normalization, spectral integration, curve interpolation)
are now covered by the tap-level comparison and the calibration notes in
`crates/spektrafilm-core/src/pipeline.rs`.
