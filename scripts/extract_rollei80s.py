#!/usr/bin/env python3
"""Extract the Rollei Retro 80S page-3 spectral circles from the 2021 PDF.

The plot is a vector chart: 15 filled circles on a 350-800 nm axis. The
ordinate is a relative log-sensitivity scale (0-3) with an arbitrary zero.
This script only updates the spectral block of the pinned source JSON; the
10-zone densitometry is transcribed from the APHOG page and checked there.
"""
import argparse
import hashlib
import json
import re
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PDF_HASH = 'f37754ce4548ceab6e6902143118cea712d5bf4f1dee32488bfa37dfb02e0f45'
# Page-3 chart geometry in PDF points, from the vector frame and axis labels.
Y_BASE = 196.814453
Y_TOP = 90.819013
X_FIRST = 346.146484
X_LAST = 528.445312


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extract(pdf_path):
    if digest(pdf_path) != PDF_HASH:
        raise ValueError('unexpected Rollei PDF identity')
    with tempfile.TemporaryDirectory() as directory:
        svg = Path(directory) / 'page3.svg'
        subprocess.run(
            ['pdftocairo', '-f', '3', '-l', '3', '-svg', str(pdf_path), str(svg)],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        text = svg.read_text()
    centres = []
    for match in re.finditer(r'<path([^>]*)>', text):
        attrs = match.group(1)
        if 'fill=' not in attrs or 'stroke=' in attrs:
            continue
        d = re.search(r'd="([^"]*)"', attrs)
        if not d or d.group(1).count('C') < 4 or len(d.group(1)) >= 400:
            continue
        nums = [float(v) for v in re.findall(r'-?\d+\.?\d*', d.group(1))]
        xs, ys = nums[0::2], nums[1::2]
        cx, cy = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
        if 340 <= cx <= 530 and 130 <= cy <= 200:
            centres.append((cx, cy))
    centres.sort()
    if len(centres) != 15:
        raise ValueError(f'expected 15 spectral circles, found {len(centres)}')
    if abs(centres[0][0] - X_FIRST) > 0.02 or abs(centres[-1][0] - X_LAST) > 0.02:
        raise ValueError('unexpected spectral x extent')
    points = []
    for cx, cy in centres:
        wavelength = 350.0 + (cx - X_FIRST) / (X_LAST - X_FIRST) * 450.0
        log_s = (Y_BASE - cy) / (Y_BASE - Y_TOP) * 3.0
        points.append([round(wavelength, 2), round(log_s, 4)])
    return points


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pdf', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    args = parser.parse_args()
    source = json.loads(args.source.read_text())
    points = extract(args.pdf)
    source['spectral_points'] = points
    source['spectral_axis'] = (
        'Rollei page-3 ordinate 0.0-3.0, read as relative log sensitivity with '
        'an arbitrary zero; 15 plotted circle centres from 350 to 800 nm. Not '
        'absolute log sensitivity and not equal-energy corrected.'
    )
    args.source.write_text(json.dumps(source, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps(points, indent=2))


if __name__ == '__main__':
    main()
