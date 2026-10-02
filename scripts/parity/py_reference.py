"""Pinned Python 0.3.4 reference driver for the parity harness.

Generates the shared input fixtures and per-tap f64 dumps using ONLY the
core ``spektrafilm`` package (the physical runtime). The GUI
(``spektrafilm_gui``) and LUT creator (``spektrafilm_lut_creator``) are
never imported — that is asserted at exit.

Run inside the pinned venv:

    /tmp/spektrafilm-034-venv/bin/python scripts/parity/py_reference.py \
        --repo /home/szf/repos/spektrafilm \
        --out-dir target/parity/fixtures --all

Fails hard (non-zero exit, no fixtures written for the scenario) when:
  * the reference repo is not checked out at the pinned commit
    3bb2c2d2801ff68b92019cf1dbcbb133d60832bc (0.3.4);
  * the importable ``spektrafilm`` does not come from that repo;
  * a scenario names an unknown tap or a param path that does not exist.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from scenarios import (  # noqa: E402
    INPUTS,
    PY_COMMIT,
    PY_VERSION,
    SCENARIOS,
    scenario_dir_name,
)

FORBIDDEN_MODULES = ("spektrafilm_gui", "spektrafilm_lut_creator")


def _fail(msg: str) -> None:
    print(f"[py_reference] ERROR: {msg}", file=sys.stderr)
    raise SystemExit(1)


def check_pin(repo: Path) -> None:
    head = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    if head != PY_COMMIT:
        _fail(
            f"reference repo {repo} is at {head}, expected {PY_COMMIT} "
            f"(spektrafilm {PY_VERSION}). Fix with: git -C {repo} checkout "
            f"{PY_COMMIT}"
        )


def make_input(spec: dict):
    import numpy as np

    w, h = spec["width"], spec["height"]
    if spec["kind"] == "mod37":
        data = np.array(
            [0.05 + 0.5 * ((i * 37) % 256) / 255.0 for i in range(w * h * 3)],
            dtype=np.float64,
        ).reshape(h, w, 3)
    elif spec["kind"] == "const":
        data = np.full((h, w, 3), spec["value"], dtype=np.float64)
    else:
        _fail(f"unknown input kind {spec['kind']!r}")
    return data


def write_input_tiff(arr, path: Path) -> None:
    import numpy as np
    import OpenImageIO as oiio

    small = arr.astype(np.float32)
    spec = oiio.ImageSpec(small.shape[1], small.shape[0], 3, "float")
    spec.attribute("oiio:ColorSpace", "linear")
    buf = oiio.ImageBuf(spec)
    buf.set_pixels(oiio.ROI(0, small.shape[1], 0, small.shape[0]), small)
    buf.write(str(path))


def read_input_tiff(path: Path):
    import numpy as np
    import OpenImageIO as oiio

    inp = oiio.ImageBuf(str(path))
    spec = inp.spec()
    return np.array(inp.get_pixels(oiio.TypeDesc("float"))).reshape(
        spec.height, spec.width, 3
    ).astype(np.float64)


def _with_field(obj, name: str, value):
    """Set ``name`` on ``obj``, honoring frozen dataclasses via replace."""
    import dataclasses

    if dataclasses.is_dataclass(obj) and obj.__dataclass_params__.frozen:
        # The gamut specs are @dataclass(frozen=True); rebuild them (and
        # re-attach to the parent chain) with dataclasses.replace so the
        # freeze is respected — never circumvented via object.__setattr__.
        # replace() re-runs __post_init__, so validation still applies.
        return dataclasses.replace(obj, **{name: value})
    setattr(obj, name, value)
    return obj


def set_param(params, path: str, value):
    """Return params with ``path`` set to ``value`` (root may be rebuilt)."""
    parts = path.split(".")

    def walk(obj, remaining):
        leaf = remaining[0]
        if not hasattr(obj, leaf):
            _fail(f"param path {path!r} does not exist on the pinned 0.3.4 schema")
        if len(remaining) == 1:
            current = getattr(obj, leaf)
            if isinstance(current, tuple):
                value_ = tuple(value)
            else:
                value_ = value
            return _with_field(obj, leaf, value_)
        return _with_field(obj, leaf, walk(getattr(obj, leaf), remaining[1:]))

    return walk(params, parts)


def dump_f64(name: str, arr, out_dir: Path) -> Path:
    import numpy as np

    path = out_dir / f"py_{name}.f64"
    np.ascontiguousarray(arr, dtype=np.float64).ravel().tofile(path)
    return path


def run_scenario(scn: dict, repo: Path, out_root: Path) -> dict:
    import numpy as np
    from spektrafilm import digest_params, init_params
    from spektrafilm.runtime.pipeline import SimulationPipeline
    from spektrafilm.runtime.topology import Tap

    out_dir = out_root / scenario_dir_name(scn["name"])
    out_dir.mkdir(parents=True, exist_ok=True)

    film, paper = scn["film"], scn["paper"]
    if film is None:  # expanded sweep rows carry concrete names already
        _fail(f"scenario {scn['name']!r} needs expansion before the driver")

    params = init_params(film, paper)
    for path, value in scn["params"].items():
        params = set_param(params, path, value)
    params = digest_params(params)

    if scn.get("expect_reject"):
        # Write the shared fixture so the Rust run has a real input and its
        # verdict reflects the film choice alone, not a missing file.
        img = make_input(INPUTS[scn["input"]])
        tiff = out_dir / "input.tif"
        write_input_tiff(img, tiff)
        # The pinned reference must refuse this configuration (e.g. a
        # paper-support stock used as the film raises ValueError in
        # eval_erf4_spectral_bandpass). Record the rejection as evidence;
        # if upstream accepts it the catalog is wrong — fail loudly.
        try:
            SimulationPipeline(params)
        except Exception as exc:  # noqa: BLE001 — any rejection is evidence
            return {
                "scenario": scn["name"],
                "film": film,
                "paper": paper,
                "input_tiff": str(tiff),
                "python_rejected": f"{type(exc).__name__}: {exc}",
                "params": scn["params"],
            }
        _fail(f"scenario {scn['name']!r} expected upstream rejection but "
              f"the pinned reference accepted the configuration")

    img = make_input(INPUTS[scn["input"]])

    tiff = out_dir / "input.tif"
    write_input_tiff(img, tiff)
    # Consume the stored fixture so Python and Rust see bit-identical input
    # (float32 rounding included).
    img = read_input_tiff(tiff)

    pipe = SimulationPipeline(params)

    tap_values = {}
    for tap_name in scn["taps"]:
        const = getattr(Tap, tap_name.upper(), None)
        if const is None:
            _fail(f"unknown tap {tap_name!r} (see runtime/topology.py::Tap)")
        tap_values[tap_name] = const

    produced = {}
    for tap_name, tap_value in tap_values.items():
        # Pin the ambient np.random stream before each collect run: grain
        # re-seeds internally (seeds [0,1,2]) but glare draws from the
        # ambient stream, so without this pin repeated collects of
        # stochastic scenarios would drift. This is a harness determinism
        # choice — the Rust glare RNG divergence (fixed seed 42) stays a
        # recorded gap either way.
        np.random.seed(0)
        result = pipe.process(img, collect=tap_value)
        produced[tap_name] = str(dump_f64(tap_name, result, out_dir))


    return {
        "scenario": scn["name"],
        "film": film,
        "paper": paper,
        "input_tiff": str(tiff),
        "taps": produced,
        "params": scn["params"],
    }


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--repo", default="/home/szf/repos/spektrafilm",
                    help="pinned Python reference checkout")
    ap.add_argument("--out-dir", default="target/parity/fixtures",
                    help="fixture output root")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--all", action="store_true")
    g.add_argument("--scenarios", help="comma-separated scenario names")
    g.add_argument("--rows-file", dest="rows_file", default=None)
    args = ap.parse_args()

    repo = Path(args.repo).resolve()
    if not repo.exists():
        _fail(f"reference repo {repo} does not exist")
    check_pin(repo)

    out_root = Path(args.out_dir)
    out_root.mkdir(parents=True, exist_ok=True)

    if args.rows_file:
        rows = json.loads(Path(args.rows_file).read_text())
    else:
        names = None
        if args.scenarios:
            names = [n.strip() for n in args.scenarios.split(",") if n.strip()]
        from scenarios import expand_scenarios
        rows = [r for r in expand_scenarios(repo)
                if names is None or r["name"] in names]

    manifest = []
    for scn in rows:
        if scn.get("expand"):
            _fail(f"row {scn['name']!r} still carries an expansion marker")
        info = run_scenario(scn, repo, out_root)
        manifest.append(info)
        if "python_rejected" in info:
            print(f"[py_reference] {scn['name']}: reference rejected "
                  f"({info['python_rejected'].split(':')[0]}) — evidence recorded")
        else:
            print(f"[py_reference] {scn['name']}: "
                  f"{len(info['taps'])} taps dumped to "
                  f"{out_root / scenario_dir_name(scn['name'])}")

    (out_root / "py_manifest.json").write_text(json.dumps(manifest, indent=2))

    polluted = [m for m in FORBIDDEN_MODULES if m in sys.modules]
    if polluted:
        _fail(f"core-only contract violated; imported: {polluted}")
    print(f"[py_reference] OK — core-only imports, {len(manifest)} scenarios")


if __name__ == "__main__":
    main()
