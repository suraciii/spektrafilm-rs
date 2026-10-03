#!/usr/bin/env python3
"""Compare the native lens bridge with installed, pinned upstream Spektrafilm.

Run using the upstream Python environment, for example:
  /tmp/spektrafilm-034-venv/bin/python scripts/parity/lens_reference_compare.py

Requires a C++17 compiler, pkg-config, native Lensfun/Exiv2/GLib development
packages and the upstream Python dependencies. CXX selects the compiler.
All generated images and compiled libraries live in a temporary directory.
"""
import argparse
import ctypes
import importlib.metadata
import inspect
import json
import os
import pathlib
import shlex
import shutil
import subprocess
import tempfile

import exiv2
import lensfunpy
import numpy as np
import scipy
from spektrafilm.utils.raw_file_processor import (
    _apply_lens_correction,
    _read_exif_metadata,
)


def pkg_config(*arguments):
    return subprocess.check_output(["pkg-config", *arguments], text=True).strip()


def write_exif(source, destination, values):
    shutil.copyfile(source, destination)
    image = exiv2.ImageFactory.open(str(destination))
    image.readMetadata()
    metadata = exiv2.ExifData()
    for key, value in values.items():
        metadata[key] = value
    image.setExifData(metadata)
    image.writeMetadata()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=pathlib.Path,
                        help="JPEG used as the metadata carrier; defaults to docs/card.jpg")
    parser.add_argument("--max-error", type=float, default=0.0)
    parser.add_argument("--mean-error", type=float, default=0.0)
    args = parser.parse_args()
    if args.max_error < 0 or args.mean_error < 0:
        parser.error("Error budgets must be nonnegative")
    root = pathlib.Path(__file__).resolve().parents[2]
    fixture = args.fixture or root / "docs/card.jpg"
    settings = {"pixel_format": "float32", "flags": "ALL", "distance": 1000.0,
                "scale": 0.0, "geometry": "RECTILINEAR", "reverse": False,
                "interpolation_order": 1, "edge_mode": "nearest"}
    defaults = inspect.signature(lensfunpy.Modifier.initialize).parameters
    if defaults["distance"].default != settings["distance"] or defaults["scale"].default != settings["scale"]:
        raise RuntimeError("Reference modifier defaults differ from pinned lensfunpy 1.18.0")
    known = {"Exif.Image.Make": "Canon", "Exif.Image.Model": "Canon EOS 5D Mark II",
             "Exif.Photo.LensMake": "Canon", "Exif.Photo.LensModel": "Canon EF 24-70mm f/2.8L USM",
             "Exif.Photo.FocalLength": "35/1", "Exif.Photo.FNumber": "8/1"}
    cases = [("known_lens", known), ("absent_metadata", {}),
             ("absent_camera", dict(known, **{"Exif.Image.Make": "zzzzzzqqqqqq",
                                             "Exif.Image.Model": "zzzzzzqqqqqq"})),
             ("absent_lens", dict(known, **{"Exif.Photo.LensMake": "",
                                           "Exif.Photo.LensModel": "zzzzzzqqqqqq"}))]
    packages = ["lensfun", "exiv2", "glib-2.0"]
    compiler = shlex.split(os.environ.get("CXX", "c++"))
    rows = []
    with tempfile.TemporaryDirectory(prefix="spektrafilm-lens-parity-") as directory:
        temporary = pathlib.Path(directory)
        library = temporary / "lens.so"
        command = compiler + ["-std=c++17", "-O2", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"]
        command += shlex.split(pkg_config("--cflags", *packages))
        command += [str(root / "crates/spektrafilm-raw/native/lens.cpp"), "-o", str(library)]
        command += shlex.split(pkg_config("--libs", *packages))
        subprocess.run(command, check=True)
        bridge = ctypes.CDLL(str(library))
        correct = bridge.sf_raw_correct_lens
        correct.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_float), ctypes.c_uint,
                            ctypes.c_uint, ctypes.c_char_p, ctypes.c_size_t,
                            ctypes.c_char_p, ctypes.c_size_t]
        correct.restype = ctypes.c_int
        for name, tags in cases:
            path = temporary / (name + ".jpg")
            write_exif(fixture, path, tags)
            for height, width in [(48, 64), (480, 640)]:
                y, x, channel = np.indices((height, width, 3))
                original = (0.1 + ((x * 3 + y * 7 + channel * 11) % 31) / 100).astype(np.float32)
                actual = original.copy()
                summary, error = ctypes.create_string_buffer(512), ctypes.create_string_buffer(1024)
                status = correct(str(path).encode(), actual.ctypes.data_as(ctypes.POINTER(ctypes.c_float)),
                                 width, height, summary, len(summary), error, len(error))
                reference, label = _apply_lens_correction(original.copy(), _read_exif_metadata(path))
                difference = np.abs(actual.astype(np.float64) - reference.astype(np.float64))
                maximum, mean = float(difference.max()), float(difference.mean())
                native_label = summary.value.decode()
                unchanged = bool(np.array_equal(actual, original))
                correction_present = bool(label) and not unchanged
                expected_behavior = correction_present if name == "known_lens" else unchanged and not label
                rows.append({"case": name, "shape": list(original.shape), "exif": tags,
                             "status": status, "error": error.value.decode(),
                             "summary": native_label, "reference_summary": label,
                             "unchanged": unchanged, "max_abs": maximum, "mean_abs": mean,
                             "pass": status == 0 and not error.value and native_label == label
                                     and expected_behavior and maximum <= args.max_error
                                     and mean <= args.mean_error})
    versions = {package: pkg_config("--modversion", package) for package in packages}
    versions.update({"spektrafilm": importlib.metadata.version("spektrafilm"),
                     "lensfunpy": importlib.metadata.version("lensfunpy"),
                     "lensfunpy_lensfun": list(lensfunpy.lensfun_version),
                     "python_exiv2": importlib.metadata.version("exiv2"),
                     "numpy": np.__version__, "scipy": scipy.__version__})
    report = {"reference_commit": "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc",
              "reference_module": inspect.getfile(_apply_lens_correction),
              "versions": versions, "settings": settings,
              "compiler": subprocess.check_output(compiler + ["--version"], text=True).splitlines()[0],
              "budget": {"max_abs": args.max_error, "mean_abs": args.mean_error}, "results": rows}
    print(json.dumps(report, indent=2))
    return 0 if all(row["pass"] for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
