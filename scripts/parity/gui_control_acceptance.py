"""Native inspector scenarios observed through controls and saved GUI state.

Navigation uses the production canonical-path search and real editing owners;
no prototype tab/manifest is treated as evidence of a connected control.
"""
import json
import math
import re
import subprocess
import time

from gui_acceptance import require, wait_for


def saved(driver, root, name):
    return driver.state_action('Save current to file', root / (name + '.json'), save=True)


def imported_scan_illuminant(driver, root):
    """The real Convert owner persists acquisition light without scanner edits."""
    before = saved(driver, root, 'before-acquisition-light')
    driver.choose_route(5)
    scanner = saved(driver, root, 'acquisition-route')['rust']['runtime']['scanner']
    for light in ('D65', 'T'):
        driver.reveal('film_render.convert.scan_illuminant', 'Input and capture', 'Convert')
        driver.choose('scan illuminant', light)
        actual = saved(driver, root, 'acquisition-light-' + light)
        require(actual['convert']['scan_illuminant'] == light,
                f'Convert acquisition light did not persist: {light}')
        require(actual['rust']['runtime']['film_render']['convert']['scan_illuminant'] == light,
                f'Runtime acquisition light differs from Convert: {light}')
        require(actual['rust']['runtime']['scanner'] == scanner,
                'Acquisition light edited the final readout scanner')
        require('lamp' not in actual['scanner'] and 'lamp' not in scanner,
                'Retired scanner lamp leaked into current GUI state')
        path = root / ('acquisition-light-' + light + '.json')
        driver.state_action('Load from file', path)
        roundtrip = saved(driver, root, 'acquisition-light-roundtrip-' + light)
        require(roundtrip['convert']['scan_illuminant'] == light,
                f'Imported acquisition light was lost on state load: {light}')
        driver.records.append({'canonical_control': 'film_render.convert.scan_illuminant',
                               'owner': 'Input and capture', 'section': 'Convert',
                               'acquisition_light': light, 'scanner_unchanged': True,
                               'saved_state_roundtrip': True})
    restore = root / 'before-acquisition-light.json'
    driver.state_action('Load from file', restore)
    return before


