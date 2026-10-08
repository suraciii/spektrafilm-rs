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
    driver.file_action("Load state", path)
    driver.rendered(label)


def _text(lines):
    return "\n".join(" ".join(word[0] for word in line) for line in lines)


def _viewer_crop(image):
    return image.crop((0, 80, min(1060, image.width), max(81, image.height - 80)))


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


def _display_line(driver, words):
    image, _, lines = driver.read()
    wanted = [word.lower() for word in words]
    for line in lines:
        normalized = [re.sub(r"[^a-z0-9]", "", word[0].lower()) for word in line]
        if all(any(item.startswith(term) for item in normalized) for term in wanted):
            return image, line
    raise RuntimeError(f"Could not locate display control: {' '.join(words)}")


def _drag_white_border(driver):
    # The White border DragValue sits left of its label in the CONFIG
    # sidebar; its row shifts with the surrounding controls. Drag the
    # slider track itself rather than the numeric DragValue text, because
    # horizontal motion over the text field is ignored by egui.
    image, line = _display_line(driver, ("white", "borde"))
    _require(image.width >= 1460 and image.height >= 980,
             "White-border probe requires the pinned 1460x980 window")
    numeric = [word for word in line if re.match(r"^\d", word[0])]
    _require(numeric, "White border value not visible")
    value = numeric[0]
    slider_x = int(value[1] - 50)
    y = value[2] + value[4] / 2
    driver.xd("mousemove", "--window", driver.window, slider_x, int(y))
    driver.xd("mousedown", 1)
    driver.xd("mousemove", "--window", driver.window, slider_x + 35, int(y))
    driver.xd("mouseup", 1)
    time.sleep(0.25)
    return image


def _click_combo(driver, label):
    image, line = _display_line(driver, tuple(label.lower().split()))
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
    driver.click("Input", False)
    driver.click("100%", False)
    bounds = driver.measure_viewer("viewer-float-inspection")
    x = bounds[0] + pixels.shape[1] // 2
    y = bounds[1] + pixels.shape[0] // 2
    driver.xd("mousemove", "--window", driver.window, x, y)

    def read_rgb():
        image, _, lines = driver.read()
        text = _text(lines)
        status = image.crop((15, 35, 650, 63)).resize((2540, 112))
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


def _export(driver, root, exporter, name):
    path = root / name
    path.unlink(missing_ok=True)
    driver.file_action("Export", path, True)
    driver.capture_export_child(exporter)
    driver.scroll(True)
    driver.wait_text(r"Exported.*f64", f"viewer-{name}-exported", timeout=240)
    driver.no_children()
    _wait_file(path, f"viewer export {name}")
    return path


def _save(driver, root, name):
    path = root / name
    path.unlink(missing_ok=True)
    driver.file_action("Save", path, True)
    _wait_file(path, f"viewer save {name}")
    return path


