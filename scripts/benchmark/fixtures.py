#!/usr/bin/env python3
"""Generate deterministic benchmark probes and concrete renderer targets."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import urllib.request
from importlib.resources import files

import numpy as np
from PIL import Image
from documents import canonical_json, protocol_digest, sha256_file, verify_sha256, load_suite, load_target, load_policy
from image_data import srgb_decode


def write(path, value):
    path.write_bytes(canonical_json(value) + b"\n")


def region(identity, rectangle, families, role="color"):
    return {"id": identity, "label": identity.replace("_", " ").title(), "role": role,
            "geometry": rectangle, "measurements": families}


def configuration(grain=False, spatial=False):
    edits = ["camera.auto_exposure=false", "io.output_color_space=sRGB", "io.output_cctf_encoding=true",
             "debug.deactivate_spatial_effects=" + str(not spatial).lower(),
             "debug.deactivate_stochastic_effects=" + str(not grain).lower(),
             "film_render.grain.active=" + str(grain).lower()]
    if grain:
        edits += ["film_render.grain.engine=v2", "film_render.grain.v2_profile=custom", "film_render.grain.v2_mode=noise", "film_render.grain.v2_amount=25"]
    return {"film": "kodak_gold_200", "paper": "kodak_portra_endura", "route": "input > film > print > scan",
            "presets": [], "set_files": [], "set": edits}


def case(identity, path, families, regions, *, grain=False, spatial=False, probe=None):
    value = {"id": identity, "kind": "probe", "input": {"kind": "raster", "path": path.name, "sha256": sha256_file(path), "color_space": "sRGB", "cctf_decoding": True},
             "configuration": configuration(grain, spatial), "seeds": [0, 42, 99] if grain else [0],
             "regions": regions, "measurements": families}
    if probe:
        value["probe"] = probe
    return value


def create_fixtures(renderer, data_dir, out, natural_input=None, download_natural=False):
    if out.exists():
        raise ValueError(f"fixture destination already exists: {out}")
    out.mkdir(parents=True)
    width = height = 128
    colors = np.array([[.45, .45, .45], [.65, .42, .32], [.2, .55, .3], [.1, .3, .65], [.7, .15, .12], [.15, .65, .65], [.65, .15, .65], [.7, .65, .1]])
    chart = np.zeros((height, width, 3))
    for index, color in enumerate(colors):
        chart[32:, index*16:(index+1)*16] = color
    chart[:32] = np.linspace(0, 1, width)[None, :, None]
    image = out / "chart.png"
    Image.fromarray(np.rint(chart*255).astype(np.uint8)).save(image)
    chart_case = case("color_chart", image, ["numeric", "color", "tone", "flip"],
                      [region("skin", [16, 32, 16, 64], ["color", "tone"], "skin"), region("gray", [0, 32, 16, 64], ["color", "tone"], "neutral")])
    stops = np.arange(-5, 3, dtype=float)
    ramp = np.zeros((64, width, 3))
    response = []
    for index, stop in enumerate(stops):
        linear = min(1., .18*2**stop)
        encoded = 12.92*linear if linear <= .0031308 else 1.055*linear**(1/2.4)-.055
        ramp[:, index*16:(index+1)*16] = encoded
        response.append({"exposure_stops": float(stop), "geometry": [index*16+3, 3, 10, 58]})
    image = out / "exposure.png"
    Image.fromarray(np.rint(ramp*255).astype(np.uint8)).save(image)
    tone_case = case("exposure_response", image, ["numeric", "color", "tone", "flip"], [], probe={"tone_response": response})
    image = out / "flat_fields.png"
    flat = np.zeros((height, width, 3))
    for index, level in enumerate((.1, .3, .5, .8)):
        flat[:, index*32:(index+1)*32] = level
    Image.fromarray(np.rint(flat*255).astype(np.uint8)).save(image)
    grain_case = case("grain_fields", image, ["numeric", "color", "flip"],
                      [region(f"level_{index}", [index*32+4, 4, 24, 120], ["grain", "color"], "flat_field") for index in range(4)], grain=True)
    image = out / "bright_point.png"
    y, x = np.indices((height, width))
    point = np.full((height, width, 3), .04)
    point[(x-63.5)**2+(y-63.5)**2 <= 3**2] = 1.
    Image.fromarray(np.rint(point*255).astype(np.uint8)).save(image)
    point_case = case("halation_point", image, ["numeric", "color", "flip", "halation"], [], spatial=True,
                      probe={"halation": {"center_px": [63.5, 63.5], "core_radius_px": 3., "background_inner_radius_px": 45.,
                                          "background_outer_radius_px": 58., "extent_px": 40., "bin_width_px": 1., "domain": "linear_srgb"}})
    image = out / "edge.png"
    edge = np.full((height, width, 3), .1)
    edge[:, width//2:] = .8
    quantized_edge = np.rint(edge*255).astype(np.uint8)
    Image.fromarray(quantized_edge).save(image)
    input_contrast = float(srgb_decode(quantized_edge[0, -1, 0]/255)-srgb_decode(quantized_edge[0, 0, 0]/255))
    edge_case = case("edge_response", image, ["numeric", "color", "flip", "edge"], [], spatial=True,
                     probe={"edge": {"axis": "x", "plateau_fraction": .2, "domain": "linear_srgb", "input_contrast": input_contrast, "input_exposure_stops": 0.}})
    protocol = {"id": "appearance-v1", "pixels_per_degree": 60., "region_sampling_scales": [1., 4., 16.],
                "grain": {"residual": {"method": "flat_mean", "high_pass_radius_px": 4., "border_px": 1}, "spectrum": {"window": "hann"}},
                "hotspots": {"count": 6, "min_area_px": 1024}}
    quick = {"schema_version": 1, "id": "quick-probes", "protocol": protocol, "cases": [chart_case, tone_case]}
    complete = {"schema_version": 1, "id": "complete-probes", "protocol": protocol, "cases": [chart_case, tone_case, grain_case, point_case, edge_case]}
    write(out / "quick.json", quick)
    write(out / "complete.json", complete)
    targets = {}
    for name, edits in (("baseline", []), ("candidate", ["camera.exposure_compensation_ev=0.25"]), ("grain-candidate", ["film_render.grain.v2_amount=35"])):
        target = {"schema_version": 1, "id": name, "renderer": {"path": str(renderer.resolve()), "sha256": sha256_file(renderer)},
                  "data_dir": str(data_dir.resolve()), "backend": "cpu", "changes": {"set_files": [], "set": edits},
                  "environment": {"RAYON_NUM_THREADS": "8", "OPENBLAS_NUM_THREADS": "1", "OMP_NUM_THREADS": "1"}}
        write(out / f"{name}.json", target)
        targets[name] = target
    policy = {"schema_version": 1, "id": "exploratory-color", "protocol_digest": protocol_digest(complete),
              "calibration": {"status": "exploratory", "evidence": []},
              "rules": [{"id": "skin-color", "purpose": "protect", "selector": {"case_id": "color_chart", "region_id": "skin"},
                         "metric": "delta_e00.p95", "max": 2., "required": True}]}
    provenance = {"schema_version": 1, "controlled_images": "Deterministic RGB probes generated by this package; not measured spectral or physical film references.", "natural_scene": None}
    source = json.loads(files("spektrafilm_benchmark_assets").joinpath("natural-source.json").read_text())
    if natural_input or download_natural:
        destination = out / "natural.KDC"
        if natural_input:
            destination.write_bytes(natural_input.read_bytes())
        else:
            with urllib.request.urlopen(source["source_url"], timeout=120) as response_stream:
                destination.write_bytes(response_stream.read())
        verify_sha256(destination, source["sha256"])
        natural = {"id": "kodak_dc50_scene", "kind": "scene", "input": {"kind": "raw", "adapter": "spektrafilm-raw", "path": destination.name, "sha256": source["sha256"]},
                   "configuration": {**configuration(), "raw": {"white_balance": "as-shot", "lens_correction": False}}, "seeds": [0],
                   "regions": [region("shadow", [64, 64, 160, 128], ["color", "tone"], "shadow"), region("texture", [320, 192, 160, 128], ["color", "tone"], "texture")],
                   "measurements": ["numeric", "color", "tone", "flip"]}
        write(out / "natural.json", {"schema_version": 1, "id": "natural-scene", "protocol": protocol, "cases": [natural]})
        provenance["natural_scene"] = source
        quick["id"] = "quick-probes-and-scene"
        complete["id"] = "complete-probes-and-scene"
        quick["cases"].append(natural)
        complete["cases"].append(natural)
        write(out / "quick.json", quick)
        write(out / "complete.json", complete)
    policy["protocol_digest"] = protocol_digest(complete)
    write(out / "exploratory-policy.json", policy)
    write(out / "provenance.json", provenance)
    load_suite(out / "quick.json")
    load_suite(out / "complete.json")
    if (out / "natural.json").exists():
        load_suite(out / "natural.json")
    for name in targets:
        load_target(out / f"{name}.json")
    load_policy(out / "exploratory-policy.json")
    return {"schema_version": 1, "execution_status": "completed", "artifact": str(out), "suites": [path.name for path in out.glob("*.json") if path.name in ("quick.json", "complete.json", "natural.json")]}


def main():
    parser = argparse.ArgumentParser(description="Generate ready-to-run controlled benchmark suites and renderer targets.")
    parser.add_argument("--renderer", type=Path, required=True, help="Real spektrafilm or spektrafilm-f64 executable")
    parser.add_argument("--data-dir", type=Path, required=True, help="Renderer data directory")
    parser.add_argument("--out", type=Path, required=True, help="New fixture directory outside repository")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--natural-input", type=Path, help="Pinned Kodak DC50 KDC original already acquired")
    group.add_argument("--download-natural", action="store_true", help="Fetch pinned RAW into fixture directory; source license in provenance.json")
    args = parser.parse_args()
    print(json.dumps(create_fixtures(args.renderer, args.data_dir, args.out, args.natural_input, args.download_natural), indent=2))


if __name__ == "__main__":
    main()