def numeric_editor(driver, label):
    """Find the text value on a scalar row or the first tuple component row."""
    for _ in range(12):
        image, _, lines = driver.read()
        matches = driver.match([line for line in lines if line and line[0][1] >= image.width - 420], label)
        if matches:
            x, y = matches[0]
            candidates = [word for line in lines for word in line
                          if word[1] > x and abs(word[2] + word[4] / 2 - y) < 10
                          and re.fullmatch(r'[-+−]?\d+(?:[.,]\d+)?', word[0].strip('[]()|'))]
            if not candidates:
                candidates = [word for line in lines for word in line
                              if word[1] >= image.width - 410 and 8 < word[2] - y < 45
                              and re.fullmatch(r'[-+−]?\d+(?:[.,]\d+)?', word[0].strip('[]()|'))]
            if candidates:
                word = min(candidates, key=lambda word: (word[2], word[1]))
                return (word[1] + word[3] / 2, word[2] + word[4] / 2), (x, y)
        driver.xd('mousemove', '--window', driver.window, image.width - 12, image.height // 2)
        driver.xd('click', '--repeat', 3, '--delay', 50, 5)
        time.sleep(.2)
    text = driver.snap('numeric-missing-' + label.replace(' ', '-'))
    raise RuntimeError(f'Numeric editor not visible: {label}; observed OCR:\n{text}')


def enter_number(driver, label, value):
    point, _ = numeric_editor(driver, label)
    driver.snap('numeric-before-' + label.replace(' ', '-'))
    driver.xd('mousemove', '--window', driver.window, int(point[0]), int(point[1]))
    driver.xd('click', 1)
    driver.xd('key', 'ctrl+a')
    driver.xd('type', '--clearmodifiers', '--', str(value))
    driver.xd('key', 'Return')
    # Move focus away so rejected text cannot be mistaken for accepted state.
    driver.xd('key', 'Tab')
    time.sleep(.3)
    driver.snap('entered-' + label.replace(' ', '-') + '-' + str(value))



def scanner_readout(driver, root):
    """Observe current fixed-readout polarity, operator and optics through UI."""
    from gui_acceptance import read_image
    import numpy as np

    baseline_path = root / 'scanner-baseline.json'
    baseline = driver.state_action('Save current to file', baseline_path, save=True)
    probe = json.loads(json.dumps(baseline))
    probe['simulation']['route'] = 'input > film > scan'
    probe['grain']['active'] = False
    probe['scanner'].update(scan_output='direct_scan', conversion_style='standard',
                            lens_blur=0, unsharp_mask=[0, 0],
                            operator=dict(exposure_ev=0, warmth_ev=0, tint_ev=0, contrast=1))
    probe_path = root / 'scanner-probe.json'
    probe_path.write_text(json.dumps(probe))
    driver.state_action('Load from file', probe_path)
    reports = []

    def scan_save(name):
        driver.close_settings()
        driver.click('SCAN', bottom=True)
        reports.append(driver.wait_report(name, 'Scan'))
        path = root / (name + '.exr')
        driver.click('SAVE', bottom=True)
        driver.dialog(path, save=True)
        driver.wait_report(name + '-saved', 'Save')
        wait_for(path.is_file, name + ' image', 30)
        pixels = read_image(path)
        require(pixels.shape == (96, 192, 3) and np.isfinite(pixels).all(),
                f'Scanner Scan/Save lost full finite source dimensions: {pixels.shape}')
        return pixels

    direct = scan_save('scanner-preserve-polarity')
    driver.reveal('scanner.scan_output', 'Scan', 'Scanner')
    driver.choose('scan output', 'Negative to positive')
    positive = scan_save('scanner-standard-positive')
    require(not np.array_equal(direct, positive),
            'Changing scan polarity did not change full-resolution pixels')
    driver.reveal('scanner.operator.exposure_ev', 'Scan', 'Scanner')
    enter_number(driver, 'Finished exposure', '.75')
    corrected = scan_save('scanner-operator-corrected')
    require(not np.array_equal(positive, corrected),
            'Finished-positive operator correction did not change Scan pixels')
    driver.reveal('scanner.lens_blur', 'Scan', 'Scanner')
    enter_number(driver, 'Lens blur (readout optics)', '2')
    blurred = scan_save('scanner-readout-optics')
    require(not np.array_equal(corrected, blurred),
            'Readout optics edit did not change full-resolution Scan pixels')
    edited_path = root / 'scanner-edited.json'
    edited = driver.state_action('Save current to file', edited_path, save=True)
    require('lamp' not in edited['scanner'], 'Retired scanner lamp leaked into saved state')
    require(edited['scanner']['conversion_style'] == 'standard',
            'Current standard conversion was not retained')
    require(edited['scanner']['scan_output'] == 'negative_to_positive',
            'Scanner polarity selection was not accepted')
    require(math.isclose(edited['scanner']['operator']['exposure_ev'], .75, abs_tol=1e-6),
            'Finished exposure editor did not accept .75 EV')
    require(math.isclose(edited['scanner']['lens_blur'], 2, abs_tol=1e-6),
            'Readout optics editor did not accept sigma 2')
    driver.reveal('action.operator_restore_automatic', 'Scan', 'Scanner')
    driver.click('Restore automatic', right=True)
    automatic = saved(driver, root, 'scanner-automatic-restored')
    require(automatic['scanner']['operator'] == probe['scanner']['operator'],
            'Restore automatic did not reset only the operator values')
    require(automatic['scanner']['lens_blur'] == edited['scanner']['lens_blur'],
            'Restore automatic changed readout optics')
    driver.state_action('Load from file', edited_path)
    restored = saved(driver, root, 'scanner-restored')
    require(restored['scanner'] == edited['scanner'], 'Scanner state did not round-trip')
    restored_pixels = scan_save('scanner-restored-optics')
    require(np.array_equal(restored_pixels, blurred),
            'Loading scanner state did not restore full-resolution pixels')
    driver.records.append({
        'scenario': 'scanner_readout',
        'assertion': 'fixed-readout polarity, operator and optics change Scan pixels; state restores pixels',
        'scanner_controls': edited['scanner'], 'reports': reports,
        'retired_controls_absent': ['scanner.lamp'],
        'polarity_max_abs_change': float(np.max(np.abs(direct - positive))),
        'operator_max_abs_change': float(np.max(np.abs(positive - corrected))),
        'optics_max_abs_change': float(np.max(np.abs(corrected - blurred))),
        'restored_pixels_equal': True,
    })
    driver.state_action('Load from file', baseline_path)


def require_tooltip_text(text, expected):
    require(' '.join(expected.casefold().split()) in ' '.join(text.casefold().split()),
            f'Native tooltip missing: {expected}')


def upscale_tooltips(driver):
    expected = 'Scale image size up to increase resolution'
    for target in ('label', 'editor'):
        driver.xd('mousemove', '--window', driver.window, 100, 100)
        time.sleep(.6)
        editor, label = numeric_editor(driver, 'upscale factor')
        _, bbox, _ = driver.read()
        x, y = editor if target == 'editor' else label
        x, y = int(bbox['left'] + x), int(bbox['top'] + y)
        # egui needs a sampled movement trajectory to start its hover timer.
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


def control_boundaries(driver, root, factory):
    state = json.loads(json.dumps(factory))
    state['grain']['rms_granularity'] = [0, 0, 0]
    state['input_image']['upscale_factor'] = 1.5
    path = root / 'saved-user-values.json'
    path.write_text(json.dumps(state))
    driver.state_action('Load from file', path)
    actual = saved(driver, root, 'saved-user-values-roundtrip')
    require(actual['grain']['rms_granularity'] == [0, 0, 0], 'Saved zero RMS was rewritten')
    require(actual['input_image']['upscale_factor'] == 1.5, 'Saved upscale was rewritten')
    # Valid text commits, usable-range rejection, and invalid text are observed
    # from saved canonical values, not the text left inside an editor.
    scenarios = [
        ('camera.exposure_compensation_ev', 'Input and capture', 'Camera', 'exposure compensation ev',
         -10, -20, 'camera', 'exposure_compensation_ev', -10),
        ('camera.film_format_mm', 'Input and capture', 'Camera', 'Frame Long Edge',
         1, 0, 'camera', 'film_format_mm', 1),
        ('camera.film_format_mm', 'Input and capture', 'Camera', 'Frame Long Edge',
         1000, 2000, 'camera', 'film_format_mm', 1000),
        ('film_render.convert.exposure_compensation_ev', 'Input and capture', 'Convert', 'exposure compensation ev',
         -10, -20, 'convert', 'exposure_compensation_ev', -10),
        ('film_render.chemistry.gamma_factor', 'Film', 'Film chemistry', 'gamma factor',
         .5, 0, 'film_chemistry', 'gamma_factor', .5),
        ('film_render.chemistry.developer_exhaustion', 'Film', 'Film chemistry', 'developer exhaustion',
         .375, 2, 'film_chemistry', 'developer_exhaustion', .375),
        ('print_render.glare.percent', 'Scan', 'Glare', 'percent',
         .25, 2, 'glare', 'percent', .25),
        ('print_render.glare.roughness', 'Scan', 'Glare', 'roughness',
         .5, 2, 'glare', 'roughness', .5),
        ('film_render.dir_couplers.langmuir_donor_k_rgb', 'Film', 'Couplers', 'langmuir donor k rgb',
         .2, 0, 'couplers', 'langmuir_donor_k_rgb', [.2, 1, 1]),
        ('io.upscale_factor', 'Input and capture', 'Crop and upscale', 'upscale factor',
         1.5, 0, 'input_image', 'upscale_factor', 1.5),
    ]
    for index, (canonical, owner, section, label, valid, invalid, group, field, expected) in enumerate(scenarios):
        # The real owners may disable fields outside their physical route.
        driver.choose_route(3 if section == 'Convert' else 2)
        for attempt, entered in (('valid', valid), ('outside-range', invalid), ('invalid-text', 'not-a-number')):
            driver.reveal(canonical, owner, section)
            if canonical == 'camera.film_format_mm':
                driver.choose('Film Format', 'Custom')
            enter_number(driver, label, entered)
            actual = saved(driver, root, f'boundary-{index}-{attempt}')
            observed = actual[group][field]
            values = zip(observed, expected) if isinstance(expected, list) else [(observed, expected)]
            if isinstance(expected, list):
                require(len(observed) == len(expected), f'{canonical}: component count changed')
            require(all(math.isclose(a, b, rel_tol=1e-6, abs_tol=1e-6) for a, b in values),
                    f'{canonical}: entered {entered}, expected {expected}, got {observed}')
            driver.records.append({'boundary': canonical, 'attempt': attempt,
                                   'entered': entered, 'observed': observed})
    driver.reveal('io.upscale_factor', 'Input and capture', 'Crop and upscale')
    upscale_tooltips(driver)
    driver.reveal('extras.display.use_display_transform', 'Settings', 'Display')
    require(driver.locate('use display transform'), 'Display settings editor missing')
    driver.snap('settings-display-owner')
    driver.close_settings()
    return actual


def export_controls(driver):
    # Output owns retained Save conversion; independent export lives in its
    # header-opened dialog. Its draft must not mutate committed preferences.
    before = saved(driver, driver.root, 'before-export-draft')
    driver.click('Export', header=True)
    for label in ('Backend', 'Format', 'Bit depth', 'Saving color space', 'Export file', 'Cancel'):
        require(driver.locate(label), f'Export dialog lacks {label}')
    driver.snap('header-export-options')
    driver.choose('Format', 'PNG')
    driver.click('Cancel')
    after = saved(driver, driver.root, 'after-cancel-export-draft')
    require(before['rust']['export_format'] == after['rust']['export_format'],
            'Cancelled Export draft changed committed format')
    require(before['rust']['save_bit_depth'] == after['rust']['save_bit_depth'],
            'Cancelled Export draft changed committed depth')


def export_image(driver, path):
    driver.close_settings()
    driver.click('Export', header=True)
    driver.export_options(path)
    driver.dialog(path, save=True)
    driver.exported(path, 'independent-export-completed')
    report = driver.last_export_report
    state = saved(driver, driver.root, 'independent-export-state')
    expected = 'gpu' if driver.export_backend_requested == 'wgpu' else 'cpu'
    require(state['rust']['export_backend'] == expected,
            f'Independent export did not retain its selected backend: {expected}')
    driver.records.append({'independent_export_operation': report['operation'],
                           'export_backend': expected})


def profile_roundtrip(driver, root):
    for name, identity in (('Kodak Portra 400', 'kodak_portra_400'),
                           ('Kodak Gold 200', 'kodak_gold_200')):
        driver.reveal('extras.simulation.film_stock', 'Film', 'Film material')
        driver.choose('film profile', name, below=True)
        actual = saved(driver, root, 'profile-' + identity)
        require(actual['simulation']['film_stock'] == identity, f'Profile did not select {identity}')


def cancel_export(driver, source, destination):
    driver.open_image(source, 'large-import')
    driver.click('Export', header=True)
    # Cancellation does not depend on a container/depth edit. Submit the actual
    # committed draft unchanged, so setup cannot outlive a short export.
    driver.click('Export file')
    driver.dialog(destination, save=True)
    def cancel_point():
        image, _, lines = driver.read()
        rows = [line for line in lines if line and line[0][2] <= 65]
        points = driver.match(rows, 'Cancel export')
        if points:
            driver.require_export_in_flight('Cancel export', (image, lines))
            return points[0]
    point = wait_for(cancel_point, 'in-flight header Cancel export action', 15)
    driver.point_click(point)
    driver.wait_text(r'Cancelling export|Export cancel(?:ed|led)',
                     'cancel-request-accepted', timeout=15)
    driver.wait_text(r'Export cancel(?:ed|led)', 'cancelled-export', timeout=240)
    report = driver.wait_report('cancelled-export-report', 'Export', outcome='cancelled')
    driver.records.append({'cancelled_export_operation': report['operation']})
    require(not list(destination.parent.glob(destination.stem + '.*')),
            'Cancelled export published an output (including normalized format extension)')
    driver.records.append({'cancel_output_absent': True})
