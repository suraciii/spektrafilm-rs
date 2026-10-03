#!/usr/bin/env python3
"""Fresh reference RAW comparison; errors are failures, never skipped evidence."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile

import numpy as np
import OpenImageIO as oiio
import rawpy
from spektrafilm.utils.raw_file_processor import load_and_process_raw_file


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=pathlib.Path)
    parser.add_argument("--decoder", type=pathlib.Path, required=True)
    parser.add_argument("--source-url", required=True)
    parser.add_argument("--max-error", type=float, required=True,
                        help="Explicit decoder budget; report retains all measured errors")
    parser.add_argument("--mean-error", type=float, required=True)
    parser.add_argument("--lens-correction", action="store_true")
    args = parser.parse_args()
    rows = []
    with rawpy.imread(str(args.fixture)) as raw:
        camera = {"raw_type": str(raw.raw_type), "white_balance": list(raw.camera_whitebalance),
                  "raw_size": [raw.sizes.raw_width, raw.sizes.raw_height]}
    with tempfile.TemporaryDirectory(prefix="spektrafilm-raw-parity-") as directory:
        for mode, temperature, tint in [("as_shot", None, None), ("daylight", None, None),
                                        ("tungsten", None, None), ("custom", 5000.0, 1.05)]:
            output = pathlib.Path(directory) / (mode + ".tif")
            command = [str(args.decoder.resolve()), str(args.fixture.resolve()), str(output),
                       "--raw-white-balance", mode.replace("_", "-")]
            if temperature is not None:
                command += ["--raw-temperature", str(temperature), "--raw-tint", str(tint)]
            if args.lens_correction:
                command += ["--lens-correction"]
            result = subprocess.run(command, check=True, capture_output=True, text=True)
            reference = load_and_process_raw_file(args.fixture, white_balance=mode,
                                                  temperature=temperature, tint=tint,
                                                  lens_correction=args.lens_correction)
            handle = oiio.ImageInput.open(str(output))
            if handle is None:
                raise RuntimeError(oiio.geterror())
            decoded = handle.read_image(format=oiio.FLOAT)
            handle.close()
            if decoded.shape != reference.shape:
                raise RuntimeError(f"Dimensions differ: {decoded.shape} vs {reference.shape}")
            difference = np.abs(decoded.astype(np.float64) - reference.astype(np.float64))
            maximum, mean = float(difference.max()), float(difference.mean())
            rows.append({"mode": mode, "temperature": temperature, "tint": tint,
                         "lens_correction": args.lens_correction, "shape": list(decoded.shape),
                         "max_abs": maximum, "mean_abs": mean,
                         "pass": maximum <= args.max_error and mean <= args.mean_error,
                         "native_observation": result.stderr.strip()})
    import exiv2
    image = exiv2.ImageFactory.open(str(args.fixture))
    image.readMetadata()
    exif = image.exifData()
    for key in ("Exif.Image.Make", "Exif.Image.Model", "Exif.Photo.LensModel"):
        camera[key] = str(exif[key].value()) if key in exif else ""
    report = {"reference_commit": "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc",
              "source_url": args.source_url,
              "sha256": hashlib.sha256(args.fixture.read_bytes()).hexdigest(),
              "rawpy_version": rawpy.__version__, "rawpy_libraw_version": rawpy.libraw_version,
              "camera": camera, "budget": {"max_abs": args.max_error, "mean_abs": args.mean_error},
              "results": rows}
    print(json.dumps(report, indent=2))
    return 0 if all(row["pass"] for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
