#!/usr/bin/env python3
"""Offline differential orchestrator: pinned Python 0.3.4 vs Rust f64 CLI.

Drives both implementations over identical input fixtures and reports
per-stage / final max & mean errors per scenario against the budgets in
``scenarios.py``.

    python3 scripts/parity/run_parity.py                    # full matrix
    python3 scripts/parity/run_parity.py --list             # catalog
    python3 scripts/parity/run_parity.py --only bare_chain_4x4
    python3 scripts/parity/run_parity.py --rust-only        # fixtures from disk
    python3 scripts/parity/run_parity.py --record-unsupported

Prerequisites (all checked up front; any miss is a hard error listing the
exact fix — never a silent skip):

  * Python venv with the 0.3.4 deps + editable install of the pinned repo
    (SPEKTRAFILM_PY, default /tmp/spektrafilm-034-venv/bin/python);
  * reference checkout at 3bb2c2d2801ff68b92019cf1dbcbb133d60832bc
    (SPEKTRAFILM_PY_REPO, default /home/szf/repos/spektrafilm);
  * Rust f64 CLI binary (SPEKTRAFILM_RS_BIN, default
    target/release/spektrafilm-f64; build with
    `cargo build -p spektrafilm-cli --features precision-f64 \
     --bin spektrafilm-f64 --release`).
"""

from __future__ import annotations

import argparse
import json
import math
import os
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from scenarios import (  # noqa: E402
    BUDGETS,
    PY_COMMIT,
    PY_VERSION,
    RS_COMMIT,
)

REPO_ROOT = HERE.parent.parent
PY_REPO = Path(os.environ.get("SPEKTRAFILM_PY_REPO", "/home/szf/repos/spektrafilm"))
PY_BIN = Path(os.environ.get("SPEKTRAFILM_PY", "/tmp/spektrafilm-034-venv/bin/python"))
RS_BIN = Path(os.environ.get(
    "SPEKTRAFILM_RS_BIN",
    str(REPO_ROOT / "target" / "release" / "spektrafilm-f64"),
))

TAP_TO_RS = {
    "log_e_film": ("env", "SPEKTRAFILM_DUMP_FILM_LOG_RAW"),
    "cmy_film": ("env", "SPEKTRAFILM_DUMP_FILM_DENSITY"),
    "log_e_print": ("env", "SPEKTRAFILM_DUMP_PRINT_LOG_RAW"),
    "cmy_print": ("env", "SPEKTRAFILM_DUMP_PRINT_DENSITY"),
    "rgb_out": ("flag", "--raw-out"),
}


class Missing(Exception):
    """A prerequisite is absent; the payload says how to fix it."""


def preflight_python() -> dict:
    if not PY_BIN.exists():
        raise Missing(
            f"pinned Python venv interpreter not found at {PY_BIN}. "
            f"Create it with the commands in scripts/parity/README.md "
            f"(§ Reference environment) or point SPEKTRAFILM_PY at an "
            f"existing 0.3.4 venv."
        )
    if not PY_REPO.exists():
        raise Missing(f"Python reference repo not found at {PY_REPO} "
                      f"(set SPEKTRAFILM_PY_REPO).")
    head = subprocess.run(["git", "-C", str(PY_REPO), "rev-parse", "HEAD"],
                          capture_output=True, text=True)
    if head.returncode != 0:
        raise Missing(f"cannot read git HEAD of {PY_REPO}: {head.stderr.strip()}")
    if head.stdout.strip() != PY_COMMIT:
        raise Missing(
            f"{PY_REPO} is at {head.stdout.strip()}, expected {PY_COMMIT} "
            f"(spektrafilm {PY_VERSION}); git -C {PY_REPO} checkout {PY_COMMIT}"
        )
    probe = subprocess.run(
        [str(PY_BIN), "-c",
         "import numpy, scipy, colour, OpenImageIO, spektrafilm, sys; "
         "from spektrafilm import __file__ as f; print(f)"],
        capture_output=True, text=True,
    )
    if probe.returncode != 0:
        raise Missing(
            f"pinned venv {PY_BIN} cannot import the 0.3.4 runtime stack "
            f"(numpy/scipy/colour/OpenImageIO/spektrafilm): "
            f"{probe.stderr.strip().splitlines()[-1] if probe.stderr.strip() else 'no stderr'}. "
            f"Finish the venv setup from scripts/parity/README.md — do not "
            f"downgrade the check."
        )
    src = probe.stdout.strip()
    if not src.startswith(str(PY_REPO)):
        raise Missing(
            f"the importable spektrafilm resolves to {src}, not the pinned "
            f"checkout {PY_REPO}; reinstall with "
            f"{PY_BIN} -m pip install --no-deps -e {PY_REPO}"
        )
    provenance = subprocess.run(
        [str(PY_BIN), "-c",
         "import contextlib, io, json, platform, importlib.metadata as m, numpy; "
         "s=io.StringIO(); "
         "exec('with contextlib.redirect_stdout(s):\\n numpy.show_config()'); "
         "print(json.dumps(dict(os=platform.platform(), architecture=platform.machine(), "
         "python_version=platform.python_version(), dependencies={d.metadata['Name']:d.version for d in m.distributions()}, "
         "numpy_blas=s.getvalue())))"],
        capture_output=True, text=True, check=True,
    )
    return {"python": str(PY_BIN), "py_repo": str(PY_REPO), "py_spektrafilm": src,
            "reference_environment": json.loads(provenance.stdout)}


