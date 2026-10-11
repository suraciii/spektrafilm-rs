#!/usr/bin/env python3
"""Consumer-visible viewer acceptance scenarios for the native GUI."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import re
import sys
import time

def transition_evidence(before, frames, after, threshold=1.0):
    """Require two distinct blend frames, excluding both endpoint rasters.

    Flat RGB samples keep this criterion usable without desktop dependencies.
    Projection onto the endpoint difference rejects unrelated repaint noise.
    """
    before, after = tuple(before), tuple(after)
    _require(len(before) == len(after) and bool(before), 'Animation endpoints differ in size')
    direction = tuple(b - a for a, b in zip(before, after))
    energy = sum(d * d for d in direction)
    delta = sum(abs(d) for d in direction) / len(direction)
    intermediate = []
    for index, frame in enumerate(frames):
        frame = tuple(frame)
        _require(len(frame) == len(before), 'Animation frame differs in size')
        if not energy:
            continue
        alpha = sum((v - a) * d for v, a, d in zip(frame, before, direction)) / energy
        start_distance = sum(abs(v - a) for v, a in zip(frame, before)) / len(before)
        end_distance = sum(abs(v - b) for v, b in zip(frame, after)) / len(before)
        residual = sum(abs(v - (a + alpha * d))
                       for v, a, d in zip(frame, before, direction)) / len(before)
        if (0.05 < alpha < 0.95 and min(start_distance, end_distance) > threshold
                and residual <= max(threshold, delta * 0.15)):
            intermediate.append({'frame': index, 'alpha': alpha, 'residual': residual})
    distinct = any(abs(a['alpha'] - b['alpha']) >= 0.05
                   for a in intermediate for b in intermediate)
    return {'passed': delta > threshold and len(intermediate) >= 2 and distinct,
            'endpoint_mean_delta': delta, 'intermediate_frames': intermediate}

def reveal_evidence(before, frames, after, threshold=1.0):
    """Require distinct nonlinear reveal frames outside both endpoint rasters."""
    _require(bool(before) and len(before) == len(after), 'Reveal endpoints differ in size')
    intermediate = []
    distinct = []
    for index, frame in enumerate(frames):
        _require(len(frame) == len(before), 'Reveal frame differs in size')
        if min(_sample_distance(frame, before), _sample_distance(frame, after)) <= threshold:
            continue
        if all(_sample_distance(frame, other) > threshold for other in distinct):
            distinct.append(frame)
            intermediate.append(index)
    delta = _sample_distance(before, after)
    return {'passed': delta > threshold and len(distinct) >= 2,
            'endpoint_mean_delta': delta, 'intermediate_frames': intermediate}


def _sample_distance(left, right):
    _require(bool(left) and len(left) == len(right), 'Viewer samples differ in size')
    return sum(abs(a - b) for a, b in zip(left, right)) / len(left)


def _sample(image):
    return tuple(value for pixel in _viewer_crop(image).convert('RGB').resize((96, 64)).getdata()
                 for value in pixel)


from gui_acceptance import viewer_canvas_bounds, wait_for


def _require(value, message):
    if not value:
        raise RuntimeError(message)


def _wait_file(path: Path, description: str, timeout: float = 30.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.is_file():
            return
        time.sleep(0.15)
    raise RuntimeError(f"Timed out waiting for {description}: {path}")


def _state_path(root: Path, name: str, state: dict) -> Path:
    path = root / name
    path.write_text(json.dumps(state, indent=2) + "\n")
    return path


def _load_state(driver, path: Path, label: str):
    driver.state_action("Load from file", path)
    driver.rendered(label)


def _text(lines):
    return "\n".join(" ".join(word[0] for word in line) for line in lines)


def _viewer_status_text(image, lines):
    # The selected layer is in the viewer overlay below the layer tabs.
    # Use the native driver's word coordinates, independent of OCR line order.
    left, _, right, _ = viewer_canvas_bounds(image)
    content_top = 30 if sys.platform == 'darwin' else 0
    return _text([[word for word in line
                   if left <= word[1] + word[3] / 2 < right
                   and content_top + 35 <= word[2] + word[4] / 2 < content_top + 63]
                  for line in lines])


def _viewer_crop(image):
    return image.crop(viewer_canvas_bounds(image))


def _record_snap(driver, records, label):
    image, _, lines = driver.read()
    ocr = driver.snap(label, image, lines)
    screenshot = driver.records[-1]["screenshot"]
    records.append({"scenario": label, "screenshot": screenshot, "ocr": ocr})
    return image, lines


def _record_frame(driver, records, label):
    """Capture one raw desktop frame without OCR while an animation runs."""
    image, _ = driver.image()
    screenshot = f"{len(driver.records):02d}-{label}.png"
    image.save(driver.root / screenshot)
    driver.records.append({"surface": label, "screenshot": screenshot})
    records.append({"scenario": label, "screenshot": screenshot})
    return image


def _present_output(driver, records, label):
    """Clear setup animation through the native display-transform control.

    The colored Input sentinel proves that pending setup compositions have
    completed before selecting the direct Output reference.
    """
    driver.tab('CONFIG')
    _display_control(driver, 'use display transform')
    driver.reclick_last_control()
    driver.click('Input', False)
    driver.measure_viewer(label + '-input-sentinel')
    fingerprint = driver.viewer_fingerprint()
    driver.click('Output', False)
    image, _ = driver.await_viewer(fingerprint, label=label + '-output')
    driver.snap(label + '-output', image)
    records.append({'scenario': label, 'screenshot': driver.records[-1]['screenshot']})
    return _sample(image)


def _capture_to_target(driver, records, label, target, interval=0.025, timeout=120):
    """Capture through the new render report and independently prepared endpoint.

    Repeated intermediate frames never establish animation completion.
    """
    frames = []
    deadline = time.monotonic() + timeout
    report = None
    while time.monotonic() < deadline:
        image = _record_frame(driver, records, f'{label}-frame-{len(frames)}')
        frames.append(_sample(image))
        if report is None:
            for path in driver.report_directory.glob('operation-*.json'):
                try:
                    operation = json.loads(path.read_text())['operation']
                except (OSError, ValueError, KeyError):
                    continue
                if (operation['id'] > driver.last_report_id and operation['kind'] == 'preview'
                        and operation['outcome'] != 'superseded'):
                    _require(operation['outcome'] == 'succeeded',
                             f'Animation preview failed: {operation}')
                    report = operation
        if report is not None:
            distance = _sample_distance(frames[-1], target)
            if distance <= 1.0:
                return frames, target
        time.sleep(interval)
    raise RuntimeError(f'Timed out waiting for presented {label} preview ({timeout}s)')



def _display_line(driver, words, section=None):
    wanted = [word.lower() for word in words]
    for attempt in range(3):
        image, _, lines = driver.read()
        for line in lines:
            normalized = [re.sub(r"[^a-z0-9]", "", word[0].lower()) for word in line]
            if all(any(item.startswith(term) for item in normalized) for term in wanted):
                return image, line
        if attempt == 0:
            # Sidebar rows render below the diagnostics list, so read the
            # scrolled sidebar before giving up on the current view.
            driver.scroll(True)
        elif attempt == 1 and section:
            # Collapsed sections hide their controls; open the section.
            driver.click(section)
            driver.scroll(True)
    raise RuntimeError(f"Could not locate display control: {' '.join(words)}")


def _set_white_padding(driver, value="0.2"):
    # The white-padding control is a text field right of its label in the
    # CONFIG sidebar; its row shifts with the surrounding controls. Click
    # the field, replace the text and commit it with Return.
    image, line = _display_line(driver, ("white", "padd"), section="Display")
    _require(image.width >= 1460 and image.height >= 980,
             "White-padding probe requires the pinned 1460x980 window")
    numeric = [word for word in line if re.match(r"^\d", word[0])]
    _require(numeric, "White padding value not visible")
    field = numeric[0]
    driver.xd("mousemove", "--window", driver.window,
              int(field[1] + field[3] / 2), int(field[2] + field[4] / 2))
    driver.xd("click", 1)
    time.sleep(.3)
    driver.xd("key", "ctrl+a")
    driver.xd("type", "--clearmodifiers", value)
    driver.xd("key", "Return")
    time.sleep(1.0)
    return image


def _click_combo(driver, label):
    image, line = _display_line(driver, tuple(label.lower().split()), section="Profiles")
    x = 1200
    y = sum(word[2] + word[4] / 2 for word in line) / len(line) + 20
    driver.xd("mousemove", "--window", driver.window, int(x), int(y))
    driver.xd("click", 1)
    time.sleep(0.2)
    return image


def _select_profile(driver, label, option):
    _click_combo(driver, label)
    time.sleep(.5)
    driver.xd('mousemove', '--window', driver.window, 1200, 420)
    for _ in range(25):
        image, _, lines = driver.read()
        matches = driver.match(lines, option, True)
        if matches:
            x, y = matches[0]
            driver.snap('profile-option-' + option.replace(' ', '-'), image, lines)
            driver.xd('mousemove', '--window', driver.window, int(x), int(y))
            driver.xd('click', 1)
            time.sleep(.8)
            return
        driver.xd('click', 5)
        time.sleep(.25)
    driver.xd('click', 1)
    time.sleep(.8)
    return


def _hover_float(driver, pixels, records):
    import numpy as np
    before_view = driver.viewer_fingerprint()
    driver.click("Input", False)
    driver.click("100%", False)
    bounds = driver.measure_viewer('viewer-float-inspection', before_view,
                                   expected_size=(pixels.shape[1], pixels.shape[0]))
    x = bounds[0] + pixels.shape[1] // 2
    y = bounds[1] + pixels.shape[0] // 2
    driver.xd("mousemove", "--window", driver.window, x, y)

    def read_rgb():
        image, _, lines = driver.read()
        text = _text(lines)
        left, _, right, _ = viewer_canvas_bounds(image)
        status = image.crop((left + 15, 35, right, 63))
        status = status.resize((status.width * 4, status.height * 4))
        data = driver.ocr.image_to_data(status, config='--psm 7', output_type=driver.ocr.Output.DICT)
        text += '\n' + ' '.join(data['text'])
        match = re.search(r"Input\s*\(\s*(\d+)\s*,\s*(\d+)\s*\)\s*RGB\s+([-+0-9.]+)\s*,\s*([-+0-9.]+)\s*,\s*([-+0-9.]+)", text, re.IGNORECASE)
        if match:
            return image, text, np.asarray([float(value) for value in match.groups()[2:]]), (int(match[1]), int(match[2]))
        return None

    deadline = time.monotonic() + 20
    result = None
    while time.monotonic() < deadline:
        result = read_rgb()
        if result:
            break
        time.sleep(0.15)
    _require(result, "Float pixel inspection did not expose RGB values")
    image, text, actual, coordinate = result
    _require(0 <= coordinate[0] < pixels.shape[1] and 0 <= coordinate[1] < pixels.shape[0], 'Inspection coordinate outside input')
    expected = pixels[coordinate[1], coordinate[0]].astype(float)
    error = float(np.max(np.abs(actual - expected)))
    _require(error <= 2e-5, f"Float inspection mismatch: actual={actual}, expected={expected}")
    screenshot = driver.root / f"{len(driver.records):02d}-viewer-float-inspection.png"
    image.save(screenshot)
    record = {"scenario": "float_inspection", "action": "hover Input at 100%",
              "assertion": "OCR RGB equals the known fixture pixel within tolerance",
              "screenshot": screenshot.name,
              "source_coordinate": list(coordinate),
              "expected_rgb": expected.tolist(), "observed_rgb": actual.tolist(),
              "max_abs_error": error, "ocr": text}
    records.append(record)
    return record


def _display_control(driver, label):
    """Click a Display-section control, expanding and scrolling as needed."""
    _, _, lines = driver.read()
    if not driver.match(lines, label):
        driver.scroll(True)
        _, _, lines = driver.read()
    if not driver.match(lines, label):
        # The section is collapsed: open it and read its content at the bottom.
        driver.click("Display")
        driver.scroll(True)
    driver.click(label)


def _export(driver, root, name):
    path = root / name
    path.unlink(missing_ok=True)
    driver.file_action("Export", path, True)
    driver.exported(path, f"viewer-{name}-exported")
    return path


def _save(driver, root, name):
    path = root / name
    path.unlink(missing_ok=True)
    driver.file_action("Save", path, True)
    driver.scroll(True)
    driver.wait_text(r"Saved\s+" + re.escape(path.name), f"viewer-{name}-saved", timeout=30)
    _wait_file(path, f"viewer save {name}")
    import OpenImageIO as oiio
    reader = oiio.ImageInput.open(str(path))
    _require(reader, f"Viewer Save cannot be decoded: {path}")
    depth = str(reader.spec().format)
    reader.close()
    expected = {'.exr': 'half', '.tif': 'uint16', '.tiff': 'uint16',
                '.png': 'uint8', '.jpg': 'uint8', '.jpeg': 'uint8'}[path.suffix.lower()]
    _require(depth == expected, f"Save extension default differs: {path.name} has {depth}, expected {expected}")
    driver.records.append({'save_output': path.name, 'depth': depth})
    return path


def accept_viewer(driver, root, state, pixels, exporter):
    """Run required viewer scenarios and return assertion records."""
    global np
    import numpy as np
    root = Path(root)
    state = copy.deepcopy(state)
    source = root / "standard.tif"
    _require(source.is_file(), f"Viewer acceptance source is missing: {source}")
    records = []
    small_preview_state = copy.deepcopy(state)
    small_preview_state.setdefault("display", {})["preview_max_size"] = 128
    small_preview_state["display"].update({"viewer_layer": "input", "crossfade": False})
    small_preview_state.setdefault("rust", {}).setdefault("viewer", {}).setdefault("settings", {}).update({"viewer_layer": "input", "crossfade": False})
    small_preview_path = _state_path(root, "viewer-small-preview.json", small_preview_state)
    _load_state(driver, small_preview_path, "viewer-small-preview")
    driver.tab("CONFIG")
    driver.click("Input", False)
    small_input, _ = _record_snap(driver, records, "viewer-small-input-raster")
    large_preview_state = copy.deepcopy(small_preview_state)
    large_preview_state["display"]["preview_max_size"] = 256
    large_preview_path = _state_path(root, "viewer-large-preview.json", large_preview_state)
    _load_state(driver, large_preview_path, "viewer-large-preview")
    driver.tab("CONFIG")
    driver.click("Input", False)
    large_input, _ = _record_snap(driver, records, "viewer-large-input-raster")
    preview_delta = float(np.mean(np.abs(np.asarray(_viewer_crop(small_input), dtype=np.float32) -
                                         np.asarray(_viewer_crop(large_input), dtype=np.float32))))
    _require(preview_delta > 0.01, "Preview max size did not refresh Input raster")
    full_export = _export(driver, root, "viewer-preview-resolution-export.exr")
    from gui_acceptance import read_image
    _require(read_image(full_export).shape[:2] == np.asarray(pixels).shape[:2],
             "Input preview raster size reduced full-resolution Export")
    records.append({"parity_action": "refresh_preview_cache",
                    "scenario": "preview_size_isolation", "input_pixel_change": preview_delta,
                    "assertion": "Input preview refreshes; Export retains source dimensions"})
    _load_state(driver, _state_path(root, "viewer-reset-after-preview.json", state), "viewer-reset-after-preview")
    baseline = _state_path(root, "viewer-baseline.json", copy.deepcopy(state))

    driver.tab("CONFIG")
    driver.click("Paper back", False)
    def paper_back_ready():
        image, _, lines = driver.read()
        text = _text(lines)
        _require(not re.search(r'(?:Render|Load|Save|Export|Startup state|Preview state)\s*(?:error|failed)', text, re.I),
                 f'GUI failure on paper-back-selected: {text}')
        if re.search(r"\bPaper\s*Back\b", _viewer_status_text(image, lines), re.I):
            driver.snap("paper-back-selected", image, lines)
            return True

    wait_for(paper_back_ready, "paper-back-selected", timeout=20)
    image, lines = _record_snap(driver, records, "viewer-paper-back")
    paper_text = _viewer_status_text(image, lines)
    _require(re.search(r"\bPaper\s*Back\b", paper_text, re.I),
             "Paper back viewer status was not selected in the real window")
    paper = np.asarray(_viewer_crop(image), dtype=np.float32)
    left, top, right, bottom = viewer_canvas_bounds(image)
    center_x, center_y = (left + right) // 2, (top + bottom) // 2
    watermark_bounds = (center_x - 190, center_y - 150, center_x + 190, center_y + 150)
    watermark_region = image.crop(watermark_bounds)
    watermark_path = root / "viewer-paper-watermark-region.png"
    watermark_region.save(watermark_path)
    watermark = np.asarray(watermark_region, dtype=np.float32)
    records.append({"parity_action": "virtual_photo_paper",
                    "scenario": "paper_back", "action": "select Paper back",
                    "assertion": "real-window paper raster and watermark region are visible",
                    "paper_view_std": float(paper.std()),
                    "paper_view_screenshot": records[-1]["screenshot"],
                    "watermark_region": watermark_path.name,
                    "watermark_region_std": float(watermark.std()),
                    "watermark_region_bounds": list(watermark_bounds)})
    _require(float(paper.std()) > 1.0 and float(watermark.std()) > 0.1,
             "Paper back screenshot has no visible presentation raster/watermark region")

    driver.click("Output", False)
    fingerprint = driver.viewer_fingerprint()
    _display_control(driver, "spline36")
    _display_control(driver, "nearest")
    driver.await_viewer(fingerprint, label='interpolation nearest')
    nearest_image, _ = _record_snap(driver, records, "viewer-interpolation-nearest")
    fingerprint = driver.viewer_fingerprint()
    _display_control(driver, "nearest")
    _display_control(driver, "spline36")
    driver.await_viewer(fingerprint, label='interpolation spline36')
    spline_image, _ = _record_snap(driver, records, "viewer-interpolation-spline36")
    nearest = np.asarray(_viewer_crop(nearest_image), dtype=np.float32)
    spline = np.asarray(_viewer_crop(spline_image), dtype=np.float32)
    interpolation_delta = float(np.mean(np.abs(nearest - spline)))
    records.append({"parity_action": "set_output_interpolation_mode",
                    "scenario": "interpolation", "action": "select Output and switch nearest/spline36",
                    "assertion": "viewer pixels differ between nearest and spline36",
                    "nearest_screenshot": records[-2]["screenshot"],
                    "spline36_screenshot": records[-1]["screenshot"],
                    "mean_abs_pixel_change": interpolation_delta})
    _require(interpolation_delta > 0.01, "Nearest and spline36 did not produce different viewer pixels")
    fingerprint = driver.viewer_fingerprint()
    driver.click("Input", False)
    _display_control(driver, "spline36")
    _display_control(driver, "nearest")
    driver.await_viewer(fingerprint, label='input interpolation nearest')
    input_nearest_image, _ = _record_snap(driver, records, "viewer-input-nearest")
    _display_control(driver, "nearest")
    _display_control(driver, "spline36")
    input_spline_image, _ = _record_snap(driver, records, "viewer-input-spline36")
    input_interpolation_delta = float(np.mean(np.abs(
        np.asarray(_viewer_crop(input_nearest_image), dtype=np.float32) -
        np.asarray(_viewer_crop(input_spline_image), dtype=np.float32))))
    records.append({"scenario": "interpolation_isolation",
                    "action": "select Input and switch Output interpolation",
                    "assertion": "Input pixels are unchanged by Output interpolation",
                    "mean_abs_pixel_change": input_interpolation_delta})
    _require(input_interpolation_delta < 2.0,
             "Output interpolation changed Input viewer pixels")
    fingerprint = driver.viewer_fingerprint()
    _display_control(driver, "gray 18% canvas")
    driver.await_viewer(fingerprint, label='gray canvas toggled')
    gray_image, _ = _record_snap(driver, records, "viewer-gray-canvas-toggled")
    gray = np.asarray(_viewer_crop(gray_image), dtype=np.float32)
    fingerprint = driver.viewer_fingerprint()
    driver.reclick_last_control()
    driver.await_viewer(fingerprint, label='gray canvas restored')
    gray_restored_image, _ = _record_snap(driver, records, "viewer-gray-canvas-restored")
    gray_restored = np.asarray(_viewer_crop(gray_restored_image), dtype=np.float32)
    gray_delta = float(np.mean(np.abs(gray - gray_restored)))
    records.append({"parity_action": "set_gray_18_canvas",
                    "scenario": "gray_canvas", "action": "toggle 18% gray canvas",
                    "assertion": "viewer pixels change and restore after toggle",
                    "mean_abs_pixel_change": gray_delta})
    _require(gray_delta > 0.1, "18% gray canvas did not alter the real viewer")

    border_state = copy.deepcopy(state)
    border_state.setdefault("display", {})["white_padding"] = 0.0
    border_state.setdefault("rust", {}).setdefault("viewer", {}).setdefault("settings", {})["white_padding"] = 0.0
    border_path = _state_path(root, "viewer-border-zero.json", border_state)
    fingerprint = driver.viewer_fingerprint()
    _load_state(driver, border_path, "viewer-border-zero")
    driver.tab("CONFIG")
    driver.await_viewer(fingerprint, label='white padding zero')
    before_border, _ = _record_snap(driver, records, "viewer-border-zero")
    fingerprint = driver.viewer_fingerprint()
    before_border_pixels = np.asarray(_viewer_crop(before_border).convert("RGB"))
    before_white = np.all(before_border_pixels >= 250, axis=2)
    _set_white_padding(driver)

    def white_padding_visible():
        # A late frame from loading the zero-padding state can change the
        # fingerprint before the padding composition finishes. Wait for the
        # requested white border itself before measuring the settled frame.
        _, canvas = driver.viewer_canvas()
        new_white = np.all(canvas >= 250, axis=2) & ~before_white
        return float(np.mean(new_white)) > 0.01

    try:
        wait_for(white_padding_visible, "visible nonzero white padding", timeout=60)
    except RuntimeError:
        _record_snap(driver, records, "viewer-border-nonzero-timeout")
        raise
    driver.await_viewer(fingerprint, label='white padding nonzero')
    after_border, _ = _record_snap(driver, records, "viewer-border-nonzero")
    border_delta = float(np.mean(np.abs(np.asarray(_viewer_crop(before_border), dtype=np.float32) -
                                        np.asarray(_viewer_crop(after_border), dtype=np.float32))))
    records.append({"scenario": "white_border", "action": "type White padding field",
                    "assertion": "viewer pixels change after nonzero border padding",
                    "mean_abs_pixel_change": border_delta,
                    "zero_screenshot": records[-2]["screenshot"],
                    "nonzero_screenshot": records[-1]["screenshot"]})
    _require(border_delta > 0.1, "White-padding field did not alter the real viewer")

    # Prepare fixed endpoints through a display change that clears animation,
    # independent of whether either animation setting actually works.
    reference_state = copy.deepcopy(state)
    reference_state.setdefault('simulation', {})['auto_preview'] = False
    settings = {'viewer_layer': 'output', 'reveal': False, 'crossfade': False}
    reference_state.setdefault('display', {}).update(settings)
    reference_state.setdefault('rust', {}).setdefault('viewer', {}).setdefault('settings', {}).update(settings)
    references = {}
    for auto_exposure in (False, True):
        reference_state['camera']['auto_exposure'] = auto_exposure
        label = 'viewer-reference-' + ('auto' if auto_exposure else 'manual')
        _load_state(driver, _state_path(root, label + '.json', reference_state), label + '-rendered')
        references[auto_exposure] = _present_output(driver, records, label)
    _require(_sample_distance(references[False], references[True]) > 1.0,
             'Animation exposure endpoints are indistinguishable')

    for enabled in (False, True):
        label = 'viewer-reveal' if enabled else 'viewer-reveal-disabled'
        animation_state = copy.deepcopy(reference_state)
        animation_state['camera']['auto_exposure'] = False
        settings = {'viewer_layer': 'output', 'reveal': enabled, 'crossfade': False}
        animation_state['display'].update(settings)
        animation_state['rust']['viewer']['settings'].update(settings)
        _load_state(driver, _state_path(root, label + '.json', animation_state), label + '-baseline')
        presented = _present_output(driver, records, label + '-baseline')
        _require(_sample_distance(presented, references[False]) <= 1.0,
                 'Reveal baseline differs from independently prepared Output')
        fingerprint = driver.viewer_fingerprint()
        driver.click('Paper back', False)
        paper_image, _ = driver.await_viewer(fingerprint, label=label + '-paper')
        before_sample = _sample(paper_image)
        driver.snap(label + '-before', paper_image)
        driver.tab('MAIN')
        driver.click('Preview', settle=False)
        frames, endpoint = _capture_to_target(driver, records, label, references[False])
        driver.wait_report(label + '-rendered', action='Preview')
        evidence = reveal_evidence(before_sample, frames, endpoint)
        records.append({'parity_action': 'polaroid_animation',
                        'scenario': 'reveal' if enabled else 'reveal_negative_control',
                        'action': 'explicit Preview with Reveal ' + ('enabled' if enabled else 'disabled'),
                        'frames': len(frames), **evidence})
        _require(evidence['passed'] == enabled,
                 'Reveal criterion failed' if enabled else 'Disabled Reveal passed animation criterion')

    for enabled in (False, True):
        label = 'viewer-crossfade' if enabled else 'viewer-crossfade-disabled'
        crossfade_state = copy.deepcopy(reference_state)
        crossfade_state['camera']['auto_exposure'] = False
        settings = {'viewer_layer': 'output', 'reveal': False, 'crossfade': enabled}
        crossfade_state['display'].update(settings)
        crossfade_state['rust']['viewer']['settings'].update(settings)
        _load_state(driver, _state_path(root, label + '.json', crossfade_state), label + '-baseline')
        before_sample = _present_output(driver, records, label + '-baseline')
        _require(_sample_distance(before_sample, references[False]) <= 1.0,
                 'Crossfade baseline differs from independently prepared Output')
        driver.tab('MAIN')
        driver.click('Auto exposure')
        driver.click('Preview', settle=False)
        frames, endpoint = _capture_to_target(driver, records, label, references[True])
        driver.wait_report(label + '-rendered', action='Preview')
        driver.snap(label + '-after')
        evidence = transition_evidence(before_sample, frames, endpoint)
        records.append({'scenario': 'crossfade' if enabled else 'crossfade_negative_control',
                        'action': 'explicit Preview with Crossfade ' + ('enabled' if enabled else 'disabled'),
                        'assertion': 'intermediate blend frames required only when enabled',
                        'frames': len(frames), **evidence})
        _require(evidence['passed'] == enabled,
                 'Crossfade criterion failed' if enabled else 'Disabled Crossfade passed animation criterion')

    _load_state(driver, baseline, "viewer-float-baseline")
    driver.tab("CONFIG")
    _hover_float(driver, np.asarray(pixels), records)

    _load_state(driver, baseline, "viewer-isolation-baseline")
    before_save = _save(driver, root, "viewer-isolation-before-save.exr")
    before_export = _export(driver, root, "viewer-isolation-before-export.exr")
    after_export_save = _save(driver, root, "viewer-isolation-after-export-save.exr")
    from gui_acceptance import read_image
    saved_before_export = read_image(before_save)
    saved_after_export = read_image(after_export_save)
    _require(saved_before_export.shape == saved_after_export.shape and
             np.array_equal(saved_before_export, saved_after_export),
             "Independent Export replaced the latest output used by Save")
    records.append({'scenario': 'export_output_snapshot',
                    'assertion': 'Save pixels and dimensions remain identical before and after independent full-input Export',
                    'save_before': before_save.name, 'save_after': after_export_save.name,
                    'export': before_export.name})
    driver.tab("CONFIG")
    driver.click("Paper back", False)
    _display_control(driver, "spline36")
    _display_control(driver, "nearest")
    _display_control(driver, "gray 18% canvas")
    driver.reclick_last_control()
    after_save = _save(driver, root, "viewer-isolation-after-save.exr")
    after_export = _export(driver, root, "viewer-isolation-after-export.exr")
    from gui_acceptance import read_image
    save_before = read_image(before_save)
    save_after = read_image(after_save)
    export_before = read_image(before_export)
    export_after = read_image(after_export)
    _require(save_before.shape == save_after.shape and export_before.shape == export_after.shape,
             "Viewer-only changes altered Save/Export dimensions")
    save_error = float(np.max(np.abs(save_before - save_after)))
    export_error = float(np.max(np.abs(export_before - export_after)))
    records.append({"parity_action": "save_output_layer",
                    "scenario": "display_output_isolation",
                    "action": "change viewer-only controls then Save and Export",
                    "assertion": "decoded Save and Export pixels remain invariant",
                    "save_before": before_save.name, "save_after": after_save.name,
                    "export_before": before_export.name, "export_after": after_export.name,
                    "save_max_abs_delta": save_error, "export_max_abs_delta": export_error})
    _require(save_error <= 1e-6 and export_error <= 1e-6,
             f"Viewer changed output pixels: save={save_error}, export={export_error}")

    _load_state(driver, baseline, "viewer-profile-baseline")
    current_film = state.get("simulation", {}).get("film_stock")
    current_paper = state.get("simulation", {}).get("print_paper")
    film_target, film_label = (("kodak_gold_200", "Kodak Gold 200")
                               if current_film == "kodak_portra_400"
                               else ("kodak_portra_400", "Kodak Portra 400"))
    paper_target, paper_label = (("kodak_supra_endura", "Kodak Professional Supra Endura")
                                if current_paper == "kodak_portra_endura"
                                else ("kodak_portra_endura", "Kodak Professional Portra Endura"))
    driver.tab("MAIN")
    profile_before_path = root / "viewer-profile-before-state.json"
    profile_before_path.unlink(missing_ok=True)
    driver.state_action("Save current to file", profile_before_path, True)
    _wait_file(profile_before_path, "viewer profile baseline state")
    driver.tab("MAIN")
    _select_profile(driver, "film profile", film_label)
    _select_profile(driver, "print profile", paper_label)
    driver.click("Preview")
    driver.rendered("viewer-profile-selected", action="Preview")
    selected_path = root / "viewer-profile-selected-state.json"
    selected_path.unlink(missing_ok=True)
    driver.state_action("Save current to file", selected_path, True)
    _wait_file(selected_path, "viewer profile state")
    selected_state = json.loads(selected_path.read_text())
    selected_simulation = selected_state["simulation"]
    _require(selected_simulation["film_stock"] == film_target, "Film profile selection was not persisted")
    _require(selected_simulation["print_paper"] == paper_target, "Paper profile selection was not persisted")
    records.append({"parity_action": "apply_profile_defaults",
                    "scenario": "profile_selection", "action": "select film and print profiles",
                    "assertion": "selected film and paper defaults persist in saved GUI state",
                    "film_stock": selected_simulation["film_stock"],
                    "print_paper": selected_simulation["print_paper"], "state": selected_path.name})

    non_srgb = copy.deepcopy(selected_state)
    non_srgb.setdefault("input_image", {})["input_color_space"] = "ProPhoto RGB"
    non_srgb.setdefault("simulation", {})["output_color_space"] = "Display P3"
    non_srgb["simulation"]["saving_color_space"] = "Display P3"
    runtime_io = non_srgb.setdefault('rust', {}).setdefault('runtime', {}).setdefault('io', {})
    runtime_io.update(input_color_space='ProPhoto RGB', output_color_space='Display P3',
                      saving_color_space='Display P3')
    non_srgb_path = _state_path(root, "viewer-non-srgb.json", non_srgb)
    driver.file_action("Open", source)
    driver.rendered("viewer-non-srgb-source")
    _load_state(driver, non_srgb_path, "viewer-non-srgb-state")
    persisted_path = root / "viewer-non-srgb-persisted.json"
    persisted_path.unlink(missing_ok=True)
    driver.state_action("Save current to file", persisted_path, True)
    _wait_file(persisted_path, "non-sRGB persisted state")
    persisted = json.loads(persisted_path.read_text())
    _require(persisted["input_image"]["input_color_space"] == "ProPhoto RGB",
             "Non-sRGB input space was not persisted after source load")
    _require(persisted["simulation"]["output_color_space"] == "Display P3",
             "Non-sRGB output space was not persisted after source load")
    non_srgb_output = _save(driver, root, "viewer-non-srgb.exr")
    non_srgb_pixels = read_image(non_srgb_output)
    records.append({"scenario": "non_srgb", "action": "load ProPhoto RGB and Display P3 state",
                    "assertion": "non-sRGB state persists and saved output is finite",
                    "input_color_space": persisted["input_image"]["input_color_space"],
                    "output_color_space": persisted["simulation"]["output_color_space"],
                    "output": non_srgb_output.name, "state": persisted_path.name,
                    "shape": list(non_srgb_pixels.shape), "finite": bool(np.isfinite(non_srgb_pixels).all())})
    _require(np.isfinite(non_srgb_pixels).all(), "Non-sRGB GUI save contains nonfinite pixels")

    _load_state(driver, baseline, "viewer-restored-state")
    driver.file_action("Open", source)
    driver.rendered("viewer-restored-source")
    records.append({"scenario": "viewer_restore", "action": "reload baseline state and source",
                    "assertion": "caller receives restored configured source",
                    "state": baseline.name, "source": source.name})
    return records
