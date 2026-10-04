#!/usr/bin/env python3
"""Run the small, strict Rust-vs-Python acceptance gate.

The wrapper intentionally only orchestrates existing checks. Build and package
steps remain platform-specific; pass the resulting package executables with
``--package-root`` (or explicit executable flags).
"""
from __future__ import annotations

import argparse
import json
import hashlib
import os
from pathlib import Path
import subprocess
import platform
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def _run(label: str, command: list[str], env: dict[str, str], log: Path) -> None:
    log.parent.mkdir(parents=True, exist_ok=True)
    print(f"== {label} ==")
    print("$", " ".join(command))
    with log.open("w", encoding="utf-8") as stream:
        result = subprocess.run(command, cwd=ROOT, env=env, stdout=stream,
                                stderr=subprocess.STDOUT)
    if result.returncode:
        raise SystemExit(f"{label} failed with exit {result.returncode}; see {log}")


def _find(root: Path, names: tuple[str, ...]) -> Path:
    for name in names:
        candidate = root / name
        if candidate.is_file():
            return candidate
    raise SystemExit(f"package executable not found under {root}: {', '.join(names)}")


def _find_data(root: Path) -> Path:
    for name in ("data", "share/data", "Contents/Resources/data"):
        candidate = root / name
        if candidate.is_dir():
            return candidate
    raise SystemExit(f"packaged data directory not found under {root}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-bin", type=Path,
                        default=ROOT / "target/release/spektrafilm-f64")
    parser.add_argument("--data-dir", type=Path, default=ROOT / "data")
    parser.add_argument("--out-root", type=Path, default=ROOT / "target/acceptance")
    parser.add_argument("--package-root", type=Path, required=True,
                        help="installed/portable package root used by package_smoke.py")
    parser.add_argument("--raw-fixture", type=Path)
    args = parser.parse_args()

    rust_bin = args.rust_bin.resolve()
    data_dir = args.data_dir.resolve()
    package_root = args.package_root.resolve()
    out_root = args.out_root.resolve()
    if not rust_bin.is_file():
        raise SystemExit(f"Rust f64 CLI not found: {rust_bin}")
    if not data_dir.is_dir():
        raise SystemExit(f"data directory not found: {data_dir}")
    if not package_root.is_dir():
        raise SystemExit(f"package root not found: {package_root}")

    env = dict(os.environ)
    env.setdefault("SPEKTRAFILM_RS_BIN", str(rust_bin))
    env.setdefault("SPEKTRAFILM_BACKEND", "cpu")
    env.setdefault("SPEKTRAFILM_PY", "/tmp/spektrafilm-034-venv/bin/python")
    env.setdefault("SPEKTRAFILM_PY_REPO", str(ROOT.parent / "spektrafilm"))
    out_root.mkdir(parents=True, exist_ok=True)
    commands: list[dict[str, object]] = []

    def checked(label: str, command: list[str], log_name: str) -> None:
        _run(label, command, env, out_root / log_name)
        commands.append({"label": label, "command": command, "status": "pass"})

    checked("runtime parity", [
        sys.executable, str(HERE / "run_parity.py"),
        "--out-root", str(out_root / "parity"),
    ], "runtime-parity.log")
    checked("LUT acceptance", [
        sys.executable, str(HERE / "lut_acceptance.py"),
        "--cli", str(rust_bin), "--data-dir", str(data_dir),
        "--evidence-dir", str(out_root / "lut"),
    ], "lut-acceptance.log")

    cli = _find(package_root, (
        "spektrafilm.exe", "spektrafilm", "bin/spektrafilm",
        "Contents/MacOS/spektrafilm",
    ))
    exporter = _find(package_root, (
        "spektrafilm-f64.exe", "spektrafilm-f64", "bin/spektrafilm-f64",
        "Contents/MacOS/spektrafilm-f64",
    ))
    raw_helper = _find(package_root, (
        "decode_raw_gui.exe", "decode_raw_gui", "bin/decode_raw_gui",
        "Contents/MacOS/decode_raw_gui",
    ))
    gui = _find(package_root, (
        "spektrafilm-gui.exe", "spektrafilm-gui", "bin/spektrafilm-gui",
        "Contents/MacOS/spektrafilm-gui",
    ))
    package_data = _find_data(package_root)
    package_command = [
        env["SPEKTRAFILM_PY"], str(HERE / "package_smoke.py"),
        "--cli", str(cli), "--exporter", str(exporter),
        "--raw-helper", str(raw_helper), "--gui", str(gui),
        "--data-dir", str(package_data),
        "--evidence-dir", str(out_root / "package"),
    ]
    if args.raw_fixture:
        package_command += ["--raw-fixture", str(args.raw_fixture.resolve())]
    checked("package smoke", package_command, "package-smoke.log")

    report = {"status": "pass", "rust_bin": str(rust_bin),
              "rust_bin_sha256": hashlib.sha256(rust_bin.read_bytes()).hexdigest(),
              "rust_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "rust_worktree_dirty": bool(subprocess.check_output(["git", "diff", "--name-only", "HEAD"], cwd=ROOT, text=True).strip()),
              "platform": platform.platform(),
              "reference_commit": "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc",
              "data_dir": str(data_dir), "package_root": str(package_root),
              "commands": commands}
    (out_root / "report.json").write_text(json.dumps(report, indent=2) + "\n",
                                           encoding="utf-8")
    print(f"PASS: unified acceptance report: {out_root / 'report.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