def preflight_rust() -> dict:
    if not RS_BIN.exists():
        raise Missing(
            f"Rust f64 CLI not found at {RS_BIN}. Build it with: "
            f"cargo build -p spektrafilm-cli --features precision-f64 "
            f"--bin spektrafilm-f64 --release   (run from {REPO_ROOT})"
        )
    return {"rust_bin": str(RS_BIN)}


def expand_scenarios() -> list[dict]:
    """Shared expansion from scenarios.py (profile sweeps -> concrete rows)."""
    from scenarios import expand_scenarios as _expand
    return _expand(PY_REPO)


def nested_params(flat: dict) -> dict:
    """Forward canonical dotted controls into strict Rust params JSON."""
    tree: dict = {}
    for path, value in flat.items():
        parts = path.split(".")
        node = tree
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = list(value) if isinstance(value, (list, tuple)) else value
    return tree


def read_f64(path: Path) -> list[float]:
    data = path.read_bytes()
    if len(data) % 8:
        raise ValueError(f"truncated f64 dump: {path} ({len(data)} bytes)")
    n = len(data) // 8
    return list(struct.unpack(f"<{n}d", data[: n * 8]))


def compare(py: list[float], rs: list[float]) -> dict:
    if len(py) != len(rs):
        return {"error": f"length mismatch: py={len(py)} rs={len(rs)}"}
    if not py:
        return {"error": "empty arrays"}
    if not all(math.isfinite(v) for values in (py, rs) for v in values):
        return {"error": "non-finite values in comparison arrays"}
    diffs = [abs(a - b) for a, b in zip(py, rs)]
    n = len(diffs)
    return {
        "n": n,
        "max_abs": max(diffs),
        "mean_abs": sum(diffs) / n,
    }


def compare_statistical(py: list[float], rs: list[float]) -> dict:
    if not py or len(py) != len(rs):
        return {"error": f"invalid statistical array sizes: py={len(py)} rs={len(rs)}"}
    if not all(math.isfinite(v) for values in (py, rs) for v in values):
        return {"error": "non-finite values in statistical arrays"}
    def stats(xs):
        n = len(xs)
        mean = sum(xs) / n
        var = sum((x - mean) ** 2 for x in xs) / n
        return mean, math.sqrt(var)

    pm, ps = stats(py)
    rm, rs_ = stats(rs)
    return {
        "n": len(py),
        "py_mean": pm, "rs_mean": rm,
        "py_std": ps, "rs_std": rs_,
        "mean_rel": abs(rm - pm) / max(abs(pm), 1e-12),
        "std_rel": abs(rs_ - ps) / max(ps, 1e-12),
        "max_abs": max(abs(a - b) for a, b in zip(py, rs)) if py else None,
    }


def budget_verdict(budget: dict, metrics: dict) -> tuple[bool, str]:
    if "error" in metrics:
        return False, metrics["error"]
    if budget.get("max_abs") is None:
        ok = (metrics["mean_rel"] <= budget["mean_rel"]
              and metrics["std_rel"] <= budget["std_rel"])
        return ok, (f"mean_rel={metrics['mean_rel']:.3e} "
                    f"std_rel={metrics['std_rel']:.3e}")
    ok = metrics["max_abs"] <= budget["max_abs"]
    return ok, (f"max_abs={metrics['max_abs']:.3e} "
                f"(budget {budget['max_abs']:.1e})")


