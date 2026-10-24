#!/usr/bin/env python3
"""Consumer-visible viewer acceptance scenarios for the native GUI."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import re
import sys
import time

import numpy as np

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
    _preview(driver, label)


def _start_preview(driver):
    driver.close_settings()
    driver.drain_reports()
    point = wait_for(lambda: driver.locate("PREVIEW", bottom=True), "persistent PREVIEW action", 25)
    driver.snap("viewer-preview-request")
    driver.xd("mousemove", "--window", driver.window, int(point[0]), int(point[1]))
    driver.xd("click", 1)


def _preview(driver, label):
    _start_preview(driver)
    driver.wait_report(label, "Preview")
    driver.snap(label)


def _viewer_control(driver, path, label):
    driver.reveal(path, "Viewer", "Viewer")
    driver.click(label, bottom=True)


def _text(lines):
    return "\n".join(" ".join(word[0] for word in line) for line in lines)


def _viewer_status_text(image, lines):
    # The overlay is at the top of the central canvas, below the header.
    left, _, right, _ = viewer_canvas_bounds(image)
    top = 90 + (30 if sys.platform == "darwin" else 0)
    return _text([[word for word in line
                   if left <= word[1] + word[3] / 2 < right
                   and top <= word[2] + word[4] / 2 < top + 45]
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


def _transition_frames(driver, records, label, count=24, interval=0.05):
    frames = []
    for index in range(count):
        image = _record_frame(driver, records, f"{label}-frame-{index}")
        frames.append(np.asarray(_viewer_crop(image), dtype=np.float32))
        time.sleep(interval)
    return frames


def _preview_transition_frames(driver, records, label):
    """Capture during rendering and after handoff without blocking on OCR."""
    _start_preview(driver)
    frames = []
    deadline = time.monotonic() + 120
    completed = False
    while time.monotonic() < deadline:
        image = _record_frame(driver, records, f"{label}-render-frame-{len(frames)}")
        frames.append(np.asarray(_viewer_crop(image), dtype=np.float32))
        for path in driver.report_directory.glob("operation-*.json"):
            try:
                operation = json.loads(path.read_text())["operation"]
            except (OSError, ValueError, KeyError):
                continue
            if (operation["id"] > driver.last_report_id
                    and operation["kind"] == "preview"
                    and operation["outcome"] != "superseded"):
                completed = True
                break
        if completed:
            break
        time.sleep(.05)
    _require(completed, f"Timed out capturing {label} render")
    driver.wait_report(label + "-rendered", "Preview")
    frames.extend(_transition_frames(driver, records, label, count=32, interval=.05))
    changed = sum(float(np.max(np.abs(a - b))) > 1.0
                  for a, b in zip(frames, frames[1:]))
    _require(changed >= 2, f"{label} has no observable intermediate transition frames")
    return frames


def _set_white_padding(driver, value="0.2"):
    driver.reveal("extras.display.white_padding", "Settings", "Display")
    image, _, lines = driver.read()
    matches = driver.match(lines, "white padding")
    _require(matches, "Settings white-padding row is not visible")
    x, y = matches[-1]
    numeric = [word for line in lines for word in line
               if word[1] > x and abs(word[2] + word[4] / 2 - y) < 10
               and re.fullmatch(r"[-+−]?\d+(?:[.,]\d+)?", word[0].strip("[]()|"))]
    _require(numeric, "Settings white-padding editor is not visible")
    field = min(numeric, key=lambda word: word[1])
    driver.point_click((field[1] + field[3] / 2, field[2] + field[4] / 2))
    driver.xd("key", "ctrl+a")
    driver.xd("type", "--clearmodifiers", "--", value)
    driver.xd("key", "Return")
    driver.xd("key", "Tab")
    driver.close_settings()
    return image


def _select_profile(driver, label, option):
    path, owner, section = {
        "film profile": ("extras.simulation.film_stock", "Film", "Film material"),
        "print profile": ("extras.simulation.print_paper", "Print", "Print material"),
    }[label]
    driver.reveal(path, owner, section)
    driver.choose(label, option, below=True)


def _interpolation(driver, option):
    driver.reveal("extras.display.output_interpolation", "Settings", "Display")
    driver.choose("output interpolation", option)
    driver.close_settings()


def _gray_canvas(driver):
    driver.reveal("extras.display.gray_18_canvas", "Settings", "Display")
    driver.click("gray 18% canvas")
    driver.close_settings()


def _hover_float(driver, pixels, records):
    _viewer_control(driver, "viewer.layer.input", "Input")
    _viewer_control(driver, "viewer.zoom.100", "100%")
    bounds = driver.measure_viewer("viewer-float-inspection")
    x = bounds[0] + pixels.shape[1] // 2
    y = bounds[1] + pixels.shape[0] // 2
    driver.xd("mousemove", "--window", driver.window, x, y)

    def read_rgb():
        image, _, lines = driver.read()
        text = _text(lines)
        left, _, right, _ = viewer_canvas_bounds(image)
        top = 90 + (30 if sys.platform == "darwin" else 0)
        status = image.crop((left, top, right, top + 45))
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



def _export(driver, root, name):
    path = root / name
    path.unlink(missing_ok=True)
    driver.close_settings()
    driver.click("Export", header=True)
    driver.export_options(path)
    driver.dialog(path, True)
    driver.exported(path, f"viewer-{name}-exported")
    return path


def _save(driver, root, name):
    path = root / name
    path.unlink(missing_ok=True)
    driver.close_settings()
    driver.click("SAVE", bottom=True)
    driver.dialog(path, True)
    driver.wait_text(r"Saved\s+" + re.escape(path.name), f"viewer-{name}-saved", timeout=30)
    driver.wait_report(f"viewer-{name}-saved-report", 'Save')
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


def _zoom(driver, pixels, records):
    _viewer_control(driver, "viewer.layer.input", "Input")
    original, _ = driver.image()
    required_width = 260 + 420 + pixels.shape[1] * 4 + 80
    required_height = pixels.shape[0] * 4 + 240
    driver.resize_window(max(original.width, required_width), max(original.height, required_height))
    driver.search_point = None
    bounds = []
    try:
        for percent in (100, 200, 400):
            _viewer_control(driver, f"viewer.zoom.{percent}", f"{percent}%")
            driver.wait_text(r"zoom\s*" + str(percent), f"viewer-zoom-{percent}")
            measured = driver.measure_viewer(f"viewer-zoom-{percent}-pixels")
            bounds.append(measured)
        widths = [right - left for left, _, right, _ in bounds]
        expected = [pixels.shape[1] * scale for scale in (1, 2, 4)]
        _require(widths == expected, f"Exact zoom pixel widths differ: {widths}, expected {expected}")
        records.append({"scenario": "exact_zoom", "action": "select 100%, 200%, 400%",
                        "assertion": "native Input raster widths equal source width times exact zoom",
                        "pixel_bounds": bounds, "pixel_widths": widths})
    finally:
        driver.resize_window(original.width, original.height)
        driver.search_point = None
    _viewer_control(driver, "viewer.reset", "reset view")
    driver.await_viewer(label="fitted and recentered Input")


def _display_evidence(driver, root, records):
    _viewer_control(driver, "viewer.zoom.100", "100%")
    _viewer_control(driver, "viewer.layer.input", "Input")
    input_image, _ = driver.await_viewer(label="input comparison")
    input_pixels = np.asarray(_viewer_crop(input_image), dtype=np.float32)
    input_path = root / "input-viewer-raster.png"
    _viewer_crop(input_image).save(input_path)
    _viewer_control(driver, "viewer.layer.output", "Output")
    output_image, _ = driver.await_viewer(label="output comparison")
    output_pixels = np.asarray(_viewer_crop(output_image), dtype=np.float32)
    output_path = root / "output-viewer-raster.png"
    _viewer_crop(output_image).save(output_path)
    difference = float(np.mean(np.abs(input_pixels - output_pixels)))
    _require(difference > .1, "Input/output controls did not change viewer pixels")
    chroma = output_pixels.max(axis=2) - output_pixels.min(axis=2)
    content = output_pixels[(chroma > 25) & (output_pixels.max(axis=2) > 35)]
    _require(content.shape[0] > 100, "Output viewer has no measurable rendered pixels")
    contrast = float(content.std())
    _require(contrast > 5, "Output viewer lost gradient contrast")
    records.append({"scenario": "input_output_rasters",
                    "assertion": "Input and Output differ and Output retains visible gradient contrast",
                    "viewer_roi": list(viewer_canvas_bounds(input_image)),
                    "input_raster": input_path.name, "output_raster": output_path.name,
                    "mean_abs_pixel_change": difference, "output_pixel_std": contrast,
                    "output_rendered_pixel_count": int(content.shape[0])})
    driver.reveal("extras.display.use_display_transform", "Settings", "Display")
    image, _, lines = driver.read()
    text = _text(lines)
    _require(re.search(r"display\s+transform", text, re.I),
             "Settings did not report display transform status")
    driver.snap("display-transform-status", image, lines)
    records.append({"parity_action": "report_display_transform_status",
                    "scenario": "display_transform_status",
                    "assertion": "Settings reports the active display transform status",
                    "screenshot": driver.records[-1]["screenshot"], "status": text})
    driver.close_settings()


def accept_viewer(driver, root, state, pixels, exporter):
    """Run required viewer scenarios and return assertion records."""
    root = Path(root)
    state = copy.deepcopy(state)
    source = root / "standard.tif"
    _require(source.is_file(), f"Viewer acceptance source is missing: {source}")
    records = []
    small_preview_state = copy.deepcopy(state)
    small_preview_state.setdefault("display", {})["preview_max_size"] = 128
    small_preview_state.setdefault("rust", {}).setdefault("runtime", {}).setdefault("settings", {})["preview_max_size"] = 128
    small_preview_state["display"].update({"viewer_layer": "input", "crossfade": False})
    viewer = small_preview_state.setdefault('rust', {}).setdefault('viewer', {})
    viewer.setdefault('settings', {}).update({'viewer_layer': 'input', 'crossfade': False})
    # Magnified Fit and every exact zoom intentionally use full-source pixels.
    # A relative fitted zoom below 1:1 exposes the accepted preview raster.
    viewer.update(zoom=0.15, zoom_percent=None, pan=[0, 0])
    small_preview_path = _state_path(root, "viewer-small-preview.json", small_preview_state)
    _load_state(driver, small_preview_path, "viewer-small-preview")
    _viewer_control(driver, "viewer.layer.input", "Input")
    driver.await_viewer(label="small Input raster")
    small_input, _ = _record_snap(driver, records, "viewer-small-input-raster")
    large_preview_state = copy.deepcopy(small_preview_state)
    large_preview_state["display"]["preview_max_size"] = 256
    large_preview_state["rust"]["runtime"]["settings"]["preview_max_size"] = 256
    large_preview_path = _state_path(root, "viewer-large-preview.json", large_preview_state)
    _load_state(driver, large_preview_path, "viewer-large-preview")
    _viewer_control(driver, "viewer.layer.input", "Input")
    driver.await_viewer(label="large Input raster")
    large_input, _ = _record_snap(driver, records, "viewer-large-input-raster")
    # The source occupies only a small part of a nonmagnifying canvas. Compare
    # its observed colored raster, not unrelated gray background dilution.
    small_canvas = np.asarray(_viewer_crop(small_input), dtype=np.float32)
    large_canvas = np.asarray(_viewer_crop(large_input), dtype=np.float32)
    def preview_bounds(image):
        mask = ((image.max(axis=2) - image.min(axis=2) > 25)
                & (image.max(axis=2) > 35))
        ys, xs = np.where(mask)
        _require(xs.size > 100, 'Preview raster is not measurable')
        return [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1]
    small_bounds, large_bounds = preview_bounds(small_canvas), preview_bounds(large_canvas)
    _require(small_bounds == large_bounds, 'Preview-size change moved the Input raster')
    left, top, right, bottom = small_bounds
    preview_delta = float(np.mean(np.abs(small_canvas[top:bottom, left:right] -
                                        large_canvas[top:bottom, left:right])))
    canvas_delta = float(np.mean(np.abs(small_canvas - large_canvas)))
    _require(preview_delta > 0.01, "Preview max size did not refresh Input raster")
    full_export = _export(driver, root, "viewer-preview-resolution-export.exr")
    from gui_acceptance import read_image
    _require(read_image(full_export).shape[:2] == np.asarray(pixels).shape[:2],
             "Input preview raster size reduced full-resolution Export")
    records.append({"parity_action": "refresh_preview_cache",
                    "scenario": "preview_size_isolation", "input_pixel_change": preview_delta,
                    'input_raster_bounds': small_bounds, 'canvas_pixel_change': canvas_delta,
                    "assertion": "Input preview refreshes; Export retains source dimensions"})
    _load_state(driver, _state_path(root, "viewer-reset-after-preview.json", state), "viewer-reset-after-preview")
    baseline = _state_path(root, "viewer-baseline.json", copy.deepcopy(state))

    _viewer_control(driver, "viewer.layer.paper_back", "Paper back")
    driver.await_viewer(label="Paper back")
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

    _viewer_control(driver, "viewer.layer.output", "Output")
    _interpolation(driver, "spline36")
    driver.await_viewer(label="baseline spline36")
    fingerprint = driver.viewer_fingerprint()
    _interpolation(driver, "nearest")
    driver.await_viewer(fingerprint, label='interpolation nearest')
    nearest_image, _ = _record_snap(driver, records, "viewer-interpolation-nearest")
    fingerprint = driver.viewer_fingerprint()
    _interpolation(driver, "spline36")
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
    _viewer_control(driver, "viewer.layer.input", "Input")
    _interpolation(driver, "nearest")
    driver.await_viewer(label='input interpolation nearest')
    input_nearest_image, _ = _record_snap(driver, records, "viewer-input-nearest")
    _interpolation(driver, "spline36")
    driver.await_viewer(label='input interpolation spline36')
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
    _gray_canvas(driver)
    driver.await_viewer(fingerprint, label='gray canvas toggled')
    gray_image, _ = _record_snap(driver, records, "viewer-gray-canvas-toggled")
    gray = np.asarray(_viewer_crop(gray_image), dtype=np.float32)
    fingerprint = driver.viewer_fingerprint()
    _gray_canvas(driver)
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

    animation_state = copy.deepcopy(state)
    animation_state.setdefault("display", {}).update({"viewer_layer": "paper_back", "reveal": True})
    animation_state.setdefault("rust", {}).setdefault("viewer", {}).setdefault("settings", {}).update({"viewer_layer": "paper_back", "reveal": True})
    animation_path = _state_path(root, "viewer-animation.json", animation_state)
    _load_state(driver, animation_path, "viewer-animation-baseline")
    _viewer_control(driver, "viewer.layer.paper_back", "Paper back")
    driver.await_viewer(label="reveal baseline")
    reveal_frames = _preview_transition_frames(driver, records, "viewer-reveal")
    reveal_delta = float(max(np.max(np.abs(reveal_frames[0] - frame)) for frame in reveal_frames[1:]))
    records.append({"parity_action": "polaroid_animation",
                    "scenario": "reveal", "action": "preview with Paper back and Reveal",
                    "assertion": "raw transition frames differ in the viewer region",
                    "frames": len(reveal_frames), "max_frame_delta": reveal_delta})
    _require(reveal_delta > 1.0, "Reveal did not produce observable transition frames")

    crossfade_state = copy.deepcopy(state)
    crossfade_state.setdefault("simulation", {})["auto_preview"] = False
    crossfade_state.setdefault("display", {}).update({"viewer_layer": "output", "reveal": False, "crossfade": True})
    crossfade_state.setdefault("rust", {}).setdefault("viewer", {}).setdefault("settings", {}).update({"viewer_layer": "output", "reveal": False, "crossfade": True})
    crossfade_path = _state_path(root, "viewer-crossfade.json", crossfade_state)
    _load_state(driver, crossfade_path, "viewer-crossfade-baseline")
    _viewer_control(driver, "viewer.layer.output", "Output")
    driver.await_viewer(label="crossfade baseline")
    driver.reveal("camera.auto_exposure", "Input and capture", "Camera")
    driver.toggle('auto exposure')
    crossfade_frames = _preview_transition_frames(driver, records, "viewer-crossfade")
    crossfade_delta = float(max(np.max(np.abs(crossfade_frames[0] - frame)) for frame in crossfade_frames[1:]))
    records.append({"scenario": "crossfade", "action": "preview Output with Crossfade",
                    "assertion": "raw transition frames differ in the viewer region",
                    "frames": len(crossfade_frames), "max_frame_delta": crossfade_delta})
    _require(crossfade_delta > 1.0, "Crossfade did not produce observable transition frames")

    detail_state = copy.deepcopy(state)
    detail_state.setdefault("display", {})["preview_max_size"] = max(256, max(np.asarray(pixels).shape[:2]))
    detail_state.setdefault("rust", {}).setdefault("runtime", {}).setdefault("settings", {})["preview_max_size"] = detail_state["display"]["preview_max_size"]
    detail_path = _state_path(root, "viewer-full-detail.json", detail_state)
    _load_state(driver, detail_path, "viewer-float-baseline")
    _hover_float(driver, np.asarray(pixels), records)
    _zoom(driver, np.asarray(pixels), records)
    _display_evidence(driver, root, records)

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
    _viewer_control(driver, "viewer.layer.paper_back", "Paper back")
    _interpolation(driver, "nearest")
    _gray_canvas(driver)
    _gray_canvas(driver)
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
    profile_before_path = root / "viewer-profile-before-state.json"
    profile_before_path.unlink(missing_ok=True)
    driver.state_action("Save current to file", profile_before_path, True)
    _wait_file(profile_before_path, "viewer profile baseline state")
    _select_profile(driver, "film profile", film_label)
    _select_profile(driver, "print profile", paper_label)
    _preview(driver, "viewer-profile-selected")
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
    runtime_io.update(input_color_space='ProPhoto RGB', output_color_space='Display P3')
    non_srgb_path = _state_path(root, "viewer-non-srgb.json", non_srgb)
    driver.open_image(source, "viewer-non-srgb-source")
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
    driver.open_image(source, "viewer-restored-source")
    _preview(driver, "viewer-restored-output")
    records.append({"scenario": "viewer_restore", "action": "reload baseline state and source",
                    "assertion": "caller receives restored configured source",
                    "state": baseline.name, "source": source.name})
    return records
