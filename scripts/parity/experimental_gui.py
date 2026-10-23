#!/usr/bin/env python3
"""Native continuous-inspector GUI gate through X11, OCR and real file choosers.

Run under dbus-run-session + xvfb-run after building the exact recorded commit.
This gate records GUI behavior; numerical Python/Rust parity is a separate gate.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time

import numpy as np
import OpenImageIO as oiio

from gui_acceptance import X11, require, wait_for
from gui_control_acceptance import control_boundaries, export_controls, export_image, saved, profile_roundtrip, cancel_export


def parameters_equal(actual, expected):
    if isinstance(actual, bool) or isinstance(expected, bool):
        return actual is expected
    if isinstance(actual, (int, float)) and isinstance(expected, (int, float)):
        return math.isclose(actual, expected, rel_tol=1e-6, abs_tol=1e-6)
    if isinstance(actual, dict) and isinstance(expected, dict):
        return actual.keys() == expected.keys() and all(
            parameters_equal(actual[key], expected[key]) for key in actual)
    if isinstance(actual, list) and isinstance(expected, list):
        return len(actual) == len(expected) and all(
            parameters_equal(left, right) for left, right in zip(actual, expected))
    return actual == expected

UPSTREAM = '28bf883e1672e884307edc75852549376e13644e'
ROUTES = [
    'input',
    'input > film > scan',
    'input > film > print > scan',
    'input > convert-film > print > scan',
    'input > convert-film > scan-minus-base',
    'input > convert-film > scan',
]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def git(repo, *args):
    return subprocess.run(['git', *args], cwd=repo, check=True,
                          capture_output=True, text=True).stdout.strip()


def words(text):
    return re.findall(r'[a-z0-9]+', text.lower())




def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def parameter_state(value):
    result = json.loads(json.dumps(value))
    # Opening a chooser intentionally updates its remembered directory.
    # All runtime, canonical GUI, and other extension fields must still match.
    result.get('rust', {}).pop('dialog_dirs', None)
    return result


class ExperimentalDesktop(X11):
    """Use production stage/search owners, sharing X11 capture and choosers."""

    def start(self, gui, env, image=None):
        env = self.gui_environment(env)
        self.chooser_ready(env)
        self.settings_open = False
        self.search_point = None
        self.proc = subprocess.Popen([str(gui)] + ([str(image)] if image else []),
                                    cwd=self.root, env=env, stdout=self.log, stderr=self.log)
        def window():
            require(self.proc.poll() is None, 'Native GUI exited; inspect gui.log')
            ids = self.xd('search', '--onlyvisible', '--pid', self.proc.pid,
                          '--name', '^spektrafilm$', check=False)
            return ids.splitlines()[-1] if ids else None
        self.window = wait_for(window, 'native inspector window', 45)
        self.xd('windowactivate', '--sync', self.window)
        self.xd('windowsize', '--sync', self.window, 1460, 980)
        self.xd('windowmove', '--sync', self.window, 0, 0)
        wait_for(lambda: self.locate('Search controls', right=True),
                 'rendered control search', 120)

    def read(self):
        image, bbox, lines = super().read()
        # The operation status and global actions now belong to the header.
        # A dedicated complete-row pass avoids splitting it at viewer boundaries.
        from PIL import ImageOps, ImageStat
        # Read centered windows as complete rows too: enum labels and values
        # must stay on the same observed row when choosing an Export option.
        regions = [(0, 0, image.width, 26, 7),
                   (0, 26, image.width, 48, 7),
                   (0, 48, image.width, 72, 7),
                   (max(0, image.width - 520), 150, image.width, image.height, 6),
                   (0, 0, int(image.width * .65), image.height - 70, 6),
                   (int(image.width * .18), 150, int(image.width * .82), image.height - 70, 6)]
        for left, top, right, bottom, segmentation in regions:
            crop = ImageOps.grayscale(image.crop((left, top, right, bottom)))
            if ImageStat.Stat(crop).median[0] < 128:
                crop = ImageOps.invert(crop)
            crop = ImageOps.autocontrast(crop)
            data = self.ocr.image_to_data(crop.resize((crop.width * 3, crop.height * 3)),
                                          config=f'--psm {segmentation}', output_type=self.ocr.Output.DICT, timeout=30)
            grouped = {}
            for index, text in enumerate(data['text']):
                if text.strip():
                    key = (data['block_num'][index], data['par_num'][index], data['line_num'][index])
                    grouped.setdefault(key, []).append((text, left + data['left'][index] / 3,
                        top + data['top'][index] / 3, data['width'][index] / 3, data['height'][index] / 3))
            lines.extend(grouped.values())
        return image, bbox, lines

    def locate(self, label, *, exact=False, bottom=False, right=False, header=False):
        image, _, lines = self.read()
        candidates = []
        for line in lines:
            if not line or (right and line[0][1] < image.width - 520):
                continue
            if header and line[0][2] > 65:
                continue
            if bottom and line[0][2] < image.height - 150:
                continue
            if bottom and label in ('PREVIEW', 'SCAN', 'SAVE'):
                candidates.extend((word[1] + word[3] / 2, word[2] + word[4] / 2)
                                  for word in line if word[0].strip('[]()|') == label)
            elif not exact or words(' '.join(word[0] for word in line)) == words(label):
                expected = ''.join(words(label))
                for start in range(len(line)):
                    text = ''
                    for end in range(start, len(line)):
                        text += ''.join(words(line[end][0]))
                        if text == expected:
                            span = line[start:end + 1]
                            candidates.append((span[0][1] + span[0][3] / 2,
                                               span[0][2] + span[0][4] / 2))
                            break
                        if len(text) >= len(expected):
                            break
        return candidates[0] if candidates else None

    def point_click(self, point):
        self.xd('mousemove', '--window', self.window, int(point[0]), int(point[1]))
        self.xd('click', 1)
        time.sleep(.3)

    def click(self, label, right=False, *, exact=False, bottom=False, header=False):
        point = wait_for(lambda: self.locate(label, exact=exact, bottom=bottom,
                                            right=right, header=header),
                         f'visible control {label}', 25)
        self.snap('click-' + '-'.join(words(label)))
        if label in ('PREVIEW', 'SCAN', 'SAVE', 'Export file'):
            self.drain_reports()
        self.point_click(point)

    def close_settings(self):
        if self.settings_open:
            self.click('Close settings')
            self.settings_open = False

    def search(self, query):
        self.close_settings()
        if self.search_point is None:
            self.search_point = wait_for(lambda: self.locate('Search controls', right=True),
                                         'control search editor', 20)
        self.point_click(self.search_point)
        self.xd('key', 'ctrl+a')
        if query:
            self.xd('type', '--clearmodifiers', '--', query)
        else:
            self.xd('key', 'BackSpace')
        time.sleep(.4)

    def reveal(self, query, owner, section):
        self.search(query)
        locator = f'Locate {owner}: {section}'
        self.click(locator, right=True)
        self.settings_open = owner == 'Settings'
        self.snap('located-' + '-'.join(words(query)))

    def stage(self, name):
        self.search('')
        # The first matching label is the persistent jump strip, above headings.
        self.click(name, right=True)
        self.snap('stage-' + '-'.join(words(name)))

    def open_image(self, path, label):
        self.close_settings()
        self.click('Open', header=True)
        self.dialog(path)
        # Clear the pointer/hover tooltip from the source and header labels.
        self.xd('mousemove', '--window', self.window, 700, 100)
        return self.wait_text(r'Source:\s*' + re.escape(path.name), label)

    def state_action(self, label, path, save=False):
        self.reveal('action.save_state' if save else 'action.load_state',
                    'Settings', 'GUI parameters')
        self.click(label)
        self.dialog(path, save)
        result = None
        if save:
            wait_for(path.is_file, 'native saved state', 20)
            result = json.loads(path.read_text())
        else:
            self.wait_text(r'Loaded GUI state', 'state-loaded-' + path.stem)
        self.close_settings()
        return result

    def startup_action(self, label):
        self.reveal('action.save_startup' if label == 'Save current as default'
                    else 'action.restore_factory', 'Settings', 'GUI parameters')
        self.click(label)
        if label == 'Restore factory default':
            def confirmation():
                _, _, lines = self.read()
                for line in lines:
                    if words(' '.join(word[0] for word in line)) == ['restore', 'cancel']:
                        word = line[0]
                        return word[1] + word[3] / 2, word[2] + word[4] / 2
            self.point_click(wait_for(confirmation, 'factory Restore confirmation', 20))
            self.wait_text(r'Restored factory default', 'factory-reset-confirmed')
        self.close_settings()

    def choose(self, label, option, *, below=False):
        # The locator's tooltip can cover its destination with duplicate text.
        # Move away before observing the editing owner, not after taking OCR.
        self.xd('mousemove', '--window', self.window, 700, 100)
        time.sleep(.4)
        image, _, lines = self.read()
        rows = []
        expected = ''.join(words(label))
        for line in lines:
            if (not line or words(line[0][0]) == ['locate']
                    or any(words(word[0]) == ['reset'] for word in line)):
                continue
            for start in range(len(line)):
                text = ''
                for end in range(start, len(line)):
                    text += ''.join(words(line[end][0]))
                    if text == expected:
                        span = line[start:end + 1]
                        y = span[0][2] + span[0][4] / 2
                        right = max(word[1] + word[3] for word in span)
                        values = [word for word in line[end + 1:]
                                  if word[1] > right and abs(word[2] + word[4] / 2 - y) < 12]
                        # OCR can split a badge, selected value, and chevron
                        # into separate lines even when they share one UI row.
                        values.extend(word for candidate in lines for word in candidate
                                      if word[1] > right
                                      and abs(word[2] + word[4] / 2 - y) < 8)
                        if not values:
                            values = [word for word in line[:start]
                                      if word[1] + word[3] < span[0][1]
                                      and abs(word[2] + word[4] / 2 - y) < 12]
                        rows.append((span, values, y))
                        break
                    if len(text) >= len(expected):
                        break
        require(rows, f'Choice label not visible: {label}')
        if not below and rows and all(
                not any(words(word[0]) != ['experimental'] for word in row[1]) for row in rows):
            from PIL import ImageOps
            span, _, y = max(rows, key=lambda row: row[2])
            left = int(max(word[1] + word[3] for word in span))
            top = max(0, int(y - 11))
            crop = ImageOps.invert(ImageOps.grayscale(
                image.crop((left, top, image.width, int(y + 12)))))
            data = self.ocr.image_to_data(crop.resize((crop.width * 5, crop.height * 5)),
                                          config='--psm 7', output_type=self.ocr.Output.DICT)
            values = [(text, left + data['left'][i] / 5, top + data['top'][i] / 5,
                       data['width'][i] / 5, data['height'][i] / 5)
                      for i, text in enumerate(data['text']) if text.strip()]
            rows = [(span, values, y)]
        if not below:
            rows = [(span, [word for word in values
                            if words(word[0]) != ['experimental']], y)
                    for span, values, y in rows]
            rows = [row for row in rows if row[1]
                    and not any(words(word[0]) in (['reset'], ['modified']) for word in row[1])]
            require(rows, f'Choice value not visible: {label}')
        inspector_rows = [row for row in rows if row[0][0][1] >= image.width - 350]
        # Multiple OCR crops observe the same row. Prefer the real inspector
        # owner over header summaries, and the value row below its module title.
        span, values, y = max(inspector_rows or rows, key=lambda row: (row[2], len(row[1])))
        x = span[0][1]
        if below:
            candidates = [word for line in lines for word in line
                          if word[1] >= x - 5 and 10 < word[2] - y < 40]
        else:
            candidates = values
            if words(' '.join(word[0] for word in values)) == words(option):
                return
        require(candidates, f'Choice value not visible: {label}')
        # The final word is the observed combobox chevron (or the end of its
        # selected text), not a duplicate label from an overlapping OCR crop.
        selected = max(candidates, key=lambda word: word[1] + word[3])
        self.records.append({'choice_label': label, 'choice_requested': option,
                             'observed_label_span': span, 'observed_value_words': candidates,
                             'choice_click': [selected[1] + selected[3] / 2, selected[2] + selected[4] / 2]})
        self.point_click((selected[1] + selected[3] / 2, selected[2] + selected[4] / 2))
        self.snap('choice-open-' + '-'.join(words(label)))
        for direction in (4, 5):
            for _ in range(16):
                frame, _, rows = self.read()
                # Popup rows need their own text block: the retained viewer
                # image and adjacent inspector headings otherwise merge them.
                from PIL import ImageOps, ImageStat
                left = max(0, int(min(word[1] for word in candidates)) - 12)
                top = max(0, int(y + 9))
                crop = ImageOps.grayscale(frame.crop((left, top, frame.width,
                                                       min(frame.height, top + 430))))
                if ImageStat.Stat(crop).median[0] < 128:
                    crop = ImageOps.invert(crop)
                data = self.ocr.image_to_data(crop.resize((crop.width * 4, crop.height * 4)),
                                              config='--psm 6', output_type=self.ocr.Output.DICT)
                grouped = {}
                for index, text in enumerate(data['text']):
                    if text.strip():
                        key = (data['block_num'][index], data['par_num'][index], data['line_num'][index])
                        grouped.setdefault(key, []).append((text, left + data['left'][index] / 4,
                            top + data['top'][index] / 4, data['width'][index] / 4, data['height'][index] / 4))
                rows = list(grouped.values()) + rows
                options = [line for line in rows
                           if words(' '.join(word[0] for word in line)) == words(option)]
                if options:
                    line = options[0]
                    self.snap('choice-' + '-'.join(words(option)), frame, rows)
                    self.point_click((line[0][1] + 8, line[0][2] + line[0][4] / 2))
                    return
                self.xd('mousemove', '--window', self.window, int(x + 45), int(y + 70))
                self.xd('click', direction)
                time.sleep(.15)
        self.snap('choice-missing-' + '-'.join(words(option)))
        raise RuntimeError(f'Native choice option not visible: {label}: {option}')

    def choose_route(self, index):
        self.reveal('workflow.route', 'Input and capture', 'Workflow')
        self.choose('workflow', ROUTES[index])

    def surfaces(self):
        for label in ('Open', 'Undo', 'Redo', 'Settings', 'Export'):
            require(self.locate(label, header=True), f'Header lacks {label}')
        for name in ('Input and capture', 'Film', 'Print', 'Scan', 'Output'):
            self.stage(name)
            for label in ('PREVIEW', 'SCAN', 'SAVE'):
                require(self.locate(label, bottom=True), f'{name}: persistent {label} missing')
        self.reveal('action.save_state', 'Settings', 'GUI parameters')
        require(self.locate('Save current to file'), 'Settings state controls missing')
        self.close_settings()

    def export_options(self, path, float_export=False):
        formats = {'.jpg': 'JPEG', '.jpeg': 'JPEG', '.png': 'PNG',
                   '.tif': 'TIFF', '.tiff': 'TIFF', '.exr': 'EXR'}
        requested = formats.get(path.suffix.lower())
        require(requested, f'Unsupported acceptance export extension: {path}')
        self.xd('mousemove', '--window', self.window, 700, 100)
        time.sleep(.4)
        self.snap('export-dialog-controls')
        wait_for(lambda: self.locate('Backend'), 'Export dialog', 20)
        image, _, lines = self.read()
        cpu = next((word[0] for line in lines for word in line if word[0].startswith('CPU')), None)
        require(cpu, 'Export did not open with a visible CPU backend')
        self.choose('Format', requested)
        if requested in ('TIFF', 'EXR'):
            self.choose('Bit depth', '32 bit' if float_export else '16 bit')
        self.click('Export file')


def fixture(path):
    y, x = np.mgrid[:24, :32]
    rgb = np.stack((.06 + .65*x/31, .09 + .55*y/23,
                    .12 + .5*(x+32*y)/(32*24-1)), axis=-1).astype('float32')
    writer = oiio.ImageOutput.create(str(path))
    require(writer and writer.open(str(path), oiio.ImageSpec(32, 24, 3, oiio.FLOAT)),
            'Cannot create float RGB fixture')
    require(writer.write_image(rgb), 'Cannot write RGB fixture')
    writer.close()


def read_output(path, size=(32, 24)):
    image = oiio.ImageInput.open(str(path))
    require(image, f'Cannot read saved render {path}')
    spec = image.spec()
    pixels = np.asarray(image.read_image(oiio.FLOAT))
    image.close()
    require(np.isfinite(pixels).all(), f'Nonfinite render {path}')
    require((spec.width, spec.height, spec.nchannels) == (*size, 3),
            f'Unexpected render geometry {path}: {spec.width}x{spec.height}x{spec.nchannels}')
    return {'sha256': sha(path), 'size': [spec.width, spec.height],
            'min': float(pixels.min()), 'max': float(pixels.max())}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gui', required=True, type=Path)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--evidence', required=True, type=Path)
    parser.add_argument('--factory-reference', required=True, type=Path,
                        help='Independently generated pinned upstream GUI state')
    parser.add_argument('--raw', required=True, type=Path)
    parser.add_argument('--development-smoke', action='store_true',
                        help='Allow dirty development run; never reports acceptance pass')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    require(git(args.upstream, 'rev-parse', 'HEAD') == UPSTREAM, 'Wrong upstream checkout')
    root = args.evidence.resolve()
    root.mkdir(parents=True, exist_ok=False)
    commit = git(repo, 'rev-parse', 'HEAD')
    dirty = git(repo, 'status', '--porcelain', '--untracked-files=all')
    require(not dirty or args.development_smoke, 'Commit implementation before recording acceptance')
    gui = args.gui.resolve()
    config = root/'config'
    config.mkdir()
    source = root/'input.tif'
    fixture(source)
    factory = json.loads(args.factory_reference.read_text())
    oracle_dir = Path(__file__).parent/'fixtures/gui_28bf883'
    provenance = json.loads((oracle_dir/'provenance.json').read_text())
    require(sha(args.factory_reference) == provenance['artifact_sha256']['factory_state.json'],
            'Factory reference is not the retained independent upstream oracle')
    # Explicit contract changes since the retained upstream oracle: the retired
    # output-lightness leaf and the optional inherited capture illuminant.
    factory['output_gamut_compress'].pop('lightness_compression')
    factory['camera']['capture_illuminant'] = None
    rust_factory = json.loads((repo/'crates/spektrafilm-gui/src/factory_state.json').read_text())
    require(parameters_equal({key: value for key, value in rust_factory.items() if key != 'rust'},
                             {key: value for key, value in factory.items() if key != 'rust'}),
            'Rust shared factory differs from independent upstream oracle')
    seed = json.loads(json.dumps(factory))
    seed['simulation']['auto_preview'] = False
    seed['grain']['active'] = False
    seed['camera']['auto_exposure'] = False
    seed['input_image']['input_color_space'] = 'sRGB'
    seed['input_image']['input_cctf_decoding'] = False
    seed['simulation']['output_color_space'] = 'sRGB'
    seed['simulation']['saving_color_space'] = 'sRGB'
    seed['simulation']['saving_cctf_encoding'] = False
    seed['rust'] = dict(rust_factory['rust'], export_format='tiff', save_bit_depth=32)
    seed_path = root/'render-seed.json'
    save_json(seed_path, seed)
    env = dict(os.environ, SPEKTRAFILM_CONFIG_DIR=str(config),
               SPEKTRAFILM_DATA_DIR=str(repo/'data'), SPEKTRAFILM_GUI_RENDERER='glow')
    data_hashes = {str(p.relative_to(repo/'data')): sha(p)
                   for group in ('profiles', 'presets', 'filters')
                   for p in sorted((repo/'data'/group).rglob('*')) if p.is_file()}
    report = {'upstream_commit': UPSTREAM, 'rust_commit': commit,
              'rust_worktree_dirty': bool(dirty), 'gui_sha256': sha(gui),
              'fixture_sha256': sha(source), 'factory_state_sha256':
              sha(repo/'crates/spektrafilm-gui/src/factory_state.json'),
              'upstream_factory_sha256': sha(args.factory_reference),
              'seed_state_sha256': sha(seed_path),
              'data_sha256': data_hashes, 'routes': [], 'status': 'running'}
    driver = ExperimentalDesktop(root)
    try:
        driver.start(gui, env)
        fresh = saved(driver, root, 'fresh-startup')
        for section, expected in factory.items():
            if section != 'rust':
                require(parameters_equal(fresh.get(section), expected), f'Fresh startup differs: {section}')
        require(parameters_equal({key: fresh['rust'].get(key) for key in rust_factory['rust']},
                                 rust_factory['rust']),
                'Fresh Rust export defaults differ from factory')
        driver.surfaces()
        control_boundaries(driver, root, factory)
        profile_roundtrip(driver, root)
        driver.state_action('Load from file', seed_path)
        driver.open_image(source, 'imported-rgb')
        driver.click('PREVIEW', bottom=True)
        driver.rendered('initial-preview', action='Preview')
        for index, route in enumerate(ROUTES):
            driver.choose_route(index)
            state_path = root/f'route-{index}.json'
            state = driver.state_action('Save current to file', state_path, save=True)
            require(state['simulation']['route'] == route,
                    f'Workflow selection {index} saved {state["simulation"].get("route")}')
            require('scan_film' not in state['simulation'], 'Legacy scan_film persisted')
            require('workflow' not in state['simulation'], 'Legacy nested workflow persisted')
            driver.click('PREVIEW', bottom=True)
            status = driver.rendered(f'route-{index}-preview', action='Preview')
            driver.state_action('Save current to file', root/f'route-{index}-before-scan.json', save=True)
            driver.wait_text(r'Saved GUI state', f'route-{index}-before-scan')
            driver.click('SCAN', bottom=True)
            status = driver.rendered(f'route-{index}-scan', action='Scan')
            output = root/f'route-{index}.tif'
            driver.click('SAVE', bottom=True)
            driver.dialog(output, save=True)
            wait_for(output.is_file, 'native saved route output', 30)
            output_info = read_output(output)
            driver.state_action('Load from file', state_path)
            reloaded_path = root/f'route-{index}-reloaded.json'
            reloaded = driver.state_action('Save current to file', reloaded_path, save=True)
            require(parameter_state(reloaded) == parameter_state(state),
                    f'Canonical state roundtrip differs for {route}')
            report['routes'].append({'route': route, 'state_sha256': sha(state_path),
                                     'state': state_path.name, 'output': output.name,
                                     'render_status': status, **output_info})
        driver.startup_action('Save current as default')
        saved_default = json.loads((config/'gui_default_state.json').read_text())
        driver.close()
        driver.start(gui, env)
        restart_path = root/'restarted.json'
        restarted = driver.state_action('Save current to file', restart_path, save=True)
        require(parameter_state(restarted) == parameter_state(saved_default),
                'Startup default lost canonical parameters')
        driver.state_action('Load from file', root/'saved-user-values.json')
        driver.startup_action('Save current as default')
        driver.close()
        driver.start(gui, env)
        retained = saved(driver, root, 'saved-zero-restarted')
        require(retained['grain']['rms_granularity'] == [0, 0, 0], 'Startup rewrote saved zero RMS')
        require(retained['input_image']['upscale_factor'] == 1.5, 'Startup rewrote saved upscale')
        driver.startup_action('Restore factory default')
        restored_path = root/'factory-restored.json'
        restored = driver.state_action('Save current to file', restored_path, save=True)
        for section, expected in factory.items():
            if section != 'rust':
                require(parameters_equal(restored.get(section), expected),
                        f'Factory restore changed {section}')
        report['restart_state_sha256'] = sha(restart_path)
        report['restored_factory_sha256'] = sha(restored_path)
        driver.state_action('Load from file', seed_path)
        require(sha(args.raw) == '37e290dbd0053f00e508d02a6b3a2a990432dad1eb74c40a52ca899f0f225ecc', 'Unexpected RAW fixture')
        driver.open_image(args.raw.resolve(), 'imported-raw')
        raw_text = driver.wait_text(
            r'(?:Loaded[^\n]*768\s*[x×=*]\s*512|(?:Preview|Scan)[^\n]*(?:640\s*[x×=*]\s*426|768\s*[x×=*]\s*512))',
            'raw-import-dimensions')
        driver.records.append({'raw_fixture_sha256': sha(args.raw),
                               'raw_input_size': [768, 512], 'observed_status': raw_text})
        driver.reveal('extras.load_raw.white_balance', 'Input and capture', 'Import Raw')
        driver.choose('white balance', 'tungsten')
        driver.wait_text(r'RAW decode settings pending', 'raw-settings-pending')
        driver.click('Reprocess RAW', right=True)
        driver.wait_text(r'Loaded[^\n]*768\s*[x×=*]\s*512', 'raw-reprocessed')
        driver.wait_text(r'RAW decode settings match', 'raw-settings-applied')
        raw_state = saved(driver, root, 'raw-import-state')
        require(raw_state['input_image']['input_color_space'] == 'ACES2065-1', 'RAW input space differs')
        require(raw_state['input_image']['input_cctf_decoding'] is False, 'RAW decoding flag differs')
        require(raw_state['load_raw']['white_balance'] == 'tungsten', 'RAW reprocess selection lost')
        driver.click('SCAN', bottom=True)
        driver.rendered('raw-reprocessed-scan', action='Scan')
        raw_output = root/'raw-reprocessed.tif'
        driver.click('SAVE', bottom=True)
        driver.dialog(raw_output, save=True)
        wait_for(raw_output.is_file, 'reprocessed RAW saved pixels', 30)
        report['raw_reprocessed_output'] = read_output(raw_output, size=(768, 512))
        driver.state_action('Load from file', seed_path)
        driver.open_image(source, 'export-input-reloaded')
        export_controls(driver)
        export_path = root/'cpu-export.tif'
        export_image(driver, export_path)
        report['cpu_export'] = read_output(export_path)
        large = root/'large.tif'
        pixels = np.tile(np.linspace(.05, .8, 6144, dtype=np.float32)[None, :, None], (4608, 1, 3))
        writer = oiio.ImageOutput.create(str(large))
        require(writer.open(str(large), oiio.ImageSpec(6144, 4608, 3, oiio.FLOAT)), 'Large fixture open failed')
        require(writer.write_image(pixels), 'Large fixture write failed')
        writer.close()
        cancellation_state = json.loads(json.dumps(seed))
        cancellation_state['simulation']['route'] = 'input > film > print > scan'
        cancellation_state['grain']['active'] = True
        cancellation_seed = root/'cancellation-seed.json'
        save_json(cancellation_seed, cancellation_state)
        driver.state_action('Load from file', cancellation_seed)
        cancel_export(driver, large, root/'cancelled.tif')
        if not args.development_smoke:
            require(git(repo, 'rev-parse', 'HEAD') == commit, 'Implementation changed during native run')
            require(not git(repo, 'status', '--porcelain', '--untracked-files=all'),
                    'Worktree changed during native run')
        report['status'] = 'development-smoke' if args.development_smoke else 'pass'
        driver.close()
    except Exception as error:
        report['status'] = 'fail'
        report['failure'] = str(error)
        raise
    finally:
        driver.finish()
        report['observations_sha256'] = sha(root/'observations.json')
        report['screenshots_sha256'] = {p.name: sha(p) for p in sorted(root.glob('*.png'))}
        save_json(root/'report.json', report)
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
