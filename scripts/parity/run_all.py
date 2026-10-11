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
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from build_evidence import load_manifest, verify_manifest, executable_path

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




def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-manifest', type=Path, required=True)
    parser.add_argument('--package-manifest', type=Path,
                        help='Packaged build-manifest.json (defaults to package root)')
    parser.add_argument('--out-root', type=Path, default=ROOT / 'target/acceptance')
    parser.add_argument("--package-root", type=Path, required=True,
                        help="installed/portable package root used by package_smoke.py")
    parser.add_argument("--raw-fixture", type=Path)
    args = parser.parse_args()

    manifest = load_manifest(args.build_manifest)
    rust_bin = Path(manifest['binaries']['exporter']['path'])
    data_dir = Path(manifest['data_dir'])
    package_root = args.package_root.resolve()
    out_root = args.out_root.resolve()
    package_manifest_path = args.package_manifest or package_root / 'build-manifest.json'
    if not package_manifest_path.exists() and package_root.suffix == '.app' and not args.package_manifest:
        package_manifest_path = package_root.parent / 'build-manifest.json'
    packaged = load_manifest(package_manifest_path)
    if packaged['source'] != manifest['source']:
        raise SystemExit('Package and parity build sources differ')
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

    cli = executable_path(packaged, 'cli')
    exporter = executable_path(packaged, 'exporter')
    raw_helper = executable_path(packaged, 'raw_helper')
    gui = executable_path(packaged, 'gui')
    package_data = Path(packaged['data_dir'])
    package_command = [
        env["SPEKTRAFILM_PY"], str(HERE / "package_smoke.py"),
        '--build-manifest', str(package_manifest_path),
        "--cli", str(cli), "--exporter", str(exporter),
        "--raw-helper", str(raw_helper), "--gui", str(gui),
        "--data-dir", str(package_data),
        "--package-root", str(package_root),
        "--evidence-dir", str(out_root / "package"),
    ]
    if args.raw_fixture:
        package_command += ["--raw-fixture", str(args.raw_fixture.resolve())]
    checked("package smoke", package_command, "package-smoke.log")
    package_report_path = out_root / "package" / "package_report.json"
    if not package_report_path.is_file():
        raise SystemExit(f"package smoke omitted provenance: {package_report_path}")
    package_report = json.loads(package_report_path.read_text(encoding="utf-8"))
    verify_manifest(manifest)
    verify_manifest(packaged)
    if package_report.get('source_revision') != manifest['source_revision']:
        raise SystemExit('Package smoke source does not match the parity build')
    report = {"status": "pass", "rust_bin": str(rust_bin),
              "rust_bin_sha256": hashlib.sha256(rust_bin.read_bytes()).hexdigest(),
              "rust_commit": manifest['source']['commit'],
              "rust_worktree_dirty": bool(manifest['source']['status']),
              "source": manifest['source'], "source_revision": manifest['source_revision'],
              "build": manifest,
              "platform": platform.platform(),
              "reference_commit": "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc",
              "data_dir": str(data_dir), "package_root": str(package_root),
              "package_provenance": {
                  "report": str(package_report_path),
                  "sha256": hashlib.sha256(package_report_path.read_bytes()).hexdigest(),
                  "package_tree_sha256": package_report["package_tree_sha256"],
                  "binaries": package_report["binaries"],
                  "scenario_count": package_report["scenario_count"],
              },
              "commands": commands}
    (out_root / "report.json").write_text(json.dumps(report, indent=2) + "\n",
                                           encoding="utf-8")
    print(f"PASS: unified acceptance report: {out_root / 'report.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
