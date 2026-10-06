#!/usr/bin/env python3
"""Strict artifact acceptance against the editable, pinned Python 0.3.4 checkout.

Run with the reference Python interpreter (or set SPEKTRAFILM_PY). The CLI
must have been built with precision-f64; its release/deps directory is used
by rustc for the public format API probe, since 3DL/Hald have no CLI flags.
"""
from __future__ import annotations

import argparse
import dataclasses
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import traceback
from urllib.parse import unquote, urlsplit
import zipfile

PIN = "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc"
FILM = "kodak_portra_400"
PRINTS = ("kodak_portra_endura", "kodak_2383")
THRESHOLDS = {
    "qa_relative": 1e-5, "qa_absolute_floor": 1e-7,
    "qa_reference_magnitude_floor": 1e-9,
    "baked_cube_max_abs": 2e-6, "baked_lumix_max_abs": 2e-6,
    "ocio_processor_max_abs": 1e-6,
    "synthetic_cube_max_abs": 6e-10, "synthetic_lumix_max_abs": 5.01e-7,
    "synthetic_3dl_max_abs": .501 / 1023,
    "synthetic_hald_png_max_abs": .501 / 255,
}
FORMAT_PROBE = r'''
use std::path::Path;
use spektrafilm_core::lut_formats::{LutDocument,LutFormat,write_lut,read_lut};
fn main()->Result<(),Box<dyn std::error::Error>> {
 let arg=std::env::args().nth(1).ok_or("format output directory required")?;
 let root=Path::new(&arg);std::fs::create_dir_all(root)?;
 let mut table=Vec::new();
 for r in 0..4 {for g in 0..4 {for b in 0..4 {
  let r=r as f64/3.0;let g=g as f64/3.0;let b=b as f64/3.0;
  table.push([r*r+0.13*g-0.08,g*g+0.11*b+0.04,b*b+0.07*r+0.03]);
 }}}
 table[0]=[0.5/255.0,1.5/255.0,2.5/255.0];
 table[1]=[0.5/1023.0,1.5/1023.0,2.5/1023.0];
 let document=LutDocument{resolution:4,table,domain_min:[-0.25,0.1,0.0],domain_max:[1.25,1.1,2.0],title:"Axis-tagged nonlinear LUT".into()};
 for format in [LutFormat::Cube,LutFormat::Lumix,LutFormat::ThreeDl,LutFormat::HaldPng] {
  let path=root.join(format!("synthetic_{}{}",format.name(),format.extension()));
  write_lut(format,&document,&path,&["Independent format acceptance".into()],Some("VLOG"))?;
  let read=read_lut(format,&path)?;
  if read.resolution!=4 || read.table.len()!=64 {return Err("format reader dimensions drift".into());}
  let scale=match format {LutFormat::ThreeDl=>1023.0,LutFormat::HaldPng=>255.0,_=>0.0};
  for (source,actual) in document.table.iter().zip(&read.table) {
   for c in 0..3 {
    let expected=if scale>0.0 {(source[c]*scale).round_ties_even().clamp(0.0,scale)/scale} else {source[c]};
    let budget=if scale>0.0 {1e-12} else if matches!(format,LutFormat::Lumix) {5.01e-7} else {6e-10};
    if (expected-actual[c]).abs()>budget {return Err("Rust format reader axis/quantization drift".into());}
   }
  }
 }
 Ok(())
}
'''


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def command(argv, log, report):
    report["commands"].append([str(a) for a in argv])
    with log.open("w") as stream:
        result = subprocess.run([str(a) for a in argv], stdout=stream,
                                stderr=subprocess.STDOUT, env={**os.environ,
                                "SPEKTRAFILM_BACKEND": "cpu", "MPLBACKEND": "Agg"})
    require(result.returncode == 0, f"command exited {result.returncode}; see {log}")


def local_file(root, relative):
    path = PurePosixPath(relative)
    require(relative and "\\" not in relative and not path.is_absolute()
            and ".." not in path.parts, f"unsafe relative artifact {relative!r}")
    full = root.joinpath(*path.parts)
    require(full.resolve().is_relative_to(root.resolve()), f"artifact escapes root: {relative}")
    require(full.is_file(), f"missing artifact {full}")
    return full


