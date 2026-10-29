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
        regions.append((max(0, image.width // 2 - 340), max(0, image.height // 2 - 70),
                        min(image.width, image.width // 2 + 340),
                        min(image.height, image.height // 2 + 70), 6))
        regions.append((260, 96, max(261, image.width - 320), 130, 7))
        # The amber Export button has a different contrast from adjacent
        # header text. A whole-row OCR pass drops its label; overlapping local
        # observations retain the actual rendered button and its coordinates.
        regions.extend((left, 0, min(image.width, left + 240), 26, 7)
                       for left in range(0, image.width, 160))
        regions.extend((left, 0, min(image.width, left + 80), 26, 13)
                       for left in range(0, min(image.width, 800), 60))
        if self.settings_open:
            # egui places Settings beside the preset panel, not necessarily at
            # the screen center. Observe the whole modal separately from the
            # photograph, whose pixels can otherwise disrupt block segmentation.
            regions.append((260, 90, min(image.width, 840), min(image.height, 800), 6))
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
            if self.settings_open:
                if segmentation != 13:
                    for row in grouped.values():
                        if not any(word[0].lower().startswith('requested') for word in row):
                            continue
                        row_left = max(0, int(min(word[1] for word in row) - 3))
                        row_top = max(0, int(min(word[2] for word in row) - 3))
                        region = (row_left, row_top, min(image.width, row_left + 500),
                                  min(image.height, int(max(word[2] + word[4] for word in row) + 4)), 13)
                        if region not in regions:
                            regions.append(region)
                # The state toolbar follows the observed GUI parameters
                # heading. Read it as one row; block OCR drops highlighted Save.
                for row in grouped.values():
                    labels = [word for word in row if words(word[0]) in (['gui'], ['parameters'])]
                    if {tuple(words(word[0])) for word in labels} != {('gui',), ('parameters',)}:
                        continue
                    if min(word[1] for word in labels) >= image.width - 520:
                        continue
                    left = max(0, int(min(word[1] for word in labels) - 12))
                    top = int(max(word[2] + word[4] for word in labels) + 4)
                    region = (left, top, min(image.width, left + 560),
                              min(image.height, top + 30), 7)
                    if region not in regions:
                        regions.append(region)
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
            if bottom and label in ('SCAN', 'SAVE'):
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
        if label in ('SCAN', 'SAVE', 'Export file'):
            self.drain_reports()
        self.point_click(point)

    def close_settings(self):
        if self.settings_open:
            # Escape is the production Settings close action and restores its
            # caller's focus. Observe closure instead of updating a shadow flag.
            self.xd('key', 'Escape')
            wait_for(lambda: self.locate('Close settings') is None,
                     'Settings dialog closed', 20)
            self.settings_open = False

    def toggle(self, label):
        """Toggle the observed on/off editor, not its separate row label."""
        self.xd('mousemove', '--window', self.window, 700, 100)
        image, _, lines = self.read()
        wanted = ''.join(words(label))
        spans = []
        for line in lines:
            for start in range(len(line)):
                for end in range(start + 1, len(line) + 1):
                    span = line[start:end]
                    if (span[0][1] >= image.width - 350
                            and ''.join(words(' '.join(word[0] for word in span))) == wanted):
                        spans.append(span)
        require(spans, f'Boolean control label is not visible: {label}')
        from PIL import ImageOps
        for span in spans:
            right = int(max(word[1] + word[3] for word in span))
            y = sum(word[2] + word[4] / 2 for word in span) / len(span)
            # Adjacent flat on/off button has 12px padding after its label;
            # exclude its border and locator focus ring before recognizing it.
            left, top = right + 11, int(y - 8)
            crop = ImageOps.invert(ImageOps.grayscale(
                image.crop((left, top, min(image.width, left + 40), int(y + 9)))))
            data = self.ocr.image_to_data(crop.resize((crop.width * 8, crop.height * 8)),
                                          config='--psm 7', output_type=self.ocr.Output.DICT)
            values = [(text, left + data['left'][i] / 8, top + data['top'][i] / 8,
                       data['width'][i] / 8, data['height'][i] / 8)
                      for i, text in enumerate(data['text']) if words(text) in (['on'], ['off'])]
            if values:
                value = values[0]
                self.point_click((value[1] + value[3] / 2, value[2] + value[4] / 2))
                return
        raise RuntimeError(f'Boolean on/off editor is not visible: {label}')

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
        self.drain_reports()
        self.dialog(path)
        self.discard_photograph_changes()
        # Clear the pointer/hover tooltip from the source and header labels.
        self.xd('mousemove', '--window', self.window, 700, 100)
        # The photo-first header shows the source file name without a label.
        self.wait_text(re.escape(path.name), label)
        report = self.wait_report(label + '-automatic-preview', 'Preview')
        self.snap(label + '-automatic-preview')
        return report

    def discard_photograph_changes(self):
        # Complete-state transitions protect accepted photograph adjustments.
        # Isolated scenarios explicitly discard through the actual modal.
        try:
            confirmation = self.locate('Save this photograph’s adjustments')
        except RuntimeError:
            if self.proc.poll() is None:
                raise
            return
        if confirmation:
            self.snap('unsaved-photograph-confirmation')
            self.click('Discard changes')
            self.records.append({'photograph_transition': 'discard_changes',
                                 'confirmation_observed': True})

    def state_action(self, label, path, save=False):
        self.reveal('action.save_state' if save else 'action.load_state',
                    'Settings', 'GUI parameters')
        self.click(label)
        if not save:
            self.drain_reports()
        self.dialog(path, save)
        if not save:
            self.discard_photograph_changes()
        result = None
        if save:
            wait_for(path.is_file, 'native saved state', 20)
            result = json.loads(path.read_text())
        else:
            # Loading complete state replaces the accepted source/output and
            # automatically renders it. Consume that report here exactly once.
            has_source = bool(self.locate('Source:', header=True))
            expected = json.loads(path.read_text())
            if has_source and self.locate('No image loaded', header=True) is None:
                report = self.wait_report('state-loaded-' + path.stem, 'Preview')
                result = report
                route = dict(zip(ROUTES, ('input', 'film_scan', 'film_print_scan',
                             'convert_film_print_scan', 'convert_film_scan_minus_base',
                             'convert_film_scan')))[expected['simulation']['route']]
                require(report['configuration']['render']['workflow_route'] == route,
                        'State-load preview did not use the selected workflow route')
                rendered = report['configuration']['render']
                for state_key, report_key in (('scan_output', 'scan_output'),
                                              ('conversion_style', 'conversion_style'),
                                              ('operator', 'scan_operator')):
                    require(parameters_equal(expected['scanner'][state_key], rendered[report_key]),
                            f'State-load preview did not use loaded scanner {state_key}')
                self.snap('state-loaded-' + path.stem)
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
            line_tokens = words(' '.join(word[0] for word in line))
            if (not line or line_tokens[:1] == ['locate']
                    or 'reset' in line_tokens or 'factory' in line_tokens
                    or any(token.startswith('reset') for token in line_tokens)):
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
            # Raw-line segmentation preserves text inside the combo frame.
            data = self.ocr.image_to_data(crop.resize((crop.width * 5, crop.height * 5)),
                                          config='--psm 13', output_type=self.ocr.Output.DICT)
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
            for _ in range(16 if direction == 4 else 80):
                frame, _, rows = self.read()
                # Popup rows need their own text block: the retained viewer
                # image and adjacent inspector headings otherwise merge them.
                from PIL import ImageOps, ImageStat
                left = max(0, int(min(word[1] for word in candidates)))
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
                    self.point_click(((min(word[1] for word in line) + max(word[1] + word[3] for word in line)) / 2,
                                      (min(word[2] for word in line) + max(word[2] + word[4] for word in line)) / 2))
                    return
                self.xd('mousemove', '--window', self.window,
                        int(selected[1] + selected[3] / 2), int(y + 70))
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
            for label in ('SCAN', 'SAVE'):
                require(self.locate(label, bottom=True), f'{name}: persistent {label} missing')
            require(self.locate('PREVIEW', bottom=True) is None,
                    f'{name}: retired PREVIEW action remains visible')
        self.reveal('action.save_state', 'Settings', 'GUI parameters')
        require(self.locate('Save current to file'), 'Settings state controls missing')
        self.close_settings()

    def export_options(self, path, float_export=False, saving_color_space=None,
                       saving_cctf_encoding=None):
        def modal_lines(image, box, scale=3, psm=6):
            from PIL import ImageOps
            crop = image.crop(box)
            grouped = {}
            # Keep each contrast pass's lines independent: their word boxes
            # are alternative observations, not parts of one physical row.
            for pass_index, prepare in enumerate((ImageOps.equalize, ImageOps.autocontrast)):
                prepared = prepare(ImageOps.invert(ImageOps.grayscale(crop)))
                data = self.ocr.image_to_data(prepared.resize((crop.width * scale, crop.height * scale)),
                                              config=f'--psm {psm}', output_type=self.ocr.Output.DICT,
                                              timeout=15)
                for i, text in enumerate(data['text']):
                    if text.strip():
                        if text.casefold() == 'bitdepth':
                            text = 'Bit depth'
                        key = (pass_index, data['block_num'][i], data['par_num'][i],
                               data['line_num'][i])
                        grouped.setdefault(key, []).append((text, box[0] + data['left'][i] / scale,
                            box[1] + data['top'][i] / scale, data['width'][i] / scale,
                            data['height'][i] / scale))
            return list(grouped.values())

        def matched_span(line, label):
            wanted = ''.join(re.findall(r'[a-z0-9]+', label.lower()))
            for start in range(len(line)):
                if not re.search(r'[a-z0-9]', line[start][0].lower()):
                    continue
                text = ''
                for end in range(start, len(line)):
                    text += ''.join(re.findall(r'[a-z0-9]+', line[end][0].lower()))
                    if text == wanted:
                        return line[start:end + 1]
                    if len(text) >= len(wanted):
                        break
            return None

        def center(span):
            return ((min(word[1] for word in span) + max(word[1] + word[3] for word in span)) / 2,
                    (min(word[2] for word in span) + max(word[2] + word[4] for word in span)) / 2)

        def flat_bounds(image, span, button=False):
            """Follow rendered fills; row OCR may include the button border."""
            pixels = np.asarray(image)[:, :, :3].astype(np.int16)
            if button:
                cx, cy = center(span)
                text_height = max(word[4] for word in span)
                text_width = max(word[1] + word[3] for word in span) - min(word[1] for word in span)
                candidates = []
                tx = int(cx)
                for ty in range(max(0, int(cy - text_height)),
                                min(image.height, int(cy + text_height) + 1)):
                    background = pixels[ty, tx]
                    strip = np.max(np.abs(pixels[ty] - background), axis=1) <= 3
                    left, right = tx, tx + 1
                    while left > 0 and strip[left - 1]:
                        left -= 1
                    while right < image.width and strip[right]:
                        right += 1
                    if (right - left < text_width * .8
                            or left > min(word[1] for word in span) + text_height * .25
                            or right < max(word[1] + word[3] for word in span) - text_height * .25):
                        continue
                    edge_x = max(left, right - 3)
                    top, bottom = ty, ty + 1
                    while top > 0 and np.max(np.abs(pixels[top - 1, edge_x] - background)) <= 3:
                        top -= 1
                    while bottom < image.height and np.max(
                            np.abs(pixels[bottom, edge_x] - background)) <= 3:
                        bottom += 1
                    if left <= cx < right and top <= cy < bottom and bottom - top >= text_height * .6:
                        candidates.append((left, top, right, bottom))
                return min(candidates, key=lambda box: (box[2] - box[0]) * (box[3] - box[1])) if candidates else None
            tx = int(center(span)[0])
            ty = max(0, int(min(word[2] for word in span)) - 2)
            background = pixels[ty, tx]
            strip = np.max(np.abs(pixels[ty] - background), axis=1) <= 3
            left, right = tx, tx + 1
            while left > 0 and strip[left - 1]:
                left -= 1
            while right < image.width and strip[right]:
                right += 1
            top = ty
            while top > 0 and np.max(np.abs(pixels[top - 1, tx] - background)) <= 3:
                top -= 1
            # The inner right edge avoids the label and the combo arrow.
            edge_x = max(left, right - 3)
            bottom = ty + 1
            while bottom < image.height and np.max(
                    np.abs(pixels[bottom, edge_x] - background)) <= 3:
                bottom += 1
            if all(left <= word[1] and word[1] + word[3] <= right
                   and top <= word[2] and word[2] + word[4] <= bottom for word in span):
                return left, top, right, bottom
            return None

        title_band = None
        title_point = None

        def modal_read():
            nonlocal title_band, title_point
            image, _ = self.image()
            whole = (0, 0, image.width, image.height)
            discovery = modal_lines(image, title_band or whole, 2, 11)
            titles = [span for line in discovery
                      if (span := matched_span(line, 'Export options'))]
            if not titles and title_band is not None:
                discovery = modal_lines(image, whole, 2, 11)
                titles = [span for line in discovery
                          if (span := matched_span(line, 'Export options'))]
            if not titles:
                return None
            pixels = np.asarray(image)[:, :, :3].astype(np.int16)
            for title in sorted(titles, key=lambda span: span[0][2]):
                header = flat_bounds(image, title)
                if header is None:
                    continue
                left, top, right, header_bottom = header
                edge_x = max(left, right - 4)
                body_top = int(max(word[2] + word[4] for word in title)) + 1
                background = pixels[max(0, int(min(word[2] for word in title)) - 2),
                                    int(center(title)[0])]
                while body_top < image.height and not np.all(
                        pixels[body_top, edge_x] < background - 3):
                    body_top += 1
                if body_top == image.height:
                    continue
                body = pixels[body_top, edge_x]
                bottom = body_top + 1
                while bottom < image.height and np.max(
                        np.abs(pixels[bottom, edge_x] - body)) <= 3:
                    bottom += 1
                if bottom <= body_top + 1:
                    continue
                box = (left, top, right, min(image.height, bottom + header_bottom - top))
                lines = modal_lines(image, box)
                cancels = [span for line in lines
                           if (span := matched_span(line, 'Cancel'))
                           and span[0][2] > header_bottom]
                if not cancels:
                    continue
                # Only the title search is cached. Format/color changes may
                # change both dialog dimensions; always remeasure the body.
                title_band = (0, top, image.width, header_bottom)
                # Park in the observed blank header gap, never on title text:
                # the captured pointer can turn "options" into "pptions".
                title_point = ((left + min(word[1] for word in title)) / 2,
                               (top + header_bottom) / 2)
                return image, box, lines
            return None

        def park():
            self.xd('mousemove', '--window', self.window, *map(int, title_point))
            time.sleep(.15)

        def click(point):
            self.xd('mousemove', '--window', self.window, *map(int, point))
            self.xd('click', 1)
            park()

        wait_for(modal_read, 'visible export modal controls', 20)
        park()
        formats = {'.jpg': 'JPEG', '.jpeg': 'JPEG', '.png': 'PNG',
                   '.tif': 'TIFF', '.tiff': 'TIFF', '.exr': 'EXR'}
        requested = formats.get(Path(path).suffix.lower())
        require(requested, f'Unsupported acceptance export extension: {path}')
        # Keep these orders aligned with ExportDialog::show in export.rs.
        # CPU precision is build-dependent; only one CPU entry is rendered.
        spaces = (('sRGB', 'ACES2065-1') if requested == 'EXR' else
                  ('sRGB', 'DCI-P3', 'Display P3', 'Adobe RGB (1998)',
                   'ITU-R BT.2020', 'ProPhoto RGB', 'ACES2065-1',
                   'DaVinci Wide Gamut', 'V-Gamut'))
        depths = (('16 bit', '32 bit') if requested == 'EXR' else
                  ('8 bit', '16 bit', '32 bit') if requested == 'TIFF' else ('8 bit',))
        choices = {
            'Backend': ('CPU (f32)', 'CPU (f64)', 'GPU (WGPU f32)'),
            'Format': ('JPEG', 'PNG', 'TIFF', 'EXR'),
            'Bit depth': depths,
            'Saving color space': spaces,
        }
        if saving_color_space is not None:
            require(saving_color_space in spaces,
                    f'{requested} does not offer saving color space {saving_color_space}')

        def control(result, label):
            image, box, lines = result
            rows = []
            anchors = [span for line in lines if (span := matched_span(line, label))]
            for span in anchors:
                label_left = min(word[1] for word in span)
                label_right = max(word[1] + word[3] for word in span)
                padding = max(word[4] for word in span) / 2
                # A closed combo is a single row. Read only its value side so
                # OCR cannot split it from the label or borrow summary prose.
                value_box = (int(box[0] if label == 'Saving color space' else label_right + 1),
                             max(box[1], int(min(word[2] for word in span) - padding)),
                             int(label_left if label == 'Saving color space' else box[2]),
                             min(box[3], int(max(word[2] + word[4] for word in span) + padding)))
                if value_box[0] >= value_box[2] or value_box[1] >= value_box[3]:
                    continue
                # Border-inclusive OCR can mistake TIFF for TRF. Find the
                # closed button's contrasting flat fill within this row,
                # then read its interior instead of guessing text aliases.
                pixels = np.asarray(image)[:, :, :3].astype(np.int16)
                cy = center(span)[1]
                text_height = max(word[4] for word in span)
                buttons = set()
                for y in range(value_box[1], min(value_box[3], int(min(word[2] for word in span)) + 1)):
                    row = pixels[y]
                    edges = np.nonzero(np.max(np.abs(row[1:] - row[:-1]), axis=1) > 3)[0] + 1
                    for left, right in zip(edges[:-1], edges[1:]):
                        if (left < value_box[0] or right > value_box[2]
                                or right - left < text_height * 2
                                or np.max(np.abs(row[left] - row[int(box[2]) - 4])) <= 8):
                            continue
                        background = row[left]
                        edge_x = max(left, right - 3)
                        top, bottom = y, y + 1
                        while top > value_box[1] and np.max(np.abs(pixels[top - 1, edge_x] - background)) <= 3:
                            top -= 1
                        while bottom < value_box[3] and np.max(np.abs(pixels[bottom, edge_x] - background)) <= 3:
                            bottom += 1
                        if top <= cy < bottom and bottom - top >= text_height:
                            buttons.add((int(left), int(top), int(right), int(bottom)))
                if buttons:
                    value_box = min(buttons, key=lambda bounds: (bounds[0], -(bounds[2] - bounds[0])))
                value_lines = modal_lines(image, value_box, 4, 7)
                for line in value_lines:
                    for choice in choices[label]:
                        aliases = (choice, 'GPU (WGPU 32)') if choice == 'GPU (WGPU f32)' else (choice,)
                        selected = next((found for alias in aliases
                                         if (found := matched_span(line, alias))), None)
                        if selected:
                            rows.append((choice, selected))
                # Preserve the fresh row observations in the same-frame record.
                lines.extend(value_lines)
            if rows and len({choice for choice, _ in rows}) == 1:
                return rows[0]
            return None

        def selected(label, expected=None):
            result = modal_read()
            if result is not None:
                value = control(result, label)
                if value is not None and (expected is None or value[0] == expected):
                    return result, value
            return None

        popup_evidence = {}

        def popup_read(before, button, options, target):
            image, _ = self.image()
            if image.size != before.size:
                return None
            # The pointer is parked on the same title in both frames. Opening
            # a combo changes its button and its popup, not the modal layout.
            # Exclude the button row and find the adjacent changed rectangle.
            # This includes the popup's border/shadow even where its fill and
            # the underlying dialog have the same color.
            changed = np.max(np.abs(np.asarray(image).astype(np.int16)
                                    - np.asarray(before).astype(np.int16)), axis=2) > 8
            popup_evidence.update(image=image, changed=changed, candidates=[])
            height = button[3] - button[1]
            for top, bottom, below in ((button[3], image.height, True), (0, button[1], False)):
                ys, xs = np.nonzero(changed[top:bottom])
                if not xs.size:
                    continue
                # Color menus expand to fit their labels/help, unlike the
                # compact format/depth buttons. Keep that observed width;
                # inset the difference bounds only by the rendered shadow.
                inset = max(1, height // 4)
                box = (max(button[0], int(xs.min()) + inset), int(ys.min()) + top,
                       int(xs.max()) + 1 - inset, int(ys.max()) + top + 1)
                gap = box[1] - button[3] if below else button[1] - box[3]
                candidate = {'box': box, 'button': button}
                popup_evidence['candidates'].append(candidate)
                if (gap > height or abs(box[0] - button[0]) > height
                        or box[3] - box[1] > (len(options) + 2) * height * 2):
                    candidate['rejected_geometry'] = True
                    continue
                lines = modal_lines(image, box, 4)
                candidate['ocr'] = [' '.join(word[0] for word in line) for line in lines]
                visible = []
                for line in lines:
                    # A popup's help paragraph mentions several color spaces.
                    # Only a complete option row is a selectable observation.
                    text = ''.join(re.findall(r'[a-z0-9]+', ''.join(w[0] for w in line).lower()))
                    for index, option in enumerate(options):
                        if text == ''.join(re.findall(r'[a-z0-9]+', option.lower())):
                            visible.append((index, center(line)))
                if visible:
                    indices = [index for index, _ in visible]
                    if target in indices or target < min(indices) or target > max(indices):
                        return image, box, lines, visible
            return None

        def select(label, option):
            result, (current, span) = wait_for(lambda: selected(label),
                                             'recognized export control ' + label, 20)
            image, _, lines = result
            self.snap('modal-before-' + label.replace(' ', '-'), image, lines)
            if current != option:
                button = flat_bounds(image, span, button=True)
                require(button is not None, f'Export combo bounds are not visible: {label}')
                options = choices[label]
                target = options.index(option)
                click(center(span))
                # One wheel event per observed menu frame. The source order
                # determines direction, including a reused menu scrolled down.
                for _ in range(2 * len(options) + 2):
                    try:
                        frame, _, menu_lines, visible = wait_for(
                            lambda: popup_read(image, button, options, target),
                            f'observed {label} popup for {option}', 20)
                    except RuntimeError:
                        stem = 'failed-popup-' + label.replace(' ', '-')
                        image.save(self.root / (stem + '-closed.png'))
                        if popup_evidence:
                            popup_evidence['image'].save(self.root / (stem + '-open.png'))
                            self.Image.fromarray(popup_evidence['changed']).save(
                                self.root / (stem + '-mask.png'))
                            self.records.append({'popup_failure': label,
                                                 'button': button,
                                                 'candidates': popup_evidence['candidates']})
                        raise
                    self.snap('export-popup-' + label.replace(' ', '-'), frame, menu_lines)
                    targets = [point for index, point in visible if index == target]
                    if targets:
                        click(targets[0])
                        break
                    upwards = target < min(index for index, _ in visible)
                    _, point = min(visible) if upwards else max(visible)
                    self.xd('mousemove', '--window', self.window, *map(int, point))
                    self.xd('click', 4 if upwards else 5)
                    # Wheel smoothing is asynchronous. Do not read an
                    # intermediate scroll position or leave an option hovered.
                    time.sleep(.3)
                    park()
                else:
                    stem = 'failed-popup-' + label.replace(' ', '-')
                    image.save(self.root / (stem + '-closed.png'))
                    popup_evidence['image'].save(self.root / (stem + '-open.png'))
                    self.Image.fromarray(popup_evidence['changed']).save(self.root / (stem + '-mask.png'))
                    self.records.append({'popup_failure': label, 'button': button,
                                         'candidates': popup_evidence['candidates']})
                    raise RuntimeError(f'Export popup did not reveal {label}: {option}')
                try:
                    result, _ = wait_for(lambda: selected(label, option),
                                         f'closed export control {label} = {option}', 20)
                except RuntimeError:
                    stem = 'failed-selection-' + label.replace(' ', '-')
                    image.save(self.root / (stem + '-before.png'))
                    frame.save(self.root / (stem + '-open.png'))
                    closed, _ = self.image()
                    closed.save(self.root / (stem + '-after.png'))
                    self.Image.fromarray(popup_evidence['changed']).save(self.root / (stem + '-mask.png'))
                    raise
            self.snap('modal-selected-' + label.replace(' ', '-'), result[0], result[2])

        if getattr(self, 'backend_requested', None) == 'wgpu':
            select('Backend', 'GPU (WGPU f32)')
        select('Format', requested)
        depth = ('32 bit' if float_export else '16 bit') if requested in ('TIFF', 'EXR') else '8 bit'
        select('Bit depth', depth)
        if saving_color_space is not None:
            select('Saving color space', saving_color_space)

        def transfer(result):
            states = set()
            for line in result[2]:
                if matched_span(line, 'Summary'):
                    words = {word.lower() for word in re.findall(
                        r'[a-zA-Z]+', ' '.join(word[0] for word in line))}
                    if 'linear' in words:
                        states.add(False)
                    if words & {'cctf', 'encoded'}:
                        states.add(True)
                # Fixed-transfer formats render a full-size explanatory row;
                # the smaller summary can lose letters in "linear" to OCR.
                # Require the actual visible transfer text, not the format alone.
                if requested == 'EXR' and matched_span(line, 'EXR uses linear color and ZIP compression'):
                    states.add(False)
                if requested in ('JPEG', 'PNG') and matched_span(line, 'JPEG and PNG use encoded color'):
                    states.add(True)
            return next(iter(states)) if len(states) == 1 else None

        expected_transfer = (False if requested == 'EXR' else
                             True if requested in ('JPEG', 'PNG') else saving_cctf_encoding)
        if saving_cctf_encoding is not None:
            require(requested == 'TIFF', 'Only TIFF exposes a saving transfer toggle')
            def transfer_control():
                result = modal_read()
                if result is not None and transfer(result) is not None:
                    spans = [span for line in result[2]
                             if (span := matched_span(line, 'Apply saving CCTF encoding'))]
                    if spans:
                        return result, spans[0]
                return None
            result, span = wait_for(transfer_control, 'TIFF saving transfer control and summary', 20)
            if transfer(result) != saving_cctf_encoding:
                click(center(span))

        def submit():
            result = modal_read()
            if result is None:
                return None
            image, box, lines = result
            expected = {'Format': requested, 'Bit depth': depth}
            backend_value = control(result, 'Backend')
            if backend_value is None:
                return None
            expected['Backend'] = backend_value[0]
            if getattr(self, 'backend_requested', None) == 'wgpu':
                expected['Backend'] = 'GPU (WGPU f32)'
            if saving_color_space is not None:
                expected['Saving color space'] = saving_color_space
            for label, option in expected.items():
                value = control(result, label)
                if value is None or value[0] != option:
                    return None
            if expected_transfer is not None and transfer(result) != expected_transfer:
                return None
            cancels = [span for line in lines
                       if (span := matched_span(line, 'Cancel'))
                       and span[0][2] > title_band[3]]
            if not cancels:
                return None
            cancel = min(cancels, key=lambda span: span[0][2])
            cx, cy = center(cancel)
            button = flat_bounds(image, cancel, button=True)
            if button is None:
                return None
            padding = (button[3] - button[1]) / 2
            footer_box = (box[0], button[1], box[2], button[3])
            footer = modal_lines(image, footer_box, 4, 7)
            for line in lines + footer:
                span = matched_span(line, 'Export file')
                if span and center(span)[0] < cx and abs(center(span)[1] - cy) < padding:
                    self.snap('export-options-confirmed', image, lines + footer)
                    self.records[-1]['export_controls'] = expected
                    self.records[-1]['transfer_encoded'] = transfer(result)
                    self.export_backend_requested = ('wgpu' if expected['Backend'].startswith('GPU') else 'cpu')
                    return center(span)
            return None
        x, y = wait_for(submit, 'verified export controls and Export file button', 20)
        self.drain_reports()
        self.xd('mousemove', '--window', self.window, int(x), int(y))
        self.xd('click', 1)


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
    # Explicit contract changes since the retained upstream oracle: removed
    # output-lightness/auto-preview leaves and inherited capture illuminant.
    factory['output_gamut_compress'].pop('lightness_compression')
    factory['simulation'].pop('auto_preview')
    factory['camera']['capture_illuminant'] = None
    rust_factory = json.loads((repo/'crates/spektrafilm-gui/src/factory_state.json').read_text())
    require(parameters_equal({key: value for key, value in rust_factory.items() if key != 'rust'},
                             {key: value for key, value in factory.items() if key != 'rust'}),
            'Rust shared factory differs from independent upstream oracle')
    seed = json.loads(json.dumps(factory))
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
        for index, route in enumerate(ROUTES):
            driver.drain_reports()
            driver.choose_route(index)
            state_path = root/f'route-{index}.json'
            state = driver.state_action('Save current to file', state_path, save=True)
            require(state['simulation']['route'] == route,
                    f'Workflow selection {index} saved {state["simulation"].get("route")}')
            require('scan_film' not in state['simulation'], 'Legacy scan_film persisted')
            require('workflow' not in state['simulation'], 'Legacy nested workflow persisted')
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
        raw_preview = driver.open_image(args.raw.resolve(), 'imported-raw')
        require(raw_preview['configuration']['working_dimensions'] == [768, 512],
                'RAW import did not retain the full source dimensions')
        driver.records.append({'raw_fixture_sha256': sha(args.raw),
                               'raw_input_size': [768, 512], 'automatic_preview_report': raw_preview})
        driver.reveal('extras.load_raw.white_balance', 'Input and capture', 'Import Raw')
        driver.choose('white balance', 'tungsten')
        driver.wait_text(r'RAW decode settings pending', 'raw-settings-pending')
        driver.drain_reports()
        driver.click('Reprocess RAW', right=True)
        reprocessed = driver.wait_report('raw-reprocessed', 'Preview')
        require(reprocessed['configuration']['working_dimensions'] == [768, 512],
                'RAW reprocessing did not retain the full source dimensions')
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
