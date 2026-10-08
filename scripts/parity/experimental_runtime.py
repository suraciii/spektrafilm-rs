#!/usr/bin/env python3
"""Pinned 28bf runtime gate over one deterministic 32x24 RGB fixture.

The gate names all six upstream routes and fails closed on every route error.
"""
from __future__ import annotations
import argparse, hashlib, json, os, subprocess, sys
from pathlib import Path
import numpy as np
import OpenImageIO as oiio
ROOT = Path(__file__).resolve().parents[2]
UP = Path(os.environ.get("SPEKTRAFILM_UPSTREAM", "/tmp/spektrafilm-upstream-28bf"))
PY = Path(os.environ.get("SPEKTRAFILM_PYTHON", sys.executable))
RS = Path(os.environ.get("SPEKTRAFILM_RS_BIN", str(ROOT / "target/release/spektrafilm-f64")))
ROUTES = ("input", "input > film > scan", "input > film > print > scan",
          "input > convert-film > print > scan", "input > convert-film > scan-minus-base",
          "input > convert-film > scan")
BUDGET = 1e-5

def fixture():
    y, x = np.mgrid[0:24, 0:32]
    return np.stack((.06+.65*x/31, .09+.55*y/23, .12+.5*(x+32*y)/(32*24-1)), -1).astype(np.float32)

def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def save_tiff(img, path):
    spec = oiio.ImageSpec(32, 24, 3, oiio.FLOAT)
    output = oiio.ImageOutput.create(str(path))
    if output is None or not output.open(str(path), spec):
        raise RuntimeError(oiio.geterror())
    try:
        output.write_image(img.astype(np.float32))
    finally:
        output.close()

def provenance():
    def rev(p):
        try: return subprocess.check_output(["git","-C",str(p),"rev-parse","HEAD"],text=True).strip()
        except Exception: return None
    data_hash = hashlib.sha256()
    for path in sorted((ROOT/"data").rglob("*")):
        if path.is_file():
            data_hash.update(str(path.relative_to(ROOT/"data")).encode()); data_hash.update(path.read_bytes())
    upstream_commit = rev(UP)
    if upstream_commit != "28bf883e1672e884307edc75852549376e13644e":
        raise RuntimeError(f"upstream checkout is {upstream_commit}, expected pinned 28bf")
    status = subprocess.check_output(["git","-C",str(ROOT),"status","--porcelain"],text=True)
    return {"upstream": str(UP), "upstream_commit": upstream_commit, "rust_repo": str(ROOT),
            "rust_commit": rev(ROOT), "rust_worktree_dirty": bool(status.strip()),
            "rust_binary": str(RS), "rust_binary_sha256": sha(RS) if RS.exists() else None,
            "fixture_sha256": hashlib.sha256(fixture().tobytes()).hexdigest(),
            "data_sha256": data_hash.hexdigest()}

def run_upstream(img, route, out):
    code = r'''import json, numpy as np, sys
sys.path.insert(0, sys.argv[1]); from spektrafilm.runtime.api import init_params, simulate
p=init_params("kodak_portra_400", "kodak_portra_endura"); p.workflow.route=sys.argv[2]
p.settings.use_fast_stats=True; p.io.input_color_space="sRGB"; p.io.output_color_space="sRGB"
p.io.input_cctf_decoding=False; p.io.output_cctf_encoding=False
p.camera.auto_exposure=False; p.film_render.grain.active=False
p.print_render.glare.active=False
from dataclasses import asdict, replace
p.io.output_gamut_compress=replace(p.io.output_gamut_compress, algorithm='oklch', knee=(0.95,1.0,1.6))
from pathlib import Path
snapshot={name:asdict(getattr(p,name)) for name in ('film_render','print_render','camera','enlarger','scanner','io','workflow','settings','debug','taps')}
Path(sys.argv[5]).write_text(json.dumps(snapshot,indent=2))
x=np.fromfile(sys.argv[3],dtype=np.float32).reshape(24,32,3); np.asarray(simulate(x,p),dtype=np.float64).tofile(sys.argv[4])
'''
    inp=out/"input.f32"; inp.write_bytes(img.tobytes()); dst=out/("up-"+str(ROUTES.index(route))+".f64")
    snapshot=out/('up-params-'+str(ROUTES.index(route))+'.json')
    q=subprocess.run([str(PY),'-c',code,str(UP/'src'),route,str(inp),str(dst),str(snapshot)],
                     capture_output=True,text=True,timeout=300)
    (out/('up-'+str(ROUTES.index(route))+'.log')).write_text(q.stdout+'\n'+q.stderr)
    if q.returncode: raise RuntimeError(q.stderr)
    return np.fromfile(dst,dtype=np.float64)

