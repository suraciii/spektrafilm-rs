"""Changed-path native scenarios for the experimental GUI contract.

Uses the existing X11/OCR driver and real saved GUI state as observations.
"""
import json
import re
import time

from gui_acceptance import require, wait_for


def saved(driver, root, name):
    return driver.state_action('Save current to file', root / (name + '.json'), save=True)


def enter_number(driver, label, value):
    image, _, lines = driver.read()
    driver.snap('numeric-before-' + label.replace(' ', '-'), image, lines)
    matches = driver.match(lines, label, True)
    require(matches, f'Numeric label not visible: {label}')
    x, y = matches[0]
    candidates = [word for line in lines for word in line
                  if word[1] > x and abs(word[2] + word[4] / 2 - y) < 9
                  and re.fullmatch(r'[-+]?\d+(?:[.,]\d+)?', word[0].strip('[]()|'))]
    if not candidates:
        candidates = [word for line in lines for word in line
                      if word[1] >= image.width - 410 and 10 < word[2] - y < 42
                      and re.fullmatch(r'[-+]?\d+(?:[.,]\d+)?', word[0].strip('[]()|'))]
    require(candidates, f'Numeric editor not visible beside {label}')
    word = min(candidates, key=lambda word: word[1])
    driver.xd('mousemove', '--window', driver.window,
              int(word[1] + word[3] / 2), int(word[2] + word[4] / 2))
    driver.xd('click', 1)
    driver.xd('key', 'ctrl+a')
    driver.xd('type', '--clearmodifiers', '--', str(value))
    driver.xd('key', 'Return')
    time.sleep(.3)
    driver.snap('entered-' + label.replace(' ', '-') + '-' + str(value))


def control_boundaries(driver, root, factory):
    state = json.loads(json.dumps(factory))
    state['simulation']['auto_preview'] = False
    state['grain']['rms_granularity'] = [0, 0, 0]
    state['input_image']['upscale_factor'] = 1.5
    path = root / 'saved-user-values.json'
    path.write_text(json.dumps(state))
    driver.state_action('Load from file', path)
    actual = saved(driver, root, 'saved-user-values-roundtrip')
    require(actual['grain']['rms_granularity'] == [0, 0, 0], 'Saved zero RMS was rewritten')
    require(actual['input_image']['upscale_factor'] == 1.5, 'Saved upscale was rewritten')
    scenarios = [
        ('MAIN', 'Camera', 'Exposure compensation', -20, 'camera', 'exposure_compensation_ev', -20),
        ('MAIN', 'Camera', 'Film format', 1, 'camera', 'film_format_mm', 8),
        ('MAIN', 'Camera', 'Film format', 200, 'camera', 'film_format_mm', 120),
        ('FILM', 'Convert', 'Exposure compensation', -20, 'convert', 'exposure_compensation_ev', -20),
        ('FILM', 'Chemistry', 'Gamma factor', 0, 'film_chemistry', 'gamma_factor', .25),
        ('FILM', 'Chemistry', 'Developer exhaustion', 2, 'film_chemistry', 'developer_exhaustion', 1),
        ('PRINT', 'Glare', 'Percent', 2, 'glare', 'percent', 1),
        ('PRINT', 'Glare', 'Roughness', 2, 'glare', 'roughness', 1),
        ('FILM', 'Couplers', 'Langmuir donor', 0, 'couplers', 'langmuir_donor_k_rgb', [.1, 1, 1]),
    ]
    for index, (tab, section, label, entered, group, field, expected) in enumerate(scenarios):
        driver.tab(tab)
        driver.section(section)
        if not driver.locate(label):
            driver.scroll(True)
        enter_number(driver, label, entered)
        driver.scroll(False)
        driver.section(section)
        actual = saved(driver, root, f'boundary-{index}')
        observed = actual[group][field]
        if isinstance(expected, list):
            require(all(abs(a - b) < 1e-6 for a, b in zip(observed, expected)),
                    f'{group}.{field}: expected {expected}, got {observed}')
        else:
            require(abs(observed - expected) < 1e-6,
                    f'{group}.{field}: entered {entered}, expected {expected}, got {observed}')
        driver.records.append({'boundary': f'{group}.{field}', 'entered': entered,
                               'observed': observed})
    driver.tab('MAIN')
    driver.section('Crop and upscale')
    enter_number(driver, 'Upscale factor', 0)
    anchor = driver.locate('Upscale factor')
    driver.xd('mousemove', '--window', driver.window, int(anchor[0]), int(anchor[1]))
    time.sleep(1.2)
    driver.snap('upscale-tooltip')
    driver.section('Crop and upscale')
    actual = saved(driver, root, 'invalid-upscale-preserved')
    require(actual['input_image']['upscale_factor'] == 0, 'Invalid upscale silently substituted')
    driver.tab('CONFIG')
    driver.section('Display')
    positions = [driver.locate(label) for label in
                 ('use display transform', 'gray 18% canvas', 'white padding', 'preview max size', 'output interpolation')]
    require(all(positions), 'Display contract field missing')
    require([point[1] for point in positions] == sorted(point[1] for point in positions),
            'Display field order differs from upstream')
    driver.snap('display-field-order')
    driver.section('Display')
    driver.tab('ADVANCED')
    driver.section('Experimental')
    x, y = driver.locate('TH-KG3')
    driver.xd('mousemove', '--window', driver.window, int(x), int(y))
    driver.xd('click', 1)
    image, _, lines = driver.read()
    text = ' '.join(word[0] for line in lines for word in line)
    require(not any(value in text for value in ('D50', 'D55', 'D65')), 'Extra print illuminant choice')
    driver.snap('print-illuminant-options', image, lines)
    driver.xd('key', 'Escape')
    driver.section('Experimental')
    return actual