def max_error(actual, expected, budget, label):
    import numpy as np
    require(actual.shape == expected.shape, f"{label}: shape {actual.shape} != {expected.shape}")
    require(np.isfinite(actual).all() and np.isfinite(expected).all(), f"{label}: nonfinite values")
    delta = float(np.max(np.abs(actual - expected)))
    require(delta <= budget, f"{label}: max_abs={delta} exceeds {budget}")
    return {"max_abs": delta, "budget": budget}


def check_artifacts(root, meta, qa_report=None, archive_required=False):
    refs = meta["artifacts"]
    paths = [a["path"] for a in refs]
    require(len(paths) == len(set(paths)), f"{root.name}: duplicate artifact registry paths")
    for relative in paths:
        local_file(root, relative)
    for lut in meta["luts"]:
        local_file(root, lut["path"])
    qa_paths = {a["path"] for a in refs if a["kind"] == "qa"}
    if qa_report is not None:
        require("qa/report.json" in qa_paths, "QA JSON not registered")
        on_disk = {p.relative_to(root).as_posix() for p in (root / "qa").rglob("*") if p.is_file()}
        require(on_disk == qa_paths, f"QA registry drift: missing={on_disk-qa_paths}, extra={qa_paths-on_disk}")
        for entry in qa_report["prints"]:
            folder = root / "qa" / entry["folder"]
            for doc in ("report.md", "report.html"):
                text = local_file(root, f"qa/{entry['folder']}/{doc}").read_text()
                require("Nonfinite metric" not in text, f"{doc}: hidden nonfinite failure")
                for attr, value in re.findall(r'''\b(src|href)\s*=\s*["']([^"']+)["']''', text):
                    url = urlsplit(value)
                    require(not url.scheme and not url.netloc, f"{doc}: external asset {value}")
                    if url.path:
                        asset = folder / unquote(url.path)
                        require(asset.resolve().is_relative_to(root.resolve()) and asset.is_file(),
                                f"{doc}: unresolved local asset {value}")
                require(not re.search(r"(?:url\(|@import).*https?://", text), f"{doc}: network CSS")
                if doc == "report.html":
                    for result in entry["results"]:
                        section = re.search(r"<h2>" + re.escape(result["name"]) + r"</h2>(.*?)(?=<h2>|</body>)", text, re.S)
                        require(section is not None, f"HTML missing scenario {result['name']}")
                        status = "INFO" if result["passed"] is None else "PASS" if result["passed"] else "FAIL"
                        require(f"Status: <b>{status}</b>" in section.group(1), f"HTML status drift for {result['name']}")
    archive = root.with_suffix(".zip")
    require(not archive_required or archive.is_file(), f"missing ZIP {archive}")
    files = {p.relative_to(root).as_posix(): p for p in root.rglob("*") if p.is_file()}
    if archive.is_file():
        with zipfile.ZipFile(archive) as z:
            names = [name for name in z.namelist() if not name.endswith("/")]
            require(len(names) == len(set(names)), "duplicate ZIP members")
            archived_files = {f"{root.name}/{name}": source for name, source in files.items()}
            require(set(names) == set(archived_files), f"ZIP contents differ: missing={set(archived_files)-set(names)}, extra={set(names)-set(archived_files)}")
            for name, source in archived_files.items():
                require(z.read(name) == source.read_bytes(), f"ZIP bytes differ for {name}")
    return {"artifacts": len(refs), "qa_artifacts": len(qa_paths), "files": len(files),
            "zip_verified": archive.is_file()}


