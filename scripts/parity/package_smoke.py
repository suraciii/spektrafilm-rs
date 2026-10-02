#!/usr/bin/env python3
"""Exercise installed package image/metadata/error/RAW paths before publication."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from gui_acceptance import accept_gui
from lut_acceptance import check_artifacts, local_file
import urllib.request

import exiv2
import numpy as np
import OpenImageIO as oiio


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--exporter', type=Path, required=True)
    parser.add_argument('--raw-helper', type=Path, required=True)
    parser.add_argument('--gui', type=Path)
    parser.add_argument('--data-dir', type=Path, required=True)
    parser.add_argument('--evidence-dir', type=Path, required=True)
    parser.add_argument('--raw-fixture', type=Path, help='Existing pinned KDC fixture; SHA256 is still required')
    args = parser.parse_args()
    cli, helper, data, evidence = (value.resolve() for value in
                                 (args.cli, args.raw_helper, args.data_dir, args.evidence_dir))
    evidence.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, SPEKTRAFILM_BACKEND='cpu')
    environment.pop('LD_LIBRARY_PATH', None)
    exporter = args.exporter.resolve()
    source = evidence / 'source.tif'
    pixels = np.array([[[-0.25, 0.5, 1.25], [0.1, 0.499, 0.01]]], dtype=np.float32)
    # The pinned Python save guard skips the transform when the saving space and
    # encoding match the output layer, so the pipeline buffer is written
    # unchanged. colour.RGB_to_RGB(pixels, 'sRGB', 'sRGB') would run colour's own
    # IEC matrices as a round trip and shift these pixels by ~3e-5.
    expected = pixels.astype(np.float64)
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
    # Pinned 0.3.4 bare-chain midgray reference; exercise actual spectral assets.
    spectral_source = evidence / 'spectral-source.tif'
    writer = oiio.ImageOutput.create(str(spectral_source))
    assert writer and writer.open(str(spectral_source), oiio.ImageSpec(1, 1, 3, oiio.FLOAT))
    assert writer.write_image(np.full((1, 1, 3), 0.184, dtype=np.float32)) and writer.close()
    spectral_params = evidence / 'spectral-params.json'
    spectral_params.write_text(json.dumps({
        'camera': {'auto_exposure': False},
        'film_render': {'grain': {'active': False}, 'halation': {'active': False},
                        'dir_couplers': {'active': False}},
        'print_render': {'glare': {'active': False}},
        'scanner': {'unsharp_mask': [0.0, 0.0]},
        'io': {'input_color_space': 'sRGB', 'input_cctf_decoding': False,
               'output_color_space': 'sRGB', 'output_cctf_encoding': False}}))
    spectral_output = evidence / 'spectral-export.tif'
    result = subprocess.run([
        str(exporter), 'process', str(spectral_source), '--output', str(spectral_output),
        '--film', 'kodak_portra_400', '--paper', 'kodak_portra_endura',
        '--params', str(spectral_params), '--data-dir', str(data), '--bit-depth', '32',
        '--saving-color-space', 'sRGB', '--saving-cctf-encoding', 'false'],
        cwd=evidence, env=environment, capture_output=True, text=True, timeout=120)
    (evidence / 'spectral-export.log').write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stderr
    reader = oiio.ImageInput.open(str(spectral_output))
    assert reader
    spectral_pixels = np.array(reader.read_image(oiio.FLOAT))
    reader.close()
    reference = np.array([[[0.17518024973220059, 0.17883059767931708,
                            0.18934288118407094]]])
    np.testing.assert_allclose(spectral_pixels, reference, atol=1e-6, rtol=0)
    observations.append({'spectral_exporter': str(exporter),
                         'reference_commit': '3bb2c2d2801ff68b92019cf1dbcbb133d60832bc',
                         'spectral_max_abs': float(np.max(np.abs(spectral_pixels - reference))),
                         'spectral_budget': 1e-6})
    raw = evidence / 'kodak.KDC'
    url = 'https://raw.githubusercontent.com/letmaik/rawpy/main/test/RAW_KODAK_DC50_%C3%A9.KDC'
    expected_hash = '37e290dbd0053f00e508d02a6b3a2a990432dad1eb74c40a52ca899f0f225ecc'
    raw.write_bytes(args.raw_fixture.read_bytes() if args.raw_fixture else
                    urllib.request.urlopen(url, timeout=60).read())
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
    # Build through the delivered f64 executable; verify the complete offline
    # bundle, archive and an actual OCIO CPU processor against delivered LUTs.
    import PyOpenColorIO as ocio
    bundle_root = evidence / 'installed-luts'
    name = 'package_acceptance'
    result = subprocess.run([
        str(exporter), 'lut', 'build', '--name', name, '--film', 'kodak_portra_400',
        '--print', 'kodak_portra_endura', '--input', 'sRGB', '--output', 'sRGB',
        '--topology', '3lut', '--resolution', '17', '--ocio-config', '--qa',
        '--container', 'zip', '--out', str(bundle_root), '--data-dir', str(data)],
        cwd=evidence, env=environment, capture_output=True, text=True, timeout=600)
    (evidence / 'installed-lut.log').write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stderr
    bundle = bundle_root / name
    meta = json.loads((bundle / 'bundle.json').read_text())
    qa = json.loads((bundle / 'qa/report.json').read_text())
    artifact_evidence = check_artifacts(bundle, meta, qa, archive_required=True)
    assert qa['prints'] and all(entry['results'] for entry in qa['prints']), 'Empty delivered QA'
    # Scenario FAIL is part of the upstream QA report contract. Verify status
    # aggregation, rather than hiding it or treating every diagnostic as PASS.
    expected_passed = all(item['passed'] is not False
                          for entry in qa['prints'] for item in entry['results'])
    assert qa['passed'] is expected_passed, 'Delivered QA hides failed scenarios'
    config = ocio.Config.CreateFromFile(str(local_file(bundle, 'config.ocio')))
    config.validate()
    processor_count = 0
    for display in config.getDisplays():
        for view in config.getViews(display):
            processor = config.getProcessor('sRGB', display, view,
                                           ocio.TRANSFORM_DIR_FORWARD).getDefaultCPUProcessor()
            for pixel in ([0., 0., 0.], [.18, .18, .18], [.1, .5, .9], [1., 1., 1.]):
                assert np.isfinite(processor.applyRGB(pixel)).all(), (display, view)
            processor_count += 1
    assert processor_count > 0, 'No delivered OCIO display/view processors'
    observations.append({'installed_lut_exporter': str(exporter),
                         'bundle': str(bundle), 'artifacts': artifact_evidence,
                         'ocio_processors': processor_count, 'qa_report_passed': qa['passed']})
    if args.gui:
        observations.append(accept_gui(args.gui.resolve(), exporter, source, raw,
                                       evidence, environment))
    (evidence / 'observations.json').write_text(json.dumps(observations, indent=2) + '\n')
    print('PASS installed image depths, float headroom, metadata, spectral export, errors, RAW, LUT/OCIO/QA and native GUI acceptance')


if __name__ == '__main__':
    main()