def accept_viewer(driver, root, state, pixels, exporter):
    """Run required viewer scenarios and return assertion records."""
    root = Path(root)
    state = copy.deepcopy(state)
    state.setdefault('rust', {})['save_bit_depth'] = 32
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
    full_export = _export(driver, root, exporter, "viewer-preview-resolution-export.exr")
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
    driver.wait_text(r"PaperBack\s*[·.]?\s*zoom", "paper-back-selected", timeout=20)
    image, lines = _record_snap(driver, records, "viewer-paper-back")
    paper_text = _text(lines)
    _require(re.search(r"PaperBack\s*[·.]?\s*(?:zoom|\()", paper_text, re.I),
             "Paper back viewer status was not selected in the real window")
    paper = np.asarray(_viewer_crop(image), dtype=np.float32)
    watermark_region = image.crop((320, 300, 700, 600))
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
                    "watermark_region_bounds": [320, 300, 700, 600]})
    _require(float(paper.std()) > 1.0 and float(watermark.std()) > 0.1,
             "Paper back screenshot has no visible presentation raster/watermark region")

    driver.click("Output", False)
    driver.click("spline36", False)
    driver.click("nearest", False)
    nearest_image, _ = _record_snap(driver, records, "viewer-interpolation-nearest")
    driver.click("nearest", False)
    driver.click("spline36", False)
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
    driver.click("Input", False)
    driver.click("spline36", False)
    driver.click("nearest", False)
    input_nearest_image, _ = _record_snap(driver, records, "viewer-input-nearest")
    driver.click("nearest", False)
    driver.click("spline36", False)
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
    driver.click("18% gray", False)
    gray_image, _ = _record_snap(driver, records, "viewer-gray-canvas-toggled")
    gray = np.asarray(_viewer_crop(gray_image), dtype=np.float32)
    driver.reclick_last_control()
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
    _load_state(driver, border_path, "viewer-border-zero")
    driver.tab("CONFIG")
    before_border, _ = _record_snap(driver, records, "viewer-border-zero")
    _drag_white_border(driver)
    after_border, _ = _record_snap(driver, records, "viewer-border-nonzero")
    border_delta = float(np.mean(np.abs(np.asarray(_viewer_crop(before_border), dtype=np.float32) -
                                        np.asarray(_viewer_crop(after_border), dtype=np.float32))))
    records.append({"scenario": "white_border", "action": "click White border slider",
                    "assertion": "viewer pixels change after nonzero border padding",
                    "mean_abs_pixel_change": border_delta,
                    "zero_screenshot": records[-2]["screenshot"],
                    "nonzero_screenshot": records[-1]["screenshot"]})
    _require(border_delta > 0.1, "White-border slider did not alter the real viewer")

    animation_state = copy.deepcopy(state)
    animation_state.setdefault("display", {}).update({"viewer_layer": "paper_back", "reveal": True})
    animation_state.setdefault("rust", {}).setdefault("viewer", {}).setdefault("settings", {}).update({"viewer_layer": "paper_back", "reveal": True})
    animation_path = _state_path(root, "viewer-animation.json", animation_state)
    _load_state(driver, animation_path, "viewer-animation-baseline")
    driver.tab("CONFIG")
    driver.click("Reveal", False)
    driver.click("Reveal", False)
    driver.tab("MAIN")
    driver.click('Paper back', False)
    driver.click("Preview")
    reveal_frames = _transition_frames(driver, records, "viewer-reveal", count=32, interval=0.05)
    driver.wait_text(r"Rendered\s+\d+", "viewer-reveal-rendered")
    reveal_delta = float(max(np.max(np.abs(reveal_frames[0] - frame)) for frame in reveal_frames[1:]))
    records.append({"parity_action": "polaroid_animation",
                    "scenario": "reveal", "action": "preview with Paper back and Reveal",
                    "assertion": "raw transition frames differ in the viewer region",
                    "frames": len(reveal_frames), "max_frame_delta": reveal_delta})
    _require(reveal_delta > 1.0, "Reveal did not produce observable transition frames")

    crossfade_state = copy.deepcopy(state)
    crossfade_state.setdefault("display", {}).update({"viewer_layer": "output", "reveal": False, "crossfade": True})
    crossfade_state.setdefault("rust", {}).setdefault("viewer", {}).setdefault("settings", {}).update({"viewer_layer": "output", "reveal": False, "crossfade": True})
    crossfade_path = _state_path(root, "viewer-crossfade.json", crossfade_state)
    _load_state(driver, crossfade_path, "viewer-crossfade-baseline")
    driver.tab("CONFIG")
    driver.click("Crossfade", False)
    driver.click("Crossfade", False)
    driver.click("Reveal", False)
    driver.click("Reveal", False)
    driver.tab("MAIN")
    driver.click("Output", False)
    _, _, lines = driver.read()
    if not driver.match(lines, 'Auto exposure', True):
        driver.xd('mousemove', '--window', driver.window, 1090, 331)
        driver.xd('click', 1)
        time.sleep(.8)
    _, _, lines = driver.read()
    matches = driver.match(lines, 'Auto exposure', True)
    _require(matches, 'Crossfade trigger requires visible Auto exposure')
    driver.xd('mousemove', '--window', driver.window, 1073, int(matches[0][1]))
    driver.xd('click', 1)
    crossfade_frames = _transition_frames(driver, records, "viewer-crossfade", count=32, interval=0.05)
    driver.wait_text(r"Rendered\s+\d+", "viewer-crossfade-rendered")
    crossfade_delta = float(max(np.max(np.abs(crossfade_frames[0] - frame)) for frame in crossfade_frames[1:]))
    records.append({"scenario": "crossfade", "action": "preview Output with Crossfade",
                    "assertion": "raw transition frames differ in the viewer region",
                    "frames": len(crossfade_frames), "max_frame_delta": crossfade_delta})
    _require(crossfade_delta > 1.0, "Crossfade did not produce observable transition frames")

    _load_state(driver, baseline, "viewer-float-baseline")
    driver.tab("CONFIG")
    _hover_float(driver, np.asarray(pixels), records)

    _load_state(driver, baseline, "viewer-isolation-baseline")
    before_save = _save(driver, root, "viewer-isolation-before-save.exr")
    before_export = _export(driver, root, exporter, "viewer-isolation-before-export.exr")
    driver.tab("CONFIG")
    driver.click("Paper back", False)
    driver.click("spline36", False)
    driver.click("nearest", False)
    driver.click("18% gray", False)
    driver.reclick_last_control()
    driver.click("Reveal", False)
    driver.click("Reveal", False)
    driver.click("Crossfade", False)
    driver.click("Crossfade", False)
    after_save = _save(driver, root, "viewer-isolation-after-save.exr")
    after_export = _export(driver, root, exporter, "viewer-isolation-after-export.exr")
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
                    "action": "change viewer-only controls then Save and f64 Export",
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
    driver.file_action("Save state", profile_before_path, True)
    _wait_file(profile_before_path, "viewer profile baseline state")
    profile_before = json.loads(profile_before_path.read_text())
    driver.tab("MAIN")
    _select_profile(driver, "Film stock", film_label)
    _select_profile(driver, "Print paper", paper_label)
    driver.click("Preview")
    driver.rendered("viewer-profile-selected")
    selected_path = root / "viewer-profile-selected-state.json"
    selected_path.unlink(missing_ok=True)
    driver.file_action("Save state", selected_path, True)
    _wait_file(selected_path, "viewer profile state")
    selected_state = json.loads(selected_path.read_text())
    selected_simulation = selected_state["simulation"]
    _require(selected_simulation["film_stock"] == film_target, "Film profile selection was not persisted")
    _require(selected_simulation["print_paper"] == paper_target, "Paper profile selection was not persisted")
    before_grain = profile_before["rust"]["runtime"]["film_render"]["grain"]["rms_granularity"]
    selected_grain = selected_state["rust"]["runtime"]["film_render"]["grain"]["rms_granularity"]
    _require(before_grain != selected_grain,
             "Film profile selection did not apply stock-specific grain defaults")
    records.append({"parity_action": "apply_profile_defaults",
                    "scenario": "profile_selection", "action": "select Film stock and Print paper",
                    "assertion": "selected film and paper defaults persist in saved GUI state",
                    "film_stock": selected_simulation["film_stock"],
                    "print_paper": selected_simulation["print_paper"], "state": selected_path.name})
    records.append({"parity_action": "apply_film_profile_defaults",
                    "scenario": "profile_selection",
                    "assertion": "film selection changes stock-specific grain defaults",
                    "film_stock": selected_simulation["film_stock"],
                    "grain_before": before_grain, "grain_after": selected_grain,
                    "state": selected_path.name})

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
    driver.file_action("Save state", persisted_path, True)
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