def run_rust(img, route, out):
    inp=out/"input.tif"; save_tiff(img, inp); dst=out/("rs-"+str(ROUTES.index(route))+".f64")
    params={"settings":{"use_fast_stats":True},"workflow":{"route":route},"camera":{"auto_exposure":False},
            "io":{"input_color_space":"sRGB","input_cctf_decoding":False,
                  "output_color_space":"sRGB","output_cctf_encoding":False,
                  "output_gamut_compress":{"algorithm":"oklch","knee":[0.95,1.0,1.6]}},
            "film_render":{"grain":{"active":False}}, "print_render":{"glare":{"active":False}}}
    pf=out/("params-"+str(ROUTES.index(route))+".json"); pf.write_text(json.dumps(params))
    q=subprocess.run([str(RS),"process",str(inp),"-o",str(out/("out-"+str(ROUTES.index(route))+".tif")),
        "--film","kodak_portra_400","--paper","kodak_portra_endura","--params",str(pf),
        '--data-dir',str(ROOT/'data'),'--raw-out',str(dst)],capture_output=True,text=True,timeout=300)
    (out/('rs-'+str(ROUTES.index(route))+'.log')).write_text(q.stdout+'\n'+q.stderr)
    if q.returncode: raise RuntimeError(q.stderr)
    return np.fromfile(dst,dtype=np.float64)

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument('--out', required=True)
    ap.add_argument('--development-smoke', action='store_true')
    a=ap.parse_args(); out=Path(a.out).resolve()
    if out.exists() and any(out.iterdir()): raise SystemExit(f"refusing non-empty output directory: {out}")
    out.mkdir(parents=True,exist_ok=True)
    img=fixture(); report={"provenance":provenance(),"budget_max_abs":BUDGET,"routes":[]}; failed=False
    if report['provenance']['rust_worktree_dirty'] and not a.development_smoke:
        raise SystemExit('Commit implementation before recording runtime acceptance')
    for route in ROUTES:
        row={"route":route}
        try:
            u=run_upstream(img,route,out); r=run_rust(img,route,out)
            if u.shape != (24*32*3,) or r.shape != u.shape:
                raise RuntimeError(f'Unexpected output shapes upstream={u.shape}, rust={r.shape}')
            if not np.isfinite(u).all() or not np.isfinite(r).all():
                raise RuntimeError('Nonfinite upstream/Rust output')
            d=np.abs(u-r); ok=float(d.max())<=BUDGET
            row.update(status="pass" if ok else "fail",max_abs=float(d.max()),mean_abs=float(d.mean()),n=int(d.size),
                       upstream_sha256=sha(out/("up-"+str(ROUTES.index(route))+".f64")),
                       rust_sha256=sha(out/("rs-"+str(ROUTES.index(route))+".f64")),
                       upstream_params_sha256=sha(out/('up-params-'+str(ROUTES.index(route))+'.json')),
                       params_sha256=sha(out/("params-"+str(ROUTES.index(route))+".json")))
            failed |= not ok
        except Exception as e: row.update(status="error",reason=str(e)); failed=True
        report["routes"].append(row)
    after = provenance()
    report['provenance_after'] = after
    if not a.development_smoke and after != report['provenance']:
        report['provenance_error'] = 'Implementation, executable or data changed during run'
        failed = True
    report['status'] = 'fail' if failed else ('development-smoke' if a.development_smoke else 'pass')
    (out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))
    return 1 if failed else 0
if __name__ == "__main__": raise SystemExit(main())