def compare_qa(root, spec, delivered, report, expected_indices, reference_root):
    from spektrafilm_lut_creator.qa.suite import run as run_suite
    entries = report["prints"]
    require([e["print_index"] for e in entries] == expected_indices, "QA print selection drift")
    comparisons = []
    for entry in entries:
        index = entry["print_index"]
        results = run_suite(spec, delivered, reference_root / f"print{index:02d}", print_index=index)
        require(len(results) == 16, f"pinned suite changed: {len(results)} scenarios")
        require([r["name"] for r in entry["results"]] == [r.name for r in results], "QA scenario set/order drift")
        for actual, expected in zip(entry["results"], results):
            label = f"{root.name}/print{index}/{expected.name}"
            require(type(actual["passed"]) is type(expected.passed) and actual["passed"] == expected.passed,
                    f"{label}: status drift {actual['passed']!r} != {expected.passed!r}")
            require(set(actual["summary"]) == set(expected.summary), f"{label}: missing/extraneous metrics")
            metrics = {}
            for key, py in expected.summary.items():
                rs = actual["summary"][key]
                if isinstance(py, (int, float)) and not isinstance(py, bool):
                    require(isinstance(rs, (int, float)) and not isinstance(rs, bool), f"{label}/{key}: numeric type drift")
                    require(math.isfinite(py) and math.isfinite(rs), f"{label}/{key}: nonfinite metric")
                    budget = max(1e-5 * max(abs(py), 1e-9), 1e-7)
                    delta = abs(rs - float(py))
                    require(delta <= budget, f"{label}/{key}: {rs} != {py}, delta={delta}, budget={budget}")
                    metrics[key] = {"rust": rs, "python": float(py), "delta": delta, "budget": budget}
                else:
                    require(type(rs) is type(py) and rs == py, f"{label}/{key}: value drift {rs!r} != {py!r}")
                    metrics[key] = {"rust": rs, "python": py}
            comparisons.append({"print_index": index, "name": expected.name,
                                "rust_passed": actual["passed"], "python_passed": expected.passed,
                                "metrics": metrics})
    overall = all(r["rust_passed"] is not False for r in comparisons)
    require(report["passed"] is overall, "aggregate QA status hides scenario FAIL")
    return comparisons


def compare_ocio(root, spec, delivered, reference_path):
    import numpy as np
    import PyOpenColorIO as ocio
    from spektrafilm_lut_creator.ocio_emit import emit_ocio_config
    # Use delivered LUT paths and wire metadata, not independently baked files.
    reference_path.parent.mkdir(parents=True, exist_ok=True)
    reference_path.write_text(emit_ocio_config(delivered, spec))
    actual = ocio.Config.CreateFromFile(str(root / "config.ocio"))
    expected = ocio.Config.CreateFromFile(str(reference_path))
    expected.setWorkingDir(str(root))
    actual.validate()
    expected.validate()
    # Declaration order is not semantic — OCIO resolves color spaces by name and
    # the delivered emitter groups view spaces per print where the pinned Python
    # emitter groups them by kind. The processor matrices below are the strict
    # check. View order stays ordered: it is the display menu order.
    require(set(actual.getColorSpaceNames()) == set(expected.getColorSpaceNames()), "OCIO color space set drift")
    require(list(actual.getRoles()) == list(expected.getRoles()), "OCIO role drift")
    require(list(actual.getDisplays()) == list(expected.getDisplays()), "OCIO display drift")
    probes = np.concatenate([np.random.default_rng(34).uniform(.02, .98, (128, 3)),
                             [[0, 0, 0], [1, 1, 1], [.2, .5, .8], [.5, .5, .5],
                              [1, 0, 0], [0, 1, 0], [0, 0, 1]]])
    processors = []
    for name in expected.getColorSpaceNames():
        ac = actual.getProcessor("ACES2065-1", name).getDefaultCPUProcessor()
        py = expected.getProcessor("ACES2065-1", name).getDefaultCPUProcessor()
        av = np.array([ac.applyRGB(p.tolist()) for p in probes])
        pv = np.array([py.applyRGB(p.tolist()) for p in probes])
        processors.append({"colorspace": name, **max_error(av, pv, 1e-6, f"OCIO/{name}")})
    views = []
    for display in expected.getDisplays():
        require(list(actual.getViews(display)) == list(expected.getViews(display)), "OCIO view matrix drift")
        for view in expected.getViews(display):
            ac = actual.getProcessor(spec.input_color_space, display, view, ocio.TRANSFORM_DIR_FORWARD).getDefaultCPUProcessor()
            py = expected.getProcessor(spec.input_color_space, display, view, ocio.TRANSFORM_DIR_FORWARD).getDefaultCPUProcessor()
            av = np.array([ac.applyRGB(p.tolist()) for p in probes])
            pv = np.array([py.applyRGB(p.tolist()) for p in probes])
            views.append({"display": display, "view": view, **max_error(av, pv, 1e-6, f"OCIO/{display}/{view}")})
    return {"runtime": ocio.GetVersion(), "samples_per_processor": len(probes),
            "processors": processors, "views": views}