def run_rust(scn: dict, out_dir: Path, fixtures: Path) -> subprocess.CompletedProcess:
    # Archive only harness-owned outputs before this invocation. A stale
    # dump must never satisfy a missing tap or an expected refusal.
    prior = [path for path in out_dir.glob("rs_*.f64") if path.is_file()]
    if (out_dir / "rs_out.png").exists():
        prior.append(out_dir / "rs_out.png")
    if prior:
        import tempfile
        archive = Path(tempfile.mkdtemp(prefix="spektrafilm-parity-history-"))
        for path in prior:
            path.rename(archive / path.name)
    params_tree = nested_params(scn["params"])
    params_file = out_dir / "rs_params.json"
    params_file.write_text(json.dumps(params_tree, indent=2))

    cmd = [
        str(RS_BIN), "process",
        str(fixtures),
        "-o", str(out_dir / "rs_out.png"),
        "--film", scn["film"],
        "--paper", scn["paper"],
        "--params", str(params_file),
        "--data-dir", str(REPO_ROOT / "data"),
    ]
    if scn["params"].get("io.scan_film"):
        cmd.append("--scan-film")
    for tap in scn["taps"]:
        kind, key = TAP_TO_RS[tap]
        if kind == "env":
            continue
        cmd += [key, str(out_dir / "rs_rgb_out.f64")]

    env = dict(os.environ)
    env["SPEKTRAFILM_BACKEND"] = "cpu"
    for tap in scn["taps"]:
        kind, key = TAP_TO_RS[tap]
        if kind == "env":
            env[key] = str(out_dir / f"rs_{tap}.f64")
    # Every assigned dump remains enabled; missing taps fail the scenario.
    return subprocess.run(cmd, capture_output=True, text=True, env=env,
                          cwd=str(REPO_ROOT))


