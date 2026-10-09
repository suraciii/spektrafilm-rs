"""Changed-path native scenarios for the experimental GUI contract.

Uses the existing X11/OCR driver and real saved GUI state as observations.
"""
import json
import re
import subprocess
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

def require_tooltip_text(text, expected):
    require(' '.join(expected.casefold().split()) in ' '.join(text.casefold().split()),
            f'Native tooltip missing: {expected}')


def upscale_tooltips(driver):
    expected = 'Scale image size up to increase resolution'
    for target in ('label', 'editor'):
        driver.xd('mousemove', '--window', driver.window, 100, 100)
        time.sleep(.6)
        _, bbox, lines = driver.read()
        matches = driver.match(lines, 'upscale factor', True)
        require(matches, 'Upscale label not visible')
        x, y = matches[0]
        if target == 'editor':
            candidates = [w for line in lines for w in line
                          if w[1] > x + 25 and abs(w[2] + w[4] / 2 - y) < 10
                          and re.fullmatch(r'[-+]?\d+(?:[.,]\d+)?', w[0].strip('[]()|'))]
            require(candidates, 'Upscale editor not visible')
            w = min(candidates, key=lambda w: w[1])
            x, y = w[1] + w[3] / 2, w[2] + w[4] / 2
        x, y = int(bbox['left'] + x), int(bbox['top'] + y)
        # egui 0.31 records movement time from nonzero sampled velocity.
        # Sparse cursor warps leave it unset and suppress post-click tooltips.
        # Bypass xd's 100 ms movement pause to deliver a continuous trajectory.
        for offset in range(20, -1, -1):
            subprocess.run(['xdotool', 'mousemove', str(x - offset), str(y)],
                           check=True, timeout=15)
            time.sleep(.015)
        time.sleep(1.2)
        text = driver.snap('upscale-tooltip-' + target)
        require_tooltip_text(text, expected)
        driver.records.append({'tooltip_target': target, 'tooltip_text': expected,
                               'text_observed': True,
                               'pointer': driver.xd('getmouselocation')})
    driver.xd('mousemove', '--window', driver.window, 100, 100)
    time.sleep(.6)



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
    upscale_tooltips(driver)
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
        require('Export backend' not in text and 'Save bit depth' not in text
                and not driver.locate('Export', exact=True)
                and not driver.locate('Cancel', exact=True),
                f'{section} contains duplicate export controls')
        require(not any(re.fullmatch(r'Save(?:\.{3}|…)?', word[0])
                        for line in lines for word in line if word[1] >= driver.right_control_x),
                f'{section} contains duplicate Save action')
        driver.snap('unique-' + section.replace(' ', '-'), image, lines)
        driver.section(section)
    driver.section('Output')
    require(driver.locate('Export options'), 'Output lacks export extension')
    require(not driver.locate('Export', exact=True), 'Export extension initially expanded')
    driver.section('Export options')
    driver.scroll(True)
    driver.snap('opened-output-export-options')
    require(driver.locate('Export', exact=True), 'Output lacks Export action')
    require(not driver.locate('Export backend') and not driver.locate('Save bit depth'),
            'Modal settings duplicated in Output')
    driver.snap('output-export-options')


def export_image(driver, path):
    driver.click('Export', exact=True)
    driver.export_options(path)
    driver.dialog(path, save=True)
    driver.exported(path, 'cpu-export-completed')


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
        time.sleep(.4)
        driver.snap('profile-popup-' + label.replace(' ', '-'))
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
                driver.xd('click', direction)
                time.sleep(.2)
        driver.snap('profile-search-failed-' + label.replace(' ', '-'))
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


def cancel_export(driver, source, destination):
    driver.tab('MAIN')
    driver.scroll(False)
    driver.section('Import RGB')
    driver.click('Select file')
    driver.dialog(source)
    driver.wait_text(r'Loaded', 'large-import')
    driver.section('Import RGB')
    driver.section('Output')
    driver.scroll(True)
    if not driver.locate('Export', exact=True):
        driver.section('Export options')
        driver.scroll(True)
    from PIL import ImageOps
    image, _, lines = driver.read()
    action = driver.locate('Export', exact=True)
    require(action, 'Export action missing before cancellation')
    button_word = min((word for line in lines for word in line),
                      key=lambda word: abs(word[1] + word[3] / 2 - action[0]) + abs(word[2] + word[4] / 2 - action[1]))
    left, top, _, height = button_word[1:]
    driver.click('Export', exact=True)
    driver.export_options(destination)
    driver.dialog(destination, save=True)
    def cancel_visible():
        image, _ = driver.image()
        interior = image.crop((int(left), int(top) - 1, int(left) + 43, int(top + height) + 1)).convert('L')
        text = driver.ocr.image_to_string(ImageOps.invert(interior).resize((430, interior.height * 10)), config='--psm 7').strip()
        if re.findall(r'[a-z]+', text.lower()) == ['cancel']:
            image.save(driver.root/'cancel-button-visible.png')
            driver.records.append({'cancel_button_text': text, 'screenshot': 'cancel-button-visible.png'})
            return True
    wait_for(cancel_visible, 'rendered Cancel button', 15)
    driver.require_export_in_flight('Cancel')
    driver.xd('mousemove', '--window', driver.window, int(left + 20), int(top + height / 2))
    driver.xd('click', 1)
    driver.wait_text(r'Cancelling export|Export cancel(?:ed|led)',
                     'cancel-request-accepted', timeout=15)
    driver.wait_text(r'Export cancel(?:ed|led)', 'cancelled-export', timeout=240)
    require(not destination.exists(), 'Cancelled export published output')
    driver.records.append({'cancel_output_absent': True})