def format_acceptance(cli, root, report):
    import numpy as np
    from spektrafilm_lut_creator.formats import Lut, get_format
    deps = cli.parent / "deps"
    libraries = sorted(deps.glob("libspektrafilm_core-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
    require(bool(libraries), f"format API acceptance requires existing release/deps core rlib in {deps}")
    source = root / "format_probe.rs"
    binary = root / "format_probe"
    source.write_text(FORMAT_PROBE)
    argv = ["rustc", "--edition=2024", source, "--extern", f"spektrafilm_core={libraries[0]}",
            "-L", f"dependency={deps}", "-o", binary]
    for native in os.environ.get("LD_LIBRARY_PATH", "").split(os.pathsep):
        if native:
            argv.extend(["-L", f"native={native}"])
    command(argv, root / "format_compile.log", report)
    output = root / "formats"
    command([binary, output], root / "format_probe.log", report)
    table = np.empty((4, 4, 4, 3))
    for b in range(4):
        for g in range(4):
            for r in range(4):
                rr, gg, bb = r / 3, g / 3, b / 3
                table[b, g, r] = [rr * rr + .13 * gg - .08, gg * gg + .11 * bb + .04, bb * bb + .07 * rr + .03]
    table[0, 0, 0] = [.5 / 255, 1.5 / 255, 2.5 / 255]
    table[1, 0, 0] = [.5 / 1023, 1.5 / 1023, 2.5 / 1023]
    doc = Lut(table=table, domain_min=(-.25, .1, 0), domain_max=(1.25, 1.1, 2), title="Axis-tagged nonlinear LUT")
    results = []
    for fmt, ext in (("cube", ".cube"), ("lumix", ".cube"), ("3dl", ".3dl"), ("hald_png", ".png")):
        path = output / f"synthetic_{fmt}{ext}"
        reader = get_format(fmt)
        actual = reader.read(path)
        quantized = fmt in ("3dl", "hald_png")
        expected = np.clip(table, 0, 1) if quantized else table
        budget = THRESHOLDS[f"synthetic_{fmt}_max_abs"]
        metrics = max_error(actual.table, expected, budget, fmt)
        if quantized:
            scale = 1023 if fmt == "3dl" else 255
            require(np.array_equal(np.rint(actual.table * scale), np.clip(np.rint(table * scale), 0, scale)), f"{fmt}: quantization or axis drift")
        else:
            require(np.allclose(actual.domain_min, doc.domain_min, rtol=0, atol=5e-7)
                    and np.allclose(actual.domain_max, doc.domain_max, rtol=0, atol=5e-7), f"{fmt}: domain drift")
        reference = output / f"python_{fmt}{ext}"
        reader.write(doc, reference, **({"photo_style_tag": "VLOG"} if fmt == "lumix" else {}))
        writer = max_error(actual.table, reader.read(reference).table, budget, f"{fmt}/Python writer")
        if fmt == "lumix":
            lines = path.read_text().splitlines()
            require(lines[:6] == ['TITLE "Axis-tagged nonlinear LUT"', '#LUMIXPHOTOSTYLE VLOG', 'LUT_3D_SIZE 4', 'DOMAIN_MIN -0.250000 0.100000 0.000000', 'DOMAIN_MAX 1.250000 1.100000 2.000000', ''], "Lumix strict header drift")
            require(not any(line.startswith("#") for line in lines[2:]), "Lumix extra comments")
        results.append({"format": fmt, **metrics, "vs_python_writer": writer})
    return results


def run(args, report):
    import numpy as np
    import spektrafilm
    from spektrafilm_lut_creator.bundles import Bundle, BundleSpec
    from spektrafilm_lut_creator.builders import BundleBuilder
    from spektrafilm_lut_creator.formats import get_format
    from spektrafilm_lut_creator.metadata import LutFileMeta
    reference_repo = Path(os.environ["SPEKTRAFILM_PY_REPO"]).resolve()
    commit = subprocess.check_output(["git", "-C", str(reference_repo), "rev-parse", "HEAD"], text=True).strip()
    require(commit == PIN, f"reference checkout is {commit}, expected {PIN}")
    require(Path(spektrafilm.__file__).resolve().is_relative_to(reference_repo), "Python import is not the pinned editable checkout")
    report["reference"] = {"commit": commit, "repo": str(reference_repo),
                            "module": spektrafilm.__file__, "python": sys.executable,
                            "dependencies": {d: importlib.metadata.version(d) for d in
                                ("numpy", "scipy", "colour-science", "matplotlib", "opencolorio")}}
    cli = args.cli.resolve()
    data = args.data_dir.resolve()
    root = args.evidence_dir.resolve()
    # Refuse stale generated trees instead of deleting user files or mixing runs.
    bundles = root / "bundles"
    require(not bundles.exists(), f"{bundles} already exists; use a fresh evidence directory")
    bundles.mkdir()
    report["cli"] = {"path": str(cli), "sha256": hashlib.sha256(cli.read_bytes()).hexdigest(), "backend": "cpu"}
    report["cases"] = []
    report["scenario_count"] = 0
    for topology in ("1lut", "2lut", "3lut", "4lut"):
        name = f"qa_{topology}"
        argv = [cli, "lut", "build", "--name", name, "--film", FILM,
                "--print", PRINTS[0], "--print", PRINTS[1], "--input", "sRGB", "--output", "sRGB",
                "--topology", topology, "--resolution", "17", "--qa",
                "--out", bundles, "--data-dir", data]
        if topology == "2lut":
            argv.extend(["--qa-print-index", "1"])
        if topology == "4lut":
            argv.extend(["--combinations", "--container", "zip"])
        command(argv, root / f"{name}.log", report)
        folder = bundles / name
        meta = json.loads((folder / "bundle.json").read_text())
        spec = BundleSpec(film_profile=FILM, print_profiles=PRINTS, input_color_space="sRGB",
                          output_color_space="sRGB", topology=topology, resolution=17, name=name,
                          include_combinations=topology == "4lut",
                          exposure_ev=(meta["input_exposure"] or {}).get("exposure_ev", 0.0))
        expected = BundleBuilder(spec).build()
        require(meta["topology"] == topology and meta["resolution"] == 17, "bundle topology/resolution drift")
        require(meta["stocks"] == {"film": FILM, "prints": list(PRINTS)}, "bundle stocks drift")
        require(meta["color_spaces"] == dataclasses.asdict(expected.meta)["color_spaces"], "bundle color spaces drift")
        require(meta["provenance"]["reference_commit"] == PIN, "bundle reference provenance drift")
        fields = ("role", "path", "domain", "range", "print_profile")
        require([tuple(m[k] for k in fields) for m in meta["luts"]]
                == [tuple(getattr(m, k) for k in fields) for m in expected.meta.luts], "LUT roles/paths/order drift")
        for wire, value in meta["wires"].items():
            py = getattr(expected.meta.wires, wire)
            require((value is None) == (py is None), f"{wire}: wire presence drift")
            if py is not None:
                for key, number in dataclasses.asdict(py).items():
                    max_error(np.asarray(value[key]), np.asarray(number), 2e-6, f"wire/{wire}/{key}")
        luts = [(m["path"], get_format("cube").read(folder / m["path"])) for m in meta["luts"]]
        comparisons = []
        for (path, actual), (py_path, py) in zip(luts, expected.luts):
            require(path == py_path, "baked LUT path drift")
            comparisons.append({"path": path, **max_error(actual.table, py.table, 2e-6, path)})
        # Reconstruct the upstream consumer with actual delivered metadata and cubes.
        wires = dataclasses.replace(expected.meta.wires, **{
            key: None if value is None else type(getattr(expected.meta.wires, key))(**value)
            for key, value in meta["wires"].items()})
        delivered = Bundle(luts=luts, meta=dataclasses.replace(expected.meta, wires=wires,
                           luts=tuple(LutFileMeta(**{k: m[k] for k in fields}) for m in meta["luts"])))
        qa = json.loads((folder / "qa" / "report.json").read_text())
        indices = [1] if topology == "2lut" else [0, 1]
        case = {"name": name, "topology": topology, "lattices": comparisons}
        report["cases"].append(case)
        case["qa"] = compare_qa(folder, spec, delivered, qa, indices, root / "python-qa" / name)
        report["scenario_count"] += len(case["qa"])
        case["upstream_qa_passed"] = qa["passed"]
        case["artifacts"] = check_artifacts(folder, meta, qa, topology == "4lut")
        ocio_name = f"ocio_{topology}"
        command([cli, "lut", "build", "--name", ocio_name, "--film", FILM,
                 "--print", PRINTS[0], "--print", PRINTS[1], "--input", "Panasonic V-Log",
                 "--output", "sRGB", "--topology", topology, "--resolution", "17",
                 "--ocio-config", "--combinations", "--out", bundles, "--data-dir", data],
                root / f"{ocio_name}.log", report)
        ocio_root = bundles / ocio_name
        ocio_meta = json.loads((ocio_root / "bundle.json").read_text())
        ocio_spec = BundleSpec(film_profile=FILM, print_profiles=PRINTS,
                              input_color_space="Panasonic V-Log", output_color_space="sRGB",
                              topology=topology, resolution=17, name=ocio_name,
                              include_combinations=True,
                              exposure_ev=(ocio_meta["input_exposure"] or {}).get("exposure_ev", 0.0))
        ocio_python = BundleBuilder(ocio_spec).build()
        require([tuple(m[k] for k in fields) for m in ocio_meta["luts"]]
                == [tuple(getattr(m, k) for k in fields) for m in ocio_python.meta.luts],
                "OCIO LUT roles/paths/order drift")
        ocio_luts = [(m["path"], get_format("cube").read(ocio_root / m["path"])) for m in ocio_meta["luts"]]
        ocio_lattices = [{"path": path, **max_error(actual.table, py.table, 2e-6, f"OCIO bake/{path}")}
                         for (path, actual), (_, py) in zip(ocio_luts, ocio_python.luts)]
        ocio_wires = dataclasses.replace(ocio_python.meta.wires, **{
            key: None if value is None else type(getattr(ocio_python.meta.wires, key))(**value)
            for key, value in ocio_meta["wires"].items()})
        ocio_delivered = Bundle(luts=ocio_luts, meta=dataclasses.replace(ocio_python.meta,
                                wires=ocio_wires, luts=tuple(LutFileMeta(**{k: m[k] for k in fields})
                                for m in ocio_meta["luts"])))
        case["ocio_artifacts"] = check_artifacts(ocio_root, ocio_meta)
        case["ocio_lattices"] = ocio_lattices
        case["ocio"] = compare_ocio(ocio_root, ocio_spec, ocio_delivered, root / "python-ocio" / f"{ocio_name}.ocio")
    require(report["scenario_count"] == 112, f"expected 112 QA scenarios, got {report['scenario_count']}")
    from spektrafilm.utils.gamut_compression import InputGamutCompressSpec
    report["input_gamut_overrides"] = []
    report["override_scenario_count"] = 0
    for algorithm, active in (("xy", False), ("oklch", True)):
        name = f"qa_input_{algorithm}_{'active' if active else 'disabled'}"
        spec_path = root / f"{name}.toml"
        spec_path.write_text(
            f'name = "{name}"\nfilm_profile = "{FILM}"\n'
            f'print_profiles = ["{PRINTS[0]}"]\n'
            'input_color_space = "sRGB"\noutput_color_space = "sRGB"\n'
            'topology = "1lut"\nresolution = 17\n'
            '[input_gamut_compress]\n'
            f'active = {str(active).lower()}\nalgorithm = "{algorithm}"\n'
            'knee = [0.0, 1.0, 6.0]\n')
        command([cli, "lut", "build", "--from", spec_path, "--qa",
                 "--out", bundles, "--data-dir", data], root / f"{name}.log", report)
        folder = bundles / name
        meta = json.loads((folder / "bundle.json").read_text())
        spec = BundleSpec(film_profile=FILM, print_profiles=PRINTS[:1],
                          input_color_space="sRGB", output_color_space="sRGB",
                          topology="1lut", resolution=17, name=name,
                          input_gamut_compress=InputGamutCompressSpec(active=active, algorithm=algorithm),
                          exposure_ev=(meta["input_exposure"] or {}).get("exposure_ev", 0.0))
        expected = BundleBuilder(spec).build()
        luts = [(m["path"], get_format("cube").read(folder / m["path"])) for m in meta["luts"]]
        require([m["path"] for m in meta["luts"]] == [p for p, _ in expected.luts],
                f"{name}: LUT paths drift")
        lattices = [{"path": path, **max_error(actual.table, py.table, 2e-6, f"{name}/{path}")}
                    for (path, actual), (_, py) in zip(luts, expected.luts)]
        wires = dataclasses.replace(expected.meta.wires, **{
            key: None if value is None else type(getattr(expected.meta.wires, key))(**value)
            for key, value in meta["wires"].items()})
        delivered = Bundle(luts=luts, meta=dataclasses.replace(expected.meta, wires=wires,
                           luts=tuple(LutFileMeta(**{k: m[k] for k in fields}) for m in meta["luts"])))
        qa = json.loads((folder / "qa" / "report.json").read_text())
        comparisons = compare_qa(folder, spec, delivered, qa, [0], root / "python-qa" / name)
        diagnostics = [r for r in qa["prints"][0]["results"]
                       if r["name"] in ("input_gamut_compression_preview", "input_gamut_compression_smoothness")]
        require(len(diagnostics) == 2, f"{name}: missing input-compression diagnostics")
        for diagnostic in diagnostics:
            require(diagnostic["summary"]["active"] is active
                    and diagnostic["summary"]["algorithm"] == algorithm,
                    f"{name}: TOML compression override ignored")
        if not active:
            smooth = diagnostics[1]["summary"]
            expected_step = 0.6 * math.sin(math.pi / 720)
            require(abs(smooth["worst_step"] - expected_step) < 1e-12
                    and abs(smooth["median_step"] - expected_step) < 1e-12
                    and abs(smooth["worst_over_median_step"] - 1.0) < 1e-12,
                    f"{name}: disabled compression changed the smoothness ring")
        report["input_gamut_overrides"].append({"name": name, "algorithm": algorithm, "active": active,
                                              "lattices": lattices, "qa": comparisons,
                                              "artifacts": check_artifacts(folder, meta, qa)})
        report["override_scenario_count"] += len(comparisons)
    require(report["override_scenario_count"] == 32, "expected 32 additional override QA scenarios")
    name = "lumix"
    command([cli, "lut", "build", "--name", name, "--film", FILM, "--print", PRINTS[0],
             "--input", "Panasonic V-Log", "--output", "sRGB", "--resolution", "4",
             "--target", "lumix_realtime_vlog", "--out", bundles, "--data-dir", data], root / "lumix.log", report)
    folder = bundles / name
    meta = json.loads((folder / "bundle.json").read_text())
    expected = BundleBuilder(BundleSpec(film_profile=FILM, print_profiles=PRINTS[:1],
                            input_color_space="Panasonic V-Log", output_color_space="sRGB",
                            resolution=4, target="lumix_realtime_vlog", name=name)).build()
    require(meta["target"] == "lumix_realtime_vlog", "Lumix target drift")
    require([m["path"] for m in meta["luts"]] == [p for p, _ in expected.luts], "Lumix path drift")
    lattices = []
    for row, (_, py) in zip(meta["luts"], expected.luts):
        path = folder / row["path"]
        actual = get_format("lumix").read(path)
        lattices.append({"path": row["path"], **max_error(actual.table, py.table, 2e-6, "Lumix bake")})
        lines = path.read_text().splitlines()
        require(lines[0].startswith("TITLE ") and lines[1] == "#LUMIXPHOTOSTYLE VLOG"
                and lines[2] == "LUT_3D_SIZE 4" and lines[3].startswith("DOMAIN_MIN ")
                and lines[4].startswith("DOMAIN_MAX ") and lines[5] == ""
                and not any(line.startswith("#") for line in lines[2:]), "Lumix camera header drift")
    report["lumix"] = {"lattices": lattices, "artifacts": check_artifacts(folder, meta)}
    report["formats"] = format_acceptance(cli, root, report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, required=True)
    parser.add_argument("--evidence-dir", type=Path, required=True)
    args = parser.parse_args()
    interpreter = os.environ.get("SPEKTRAFILM_PY")
    if interpreter and Path(interpreter).absolute() != Path(sys.executable).absolute():
        return subprocess.call([interpreter, str(Path(__file__).resolve()), *sys.argv[1:]], env=os.environ)
    args.evidence_dir.mkdir(parents=True, exist_ok=True)
    report = {"reference_commit": PIN, "thresholds": THRESHOLDS,
              "commands": [], "passed": False, "failures": []}
    try:
        run(args, report)
        report["passed"] = True
    except Exception as exc:
        report["failures"].append({"type": type(exc).__name__, "message": str(exc), "traceback": traceback.format_exc()})
    (args.evidence_dir / "report.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    print(json.dumps({"passed": report["passed"], "scenario_count": report.get("scenario_count", 0),
                      "report": str(args.evidence_dir / "report.json"), "failures": report["failures"]}, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