def run_python(rows: list[dict], out_root: Path) -> None:
    """Materialize the expanded rows for the pinned driver and run it."""
    rows_file = out_root / "rows.json"
    rows_file.parent.mkdir(parents=True, exist_ok=True)
    rows_file.write_text(json.dumps(rows, indent=2))
    subprocess.run(
        [str(PY_BIN), str(HERE / "py_reference.py"),
         "--repo", str(PY_REPO), "--out-dir", str(out_root),
         "--rows-file", str(rows_file)],
        check=True,
    )


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--list", action="store_true", help="print the catalog")
    ap.add_argument("--only", help="comma-separated scenario names")
    ap.add_argument("--out-root", default="target/parity")
    ap.add_argument("--rust-only", action="store_true",
                    help="skip Python; compare against existing fixtures")
    ap.add_argument("--record-unsupported", action="store_true",
                    help="write explicit unsupported evidence and exit 2 "
                         "when prerequisites are missing")
    args = ap.parse_args()

    rows = expand_scenarios()
    if args.only:
        wanted = {n.strip() for n in args.only.split(",")}
        unknown = wanted - {r["name"] for r in rows}
        if unknown:
            sys.exit(f"unknown scenarios: {sorted(unknown)}")
        rows = [r for r in rows if r["name"] in wanted]
    if args.list:
        for r in rows:
            print(f"{r['name']:44s} {r['status']:16s} {r['budget']}")
        return

    out_root = Path(args.out_root)
    fixtures_root = out_root / "fixtures"

    env_info, unsupported = {}, []
    try:
        env_info.update(preflight_python())
    except Missing as m:
        unsupported.append({"prerequisite": "python", "detail": str(m)})
    try:
        env_info.update(preflight_rust())
    except Missing as m:
        unsupported.append({"prerequisite": "rust", "detail": str(m)})

    if unsupported:
        report = {
            "pins": {"python_commit": PY_COMMIT, "python_version": PY_VERSION,
                     "rust_baseline_commit": RS_COMMIT},
            "environment": env_info,
            "unsupported": unsupported,
            "rows": [
                {"scenario": r["name"], "status": "unsupported",
                 "reason": "prerequisite missing (see unsupported[])",
                 "owner_issue": r["owner_issue"]}
                for r in rows
            ],
        }
        out_root.mkdir(parents=True, exist_ok=True)
        (out_root / "parity_report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps(unsupported, indent=2))
        sys.exit(2)

    # ---- run Python side ----------------------------------------------------
    if not args.rust_only:
        run_python(rows, fixtures_root)

    # ---- run Rust + compare -------------------------------------------------
    from scenarios import scenario_dir_name

    results = []
    failures = []
    for scn in rows:
        name = scn["name"]
        fdir = fixtures_root / scenario_dir_name(name)

        if scn.get("expect_reject"):
            # Parity of refusal: the pinned Python rejected this
            # configuration; Rust must refuse it too (actionable error,
            # no artifact) — silently rendering it is a real gap.
            proc = run_rust(scn, fdir, fdir / "input.tif")
            rejected = proc.returncode != 0
            detail = (proc.stderr.strip().splitlines()[-1]
                      if proc.stderr.strip() else f"exit {proc.returncode}")
            artifact = (fdir / "rs_out.png").exists()
            ok = rejected and not artifact
            if not ok:
                failures.append(
                    f"{name}: Rust accepted a configuration the pinned "
                    f"Python rejects (paper-support stock as film)")
            results.append({
                "scenario": name,
                "film": scn["film"],
                "paper": scn["paper"],
                "budget": scn["budget"],
                "declared_status": scn["status"],
                "audit_baseline": scn.get("audit_baseline"),
                "owner_issue": scn["owner_issue"],
                "status": "pass" if ok else "FAIL",
                "python_rejected": True,
                "rust_rejected": rejected,
                "rust_detail": detail,
                "artifact_produced": artifact,
            })
            continue

        fixture_tiff = fdir / "input.tif"
        if not fixture_tiff.exists():
            failures.append(f"{name}: python fixture missing at {fixture_tiff}")
            results.append({"scenario": name, "status": "fixture_missing"})
            continue

        proc = run_rust(scn, fdir, fixture_tiff)
        if "spectral LUT not available" in proc.stderr:
            failures.append(
                f"{name}: Rust silently fell back to the simplified "
                f"non-spectral pipeline — parity run is invalid")
            results.append({"scenario": name, "status": "invalid_fallback"})
            continue
        if proc.returncode != 0:
            failures.append(f"{name}: rust CLI failed: "
                            f"{proc.stderr.strip().splitlines()[-1:]}")
            results.append({"scenario": name, "status": "rust_error"})
            continue


        budget = BUDGETS[scn["budget"]]
        tap_rows = {}
        scn_ok = True
        for tap in scn["taps"]:
            py_path = fdir / f"py_{tap}.f64"
            rs_path = fdir / f"rs_{tap}.f64"
            if not py_path.exists() or not rs_path.exists():
                tap_rows[tap] = {"error": f"missing dump: {py_path} / {rs_path}"}
                scn_ok = False
                continue
            py, rs = read_f64(py_path), read_f64(rs_path)
            if scn["budget"] == "statistical_texture":
                metrics = compare_statistical(py, rs)
                ok, why = budget_verdict(budget, metrics)
            else:
                metrics = compare(py, rs)
                ok, why = budget_verdict(budget, metrics)
            tap_rows[tap] = {**metrics, "within_budget": ok, "detail": why}
            if not ok:
                scn_ok = False

        status = "pass" if scn_ok else "FAIL"
        if not scn_ok:
            failures.append(f"{name}: budget exceeded ({scn['budget']})")
        results.append({
            "scenario": name,
            "film": scn["film"],
            "paper": scn["paper"],
            "budget": scn["budget"],
            "declared_status": scn["status"],
            "audit_baseline": scn.get("audit_baseline"),
            "owner_issue": scn["owner_issue"],
            "status": status,
            "taps": tap_rows,
        })

    report = {
        "pins": {"python_commit": PY_COMMIT, "python_version": PY_VERSION,
                 "rust_baseline_commit": RS_COMMIT},
        "environment": env_info,
        "unsupported": [],
        "rows": results,
    }
    out_root.mkdir(parents=True, exist_ok=True)
    (out_root / "parity_report.json").write_text(json.dumps(report, indent=2))

    # ---- console summary ----------------------------------------------------
    print(f"\n{'scenario':44s} {'status':18s} final-tap detail")
    for r in results:
        final = next((r["taps"][t].get("detail", "?")
                      for t in reversed(list(r.get("taps", {}))) ), "")
        print(f"{r['scenario']:44s} {r['status']:18s} {final}")
    if failures:
        print("\nFAILURES:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print(f"\nreport: {out_root / 'parity_report.json'}")


if __name__ == "__main__":
    main()
