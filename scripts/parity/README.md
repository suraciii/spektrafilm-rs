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

Evidence, budgets and provenance live in
[`docs/parity/baseline_evidence.md`](../../docs/parity/baseline_evidence.md);
the machine-readable inventory is
[`docs/parity/parity_matrix.json`](../../docs/parity/parity_matrix.json).

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


All three package jobs run `package_smoke.py` against installed executables
and bundled data. The native GUI driver operates real windows and file dialogs,
records screenshots, saves/restores state across restart, decodes RAW, saves a
float image, observes the bundled f64 exporter, and checks Cancel/window-close
child cleanup. Linux uses Xvfb with a 1600×1000 screen. Windows/macOS acceptance
requires actual successful workflow execution; installing a driver is not
evidence that those packages run.

Desktop acceptance requires Pillow, mss, pytesseract, psutil and the Tesseract
engine. Linux additionally requires openbox, xdotool, xclip, xprop, xwininfo and zenity. Package
smoke also builds a ZIP bundle through the installed exporter, checks offline
report references and artifacts, and executes its delivered OCIO processors.
`--raw-fixture PATH` reuses the pinned Kodak KDC download when network access
is unavailable; the same required SHA256 check runs before decoding.


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