def export_controls(driver):
    driver.tab('MAIN')
    for section in ('Import RGB', 'Import Raw'):
        driver.section(section)
        image, _, lines = driver.read()
        text = '\n'.join(' '.join(word[0] for word in line) for line in lines)
        require('Export backend' not in text and 'Save bit depth' not in text,
                f'{section} contains duplicate export controls')
        driver.snap('unique-' + section.replace(' ', '-'), image, lines)
        driver.section(section)
    driver.section('Output')
    require(driver.locate('Export options'), 'Output lacks export extension')
    require(not driver.locate('Export backend'), 'Export extension initially expanded')
    driver.section('Export options')
    driver.scroll(True)
    driver.snap('opened-output-export-options')
    require(driver.locate('Export backend') and driver.locate('Save bit depth'),
            'Export extension lacks backend/depth')
    driver.snap('output-export-options')


def export_image(driver, exporter, path):
    driver.exporter_hash = __import__('hashlib').sha256(exporter.read_bytes()).hexdigest()
    driver.watch_export()
    driver.click('Export', exact=True)
    driver.dialog(path, save=True)
    driver.capture_export_child(exporter)
    driver.wait_text(r'Exported.*f64', 'cpu-export-completed', timeout=240)
    wait_for(path.is_file, 'CPU exported image', 30)
    driver.no_children()


def profile_roundtrip(driver, root):
    driver.tab('MAIN')
    driver.section('Profiles')
    def choose(label):
        image, _, lines = driver.read()
        matches = driver.match(lines, 'Film profile', True)
        require(matches, 'Film profile picker missing')
        x, y = matches[0]
        driver.xd('mousemove', '--window', driver.window, int(x + 50), int(y + 22))
        driver.xd('click', 1)
        for direction in ('4', '5'):
            for attempt in range(14):
                image, _, lines = driver.read()
                matches = driver.match(lines, label, True)
                if matches:
                    x, y = matches[-1]
                    driver.snap('profile-menu-' + label.replace(' ', '-'), image, lines)
                    driver.xd('mousemove', '--window', driver.window, int(x), int(y))
                    driver.xd('click', 1)
                    time.sleep(.4)
                    return
                driver.xd('mousemove', '--window', driver.window, image.width - 150, 400)
                driver.xd('click', '--repeat', '5', '--delay', '50', direction)
        raise RuntimeError(f'Profile option not visible: {label}')
    choose('Kodak Portra 400')
    changed = saved(driver, root, 'profile-other')
    require(changed['simulation']['film_stock'] == 'kodak_portra_400', 'Profile did not change')
    driver.tab('MAIN')
    driver.scroll(False)
    choose('Kodak Gold 200')
    restored = saved(driver, root, 'profile-gold')
    require(restored['simulation']['film_stock'] == 'kodak_gold_200', 'Gold 200 not selected')
    require(restored['grain']['rms_granularity'] == [5, 5, 5], 'Gold 200 preset RMS differs')
    driver.tab('MAIN')
    driver.section('Profiles')


def cancel_export(driver, exporter, source, destination):
    driver.tab('MAIN')
    driver.scroll(False)
    driver.section('Import RGB')
    driver.click('Select file')
    driver.dialog(source)
    driver.wait_text(r'Loaded', 'large-import')
    driver.section('Import RGB')
    driver.section('Output')
    driver.scroll(True)
    if not driver.locate('Export backend'):
        driver.section('Export options')
        driver.scroll(True)
    driver.watch_export()
    driver.click('Export', exact=True)
    driver.dialog(destination, save=True)
    driver.capture_export_child(exporter)
    driver.require_export_in_flight('Cancel')
    driver.click('Cancel')
    driver.wait_text(r'Export cancel(?:ed|led)', 'cancelled-export', timeout=30)
    driver.no_children()
    require(not destination.exists(), 'Cancelled export published output')
    driver.records.append({'cancel_output_absent': True})
