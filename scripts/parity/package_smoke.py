#!/usr/bin/env python3
"""Exercise installed package image/metadata/error/RAW paths before publication."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.request

import colour
import exiv2
import numpy as np
import OpenImageIO as oiio


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--raw-helper', type=Path, required=True)
    parser.add_argument('--gui', type=Path)
    parser.add_argument('--data-dir', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path, required=True)
    args = parser.parse_args()
    cli, helper, data, evidence = (value.resolve() for value in
                                 (args.cli, args.raw_helper, args.data_dir, args.evidence_dir))
    evidence.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, SPEKTRAFILM_BACKEND='cpu')
    environment.pop('LD_LIBRARY_PATH', None)
    source = evidence / 'source.tif'
    pixels = np.array([[[-0.25, 0.5, 1.25], [0.1, 0.499, 0.01]]], dtype=np.float32)
    expected = colour.RGB_to_RGB(pixels.astype(np.float64), 'sRGB', 'sRGB',
                                apply_cctf_decoding=False, apply_cctf_encoding=False).astype(np.float32)
    writer = oiio.ImageOutput.create(str(source))
    assert writer and writer.open(str(source), oiio.ImageSpec(2, 1, 3, oiio.FLOAT))
    assert writer.write_image(pixels) and writer.close()
    metadata = exiv2.ImageFactory.open(str(source))
    metadata.readMetadata()
    metadata.exifData()['Exif.Image.Artist'] = 'package smoke artist'
    metadata.exifData()['Exif.Image.Orientation'] = 6
    metadata.iptcData()['Iptc.Application2.Caption'] = 'package smoke caption'
    metadata.xmpData()['Xmp.dc.description'] = 'package smoke description'
    metadata.writeMetadata()
    params = evidence / 'identity.json'
    params.write_text(json.dumps({'taps': {'inject': 'rgb_in', 'collect': 'rgb_in'},
                                'io': {'input_color_space': 'sRGB', 'input_cctf_decoding': False,
                                       'output_color_space': 'sRGB', 'output_cctf_encoding': False}}))
    observations = []

    def process(input_path, output, depth, succeeds=True):
        command = [str(cli), 'process', str(input_path), '--output', str(output),
                   '--film', 'kodak_portra_400', '--scan-film', '--params', str(params),
                   '--data-dir', str(data), '--bit-depth', str(depth),
                   '--saving-color-space', 'sRGB', '--saving-cctf-encoding', 'false']
        result = subprocess.run(command, cwd=evidence, env=environment, capture_output=True,
                                text=True, timeout=120)
        (evidence / (output.name + '.log')).write_text(result.stdout + result.stderr)
        assert (result.returncode == 0) == succeeds, result.stderr
        if not succeeds:
            assert not output.exists()
            return
        reader = oiio.ImageInput.open(str(output))
        assert reader, output
        spec = reader.spec()
        actual = np.array(reader.read_image(oiio.FLOAT))
        reader.close()
        assert (spec.width, spec.height, spec.nchannels) == (2, 1, 3)
        if output.suffix == '.tif':
            saved = exiv2.ImageFactory.open(str(output))
            saved.readMetadata()
            assert saved.exifData()['Exif.Image.Artist'].toString() == 'package smoke artist'
            assert saved.exifData()['Exif.Image.Orientation'].toString() == '1'
            assert saved.exifData()['Exif.Photo.PixelXDimension'].toString() == '2'
            assert saved.exifData()['Exif.Photo.PixelYDimension'].toString() == '1'
            assert saved.iptcData()['Iptc.Application2.Caption'].toString() == 'package smoke caption'
            assert saved.xmpData()['Xmp.dc.description'].toString() == 'lang="x-default" package smoke description'
            assert bytes(spec.getattribute('ICCProfile')) == (data / 'icc/ellelstone/sRGB-elle-V2-g10.icc').read_bytes()
        if depth == 32:
            np.testing.assert_allclose(actual, expected, atol=2e-7, rtol=0)
            assert actual[0, 0, 0] < 0 and actual[0, 0, 2] > 1
        elif output.suffix == '.tif':
            np.testing.assert_allclose(actual, np.clip(expected, 0, 1), atol=1 / 65535, rtol=0)
        observations.append({'output': output.name, 'format': str(spec.format), 'shape': list(actual.shape)})

    process(source, evidence / 'float.tif', 32)
    process(source, evidence / 'integer.tif', 16)
    process(source, evidence / 'float.exr', 32)
    corrupt = evidence / 'corrupt.png'
    corrupt.write_bytes(b'not an image')
    process(corrupt, evidence / 'corrupt-output.tif', 32, False)
    process(source, evidence / 'invalid-depth.exr', 8, False)
    raw = evidence / 'kodak.KDC'
    url = 'https://raw.githubusercontent.com/letmaik/rawpy/main/test/RAW_KODAK_DC50_%C3%A9.KDC'
    expected_hash = '37e290dbd0053f00e508d02a6b3a2a990432dad1eb74c40a52ca899f0f225ecc'
    raw.write_bytes(urllib.request.urlopen(url, timeout=60).read())
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == expected_hash
    raw_output = evidence / 'raw.tif'
    result = subprocess.run([str(helper), str(raw), str(raw_output), '--lens-correction'],
                            cwd=evidence, env=environment, capture_output=True, text=True, timeout=120)
    (evidence / 'raw.log').write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stderr
    reader = oiio.ImageInput.open(str(raw_output))
    assert reader
    spec = reader.spec()
    decoded = np.array(reader.read_image(oiio.FLOAT))
    reader.close()
    assert (spec.width, spec.height, spec.nchannels) == (768, 512, 3)
    assert np.isfinite(decoded).all() and float(decoded.max() - decoded.min()) > 0.1
    observations.append({'raw_source': url, 'raw_sha256': expected_hash,
                         'raw_shape': list(decoded.shape), 'decoder': result.stderr.strip()})
    if args.gui:
        gui_path = args.gui.resolve()
        with (evidence / 'gui.log').open('w') as log:
            process = subprocess.Popen([str(gui_path)], cwd=evidence, env=environment,
                                       stdout=log, stderr=subprocess.STDOUT)
            try:
                time.sleep(10)
                assert process.poll() is None, f'Packaged GUI exited during startup; see {evidence / "gui.log"}'
                observations.append({'gui': str(gui_path), 'running_after_seconds': 10})
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=10)
    (evidence / 'observations.json').write_text(json.dumps(observations, indent=2) + '\n')
    print('PASS package image depths, float headroom, EXIF/IPTC/XMP/ICC, errors and real RAW decode')


if __name__ == '__main__':
    main()
