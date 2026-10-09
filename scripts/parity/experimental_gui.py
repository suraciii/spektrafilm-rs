#!/usr/bin/env python3
"""Experimental 28bf native GUI gate through X11, OCR and real file choosers.

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

from gui_acceptance import X11, require, wait_for, render_count
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
TOPOLOGY = {
    'MAIN': ['Import RGB', 'Import Raw', 'Crop and upscale', 'Input', 'Camera',
             'Profiles', 'Enlarger', 'Scanner', 'Output'],
    'FILM': ['Chemistry', 'Base', 'Halation', 'Couplers', 'Grain', 'Diffusion', 'Convert'],
    'PRINT': ['Chemistry', 'Base', 'Preflash', 'Glare', 'Diffusion'],
    'ADVANCED': ['Spectral upsampling', 'Input gamut compress',
                 'Output gamut compress', 'Experimental'],
    'CONFIG': ['GUI parameters', 'Display', 'napari layers'],
}
FORBIDDEN = ['Advanced halation', 'Advanced DIR couplers', 'Layered grain details',
             'Camera UV / IR filters', 'Scan film', 'Enlarger details',
             'Saving color', 'Preview workflow', 'Spectral adaptation']


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def git(repo, *args):
    return subprocess.run(['git', *args], cwd=repo, check=True,
                          capture_output=True, text=True).stdout.strip()


def words(text):
    return re.findall(r'[a-z0-9]+', text.lower())


def has_field(lines, label, sidebar_x):
    expected = ''.join(words(label))
    return any(line and line[0][1] >= sidebar_x
               and expected in ''.join(words(' '.join(w[0] for w in line)))
               for line in lines)


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def parameter_state(value):
    result = json.loads(json.dumps(value))
    # Opening a chooser intentionally updates its remembered directory.
    # All runtime, canonical GUI, and other extension fields must still match.
    result.get('rust', {}).pop('dialog_dirs', None)
    return result


class ExperimentalDesktop(X11):
    """Reuse native capture/chooser plumbing, without the old GUI coordinates."""

    def start(self, gui, env, image=None):
        self.chooser_ready(env)
        self.proc = subprocess.Popen([str(gui)] + ([str(image)] if image else []),
                                    cwd=self.root, env=env, stdout=self.log, stderr=self.log)
        def window():
            require(self.proc.poll() is None, 'Native GUI exited; inspect gui.log')
            ids = self.xd('search', '--onlyvisible', '--pid', self.proc.pid,
                          '--name', '^spektrafilm$', check=False)
            return ids.splitlines()[-1] if ids else None
        self.window = wait_for(window, 'native experimental window', 45)
        self.xd('windowactivate', '--sync', self.window)
        self.xd('windowsize', '--sync', self.window, 1460, 980)
        self.xd('windowmove', '--sync', self.window, 0, 0)
        wait_for(lambda: self.locate('MAIN'), 'rendered experimental tabs', 120)
        self.tab('MAIN')
        if self.locate('exposure compensation ev'):
            self.section('Camera')
        if self.locate('print auto compensation'):
            self.section('Enlarger')
        if self.locate('saving color space'):
            self.section('Output')

    def locate(self, label, *, exact=False, bottom=False):
        image, _, lines = self.read()
        lines = [[(word[0].replace('E&xport', 'Export'), *word[1:]) for word in line] for line in lines]
        matches = []
        for line in lines:
            if not line or line[0][1] < image.width - 420:
                continue
            if bottom and line[0][2] < image.height - 150:
                continue
            tokens = words(' '.join(w[0] for w in line))
            expected = words(label)
            if bottom and label in ('PREVIEW', 'SCAN', 'SAVE'):
                actions = [word for word in line if word[0].strip('[]()|') == label]
                if actions:
                    word = actions[0]
                    return (word[1] + word[3] / 2, word[2] + word[4] / 2)
                continue
            if exact:
                if len(tokens) == len(expected) + 1 and (line[0][1] < image.width - 400 or tokens[0] in ('v', 'y', 'vy')):
                    tokens = tokens[1:]
                if tokens != expected:
                    continue
                matches.append((line[0][1] + line[0][3] / 2,
                                line[0][2] + line[0][4] / 2))
            else:
                matches.extend(self.match([line], label, True))
        return matches[0] if matches else None

    def click(self, label, right=True, *, exact=False, bottom=False):
        if bottom:
            self.xd('mousemove', '--window', self.window, 100, 100)
        x, y = wait_for(lambda: self.locate(label, exact=exact, bottom=bottom),
                        f'visible control {label}', 25)
        self.snap('click-' + '-'.join(words(label)))
        self.xd('mousemove', '--window', self.window, int(x), int(y))
        self.xd('click', 1)
        time.sleep(.35)

    def tab(self, name):
        def target():
            image, _ = self.image()
            left = image.width - 420
            crop = image.crop((left, 25, image.width, 55))
            crop = crop.convert('L').point(lambda value: 0 if value > 125 else 255)
            data = self.ocr.image_to_data(crop.resize((crop.width * 4, crop.height * 4)),
                                          config='--psm 7', output_type=self.ocr.Output.DICT)
            for i, text in enumerate(data['text']):
                if re.sub(r'[^A-Z]', '', text.upper()) == name:
                    return left + (data['left'][i] + data['width'][i] / 2) / 4, 25 + (data['top'][i] + data['height'][i] / 2) / 4
        x, y = wait_for(target, 'native tab ' + name, 25)
        self.xd('mousemove', '--window', self.window, int(x), int(y))
        self.xd('click', 1)
        time.sleep(.35)
        self.current_tab = name
        self.scroll(False)

    def section(self, name):
        self.click(name, exact=True)
        time.sleep(.4)

    def state_action(self, label, path, save=False):
        # Re-enter MAIN before CONFIG after a restart; the saved default may
        # leave the native tab bar focused on a different section.
        self.tab('MAIN')
        self.tab('CONFIG')
        if not self.locate(label):
            self.section('GUI parameters')
        self.click(label)
        self.dialog(path, save)
        if save:
            wait_for(path.is_file, 'native saved state', 20)
            return json.loads(path.read_text())

    def choose_route(self, index):
        # The label belongs to the actual egui combobox. The menu is clicked by
        # its observed rows; the resulting saved canonical state proves selection.
        wait_for(lambda: self.locate('Workflow', bottom=True),
                 'fixed Workflow selector', 20)
        anchor = wait_for(lambda: self.locate('input', bottom=True),
                          'selected Workflow route', 20)
        self.xd('mousemove', '--window', self.window, int(anchor[0]), int(anchor[1]))
        self.xd('click', 1)
        time.sleep(.3)
        image, _, lines = self.read()
        rows = []
        for line in lines:
            text = ' '.join(word[0] for word in line)
            if line[0][1] >= image.width - 420 and re.match(r'^input\b', text, re.I):
                rows.append((line[0][2], line))
        rows.sort(key=lambda item: item[0])
        # Match the requested route exactly; OCR may omit the highlighted current
        # row. Saved canonical state independently verifies the chosen route.
        matching = [line for _, line in rows
                    if words(' '.join(w[0] for w in line)) == words(ROUTES[index])]
        require(len(matching) == 1, f'Cannot identify Workflow option {ROUTES[index]}: {rows}')
        self.snap('workflow-options', image, lines)
        line = matching[0]
        self.xd('mousemove', '--window', self.window,
                int(line[0][1] + 15), int(line[0][2] + line[0][4] / 2))
        self.xd('click', 1)
        time.sleep(.4)

    def topology(self):
        for tab, expected in TOPOLOGY.items():
            self.tab(tab)
            image, _, lines = self.read()
            self.snap('topology-observed-' + tab, image, lines)
            sidebar = [line for line in lines if line and line[0][1] >= image.width - 420]
            seen = []
            for label in expected:
                matches = [line for line in sidebar
                           if words(' '.join(w[0] for w in line)) in
                           (words(label), ['v'] + words(label), ['y'] + words(label), ['vy'] + words(label))]
                require(len(matches) == 1, f'{tab}: expected one section {label}, saw {len(matches)}')
                seen.append(matches[0][0][2])
            require(seen == sorted(seen), f'{tab}: section order mismatch {seen}')
            text = '\n'.join(' '.join(w[0] for w in line) for line in sidebar)
            for label in FORBIDDEN:
                require(not any(words(' '.join(w[0] for w in line)) == words(label)
                                for line in sidebar), f'{tab}: obsolete section {label}')
            for label in ['Auto preview', 'black and white correction', 'Workflow', 'PREVIEW', 'SCAN', 'SAVE']:
                require(self.locate(label, bottom=True), f'{tab}: fixed action {label} missing')
            self.snap('topology-' + tab, image, lines)
            self.records.append({'topology': tab, 'sections': expected, 'observed_y': seen,
                                 'ocr': text})

    def fields(self):
        metadata = json.loads((Path(__file__).parent/'fixtures/gui_28bf883/control_metadata.json').read_text())
        for group in metadata['groups']:
            path = group['group_path']
            if group['title'] in ('Camera', 'Scanner'):
                tab = 'MAIN'
            elif group['title'].endswith('gamut compress'):
                tab = 'ADVANCED'
            elif path == 'enlarger' or path.startswith(('print_render.', 'enlarger.')):
                tab = 'PRINT'
            else:
                tab = 'FILM'
            self.tab(tab)
            self.section(group['title'])
            nested = {name for sub in group['subsections'] for name in sub['field_names']}
            labels = [field['label'] for field in group['fields']
                      if field['leaf'] in group['panel_fields'] and field['leaf'] not in nested]
            observed = []
            for position in range(4):
                image, _, lines = self.read()
                observed.extend(lines)
                self.snap('fields-' + path + '-' + str(position), image, lines)
                self.xd('mousemove', '--window', self.window, image.width - 20, 600)
                self.xd('click', '--repeat', '5', '--delay', '50', '5')
            for label in labels:
                require(has_field(observed, label, self.right_control_x), f'{path}: missing {label}')
            self.scroll(False)
            for subgroup in group['subsections']:
                self.section(subgroup['title'])
                image, _, lines = self.read()
                for field in group['fields']:
                    if field['leaf'] in subgroup['field_names']:
                        require(has_field(lines, field['label'], self.right_control_x),
                                f'{path}/{subgroup["title"]}: missing {field["label"]}')
                self.snap('fields-' + subgroup['title'], image, lines)
                self.section(subgroup['title'])
            self.section(group['title'])


def fixture(path):
    y, x = np.mgrid[:24, :32]
    rgb = np.stack((.06 + .65*x/31, .09 + .55*y/23,
                    .12 + .5*(x+32*y)/(32*24-1)), axis=-1).astype('float32')
    writer = oiio.ImageOutput.create(str(path))
    require(writer and writer.open(str(path), oiio.ImageSpec(32, 24, 3, oiio.FLOAT)),
            'Cannot create float RGB fixture')
    require(writer.write_image(rgb), 'Cannot write RGB fixture')
    writer.close()


def read_output(path):
    image = oiio.ImageInput.open(str(path))
    require(image, f'Cannot read saved render {path}')
    spec = image.spec()
    pixels = np.asarray(image.read_image(oiio.FLOAT))
    image.close()
    require(np.isfinite(pixels).all(), f'Nonfinite render {path}')
    require((spec.width, spec.height, spec.nchannels) == (32, 24, 3),
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
    require(sha(oracle_dir/'control_metadata.json') == provenance['artifact_sha256']['control_metadata.json'],
            'Control metadata reference hash differs')
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
        driver.tab('MAIN')
        # Saving state opened CONFIG; return to pristine collapsed topology.
        driver.topology()
        driver.fields()
        control_boundaries(driver, root, factory)
        profile_roundtrip(driver, root)
        driver.state_action('Load from file', seed_path)
        driver.tab('MAIN')
        driver.section('Import RGB')
        driver.click('Select file')
        driver.dialog(source)
        driver.wait_text(r'Loaded|input\.tif', 'imported-rgb')
        driver.section('Import RGB')
        driver.state_action('Load from file', root/'invalid-upscale-preserved.json')
        driver.click('PREVIEW', bottom=True)
        def invalid_upscale_error():
            image, _, lines = driver.read()
            text = ' '.join(' '.join(w[0] for w in line) for line in lines)
            if 'upscale' in text.lower() and re.search(r'positive|greater|>\s*0', text, re.I):
                driver.snap('invalid-upscale-error', image, lines)
                return text
        report['invalid_upscale_error'] = wait_for(invalid_upscale_error, 'actionable upscale error', 30)
        driver.state_action('Load from file', seed_path)
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
        driver.click('Save current as default')
        saved_default = json.loads((config/'gui_default_state.json').read_text())
        driver.close()
        driver.start(gui, env)
        restart_path = root/'restarted.json'
        restarted = driver.state_action('Save current to file', restart_path, save=True)
        require(parameter_state(restarted) == parameter_state(saved_default),
                'Startup default lost canonical parameters')
        driver.state_action('Load from file', root/'saved-user-values.json')
        driver.click('Save current as default')
        driver.close()
        driver.start(gui, env)
        retained = saved(driver, root, 'saved-zero-restarted')
        require(retained['grain']['rms_granularity'] == [0, 0, 0], 'Startup rewrote saved zero RMS')
        require(retained['input_image']['upscale_factor'] == 1.5, 'Startup rewrote saved upscale')
        driver.click('Restore factory default')
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
        driver.tab('MAIN')
        driver.section('Import Raw')
        driver.click('Select file')
        driver.dialog(args.raw.resolve())
        raw_text = driver.wait_text(
            r'(?:Loaded[^\n]*768\s*[x×=*]\s*512|(?:Preview|Scan)[^\n]*(?:640\s*[x×=*]\s*426|768\s*[x×=*]\s*512))',
            'raw-import-dimensions')
        driver.records.append({'raw_fixture_sha256': sha(args.raw),
                               'raw_input_size': [768, 512], 'observed_status': raw_text})
        driver.section('Import Raw')
        raw_state = saved(driver, root, 'raw-import-state')
        require(raw_state['input_image']['input_color_space'] == 'ACES2065-1', 'RAW input space differs')
        require(raw_state['input_image']['input_cctf_decoding'] is False, 'RAW decoding flag differs')
        driver.state_action('Load from file', seed_path)
        driver.tab('MAIN')
        driver.section('Import RGB')
        driver.click('Select file')
        driver.dialog(source)
        driver.wait_text(r'Loaded|input\.tif', 'export-input-reloaded')
        driver.section('Import RGB')
        export_controls(driver)
        export_path = root/'cpu-export.tif'
        export_image(driver, export_path)
        report['cpu_export'] = read_output(export_path)
        driver.scroll(False)
        driver.section('Output')
        large = root/'large.tif'
        pixels = np.tile(np.linspace(.05, .8, 6144, dtype=np.float32)[None, :, None], (4608, 1, 3))
        writer = oiio.ImageOutput.create(str(large))
        require(writer.open(str(large), oiio.ImageSpec(6144, 4608, 3, oiio.FLOAT)), 'Large fixture open failed')
        require(writer.write_image(pixels), 'Large fixture write failed')
        writer.close()
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
