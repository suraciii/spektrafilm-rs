#!/usr/bin/env python3
"""Installed egui acceptance through native desktop input, file dialogs and OCR.

Linux: Xvfb/openbox/xdotool/xprop/xwininfo/xclip/zenity/tesseract; Windows/macOS: pyautogui
and Tesseract. macOS requires runner Accessibility/Automation/Screen Recording
permissions and an active desktop. No GUI hooks, exporter override, source-tree
executable or fabricated child is used.
"""
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import threading

import numpy as np
import OpenImageIO as oiio


def require(value, message):
    if not value:
        raise RuntimeError(message)
def render_count(text):
    matches = re.findall(r'Rendered\s+(\d+)', text, re.I)
    require(matches, f'Native GUI did not report a render count: {text}')
    return int(matches[-1])




def wait_for(predicate, description, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.15)
    raise RuntimeError(f'Timed out waiting for {description} ({timeout}s)')

class _FfmpegMss:
    """Small Linux screenshot fallback when the optional mss wheel is absent."""
    @classmethod
    def mss(cls):
        return cls()
    @property
    def monitors(self):
        info = subprocess.run(['xwininfo', '-root'], capture_output=True,
                              text=True, check=True, timeout=10).stdout
        width = int(re.search(r'Width:\s+(\d+)', info).group(1))
        height = int(re.search(r'Height:\s+(\d+)', info).group(1))
        return [{'left': 0, 'top': 0, 'width': width, 'height': height}]
    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def grab(self, bbox):
        command = [
            'ffmpeg', '-loglevel', 'error', '-f', 'x11grab',
            '-video_size', f"{bbox['width']}x{bbox['height']}",
            '-i', f"{os.environ['DISPLAY']}+{bbox['left']},{bbox['top']}",
            '-frames:v', '1', '-f', 'image2pipe', '-vcodec', 'png', '-',
        ]
        result = subprocess.run(command, check=True, capture_output=True, timeout=15)
        from PIL import Image
        with Image.open(io.BytesIO(result.stdout)) as image:
            image = image.convert('RGB')
            return type('ScreenShot', (), {
                'size': image.size,
                'rgb': image.tobytes(),
            })()


class _Tesseract:
    class Output:
        DICT = 'dict'

    @staticmethod
    def image_to_data(image, config='', output_type=None, timeout=15):
        png = io.BytesIO()
        image.save(png, format='PNG')
        result = subprocess.run(
            ['tesseract', 'stdin', 'stdout', '--psm', config.split()[-1], 'tsv'],
            input=png.getvalue(), capture_output=True, timeout=timeout, check=True,
        )
        rows = result.stdout.decode('utf-8', errors='replace').splitlines()
        fields = ('level', 'page_num', 'block_num', 'par_num', 'line_num',
                  'word_num', 'left', 'top', 'width', 'height', 'conf', 'text')
        values = {field: [] for field in fields}
        for row in rows[1:]:
            columns = row.split('\t')
            if len(columns) != len(fields):
                continue
            for field, value in zip(fields, columns):
                values[field].append(value if field == 'text' else int(float(value)))
        return values


def _desktop_capture():
    try:
        import mss
        return mss
    except ModuleNotFoundError:
        return _FfmpegMss


def _desktop_ocr():
    try:
        import pytesseract
        return pytesseract
    except ModuleNotFoundError:
        return _Tesseract



class X11:
    def __init__(self, root):
        require(sys.platform.startswith('linux'),
                f'Native GUI acceptance driver unavailable for {sys.platform}; this gate cannot be skipped')
        require(os.environ.get('DISPLAY'), 'GUI acceptance requires X11; run under xvfb-run')
        for command in ('xdotool', 'xprop', 'xwininfo', 'xclip', 'openbox', 'zenity', 'tesseract', 'ffmpeg'):
            require(shutil.which(command), f'GUI acceptance requires native dependency: {command}')
        import psutil
        from PIL import Image
        self.mss, self.psutil, self.ocr, self.Image = _desktop_capture(), psutil, _desktop_ocr(), Image
        self.root, self.window, self.proc = root, None, None
        self.records, self.children, self.wm = [], {}, None
        self.log = (root / 'gui.log').open('w')
        wm = subprocess.run(['xprop', '-root', '_NET_SUPPORTING_WM_CHECK'], capture_output=True, text=True)
        if 'window id #' not in wm.stdout:
            self.wm = subprocess.Popen(['openbox', '--sm-disable'], stdout=self.log, stderr=self.log)
            wait_for(lambda: 'window id #' in subprocess.run(
                ['xprop', '-root', '_NET_SUPPORTING_WM_CHECK'], capture_output=True, text=True).stdout,
                'X11 window manager', 15)

    def xd(self, *args, check=True):
        if args[0] == 'click' and int(args[-1]) == 1 and '--repeat' not in args:
            subprocess.run(['xdotool', 'mousedown', '1'], check=True, timeout=15)
            time.sleep(.1)
            subprocess.run(['xdotool', 'mouseup', '1'], check=True, timeout=15)
            return ''
        result = subprocess.run(['xdotool', *map(str, args)], check=check,
                                capture_output=True, text=True, timeout=15).stdout.strip()
        if args[0] == 'mousemove':
            time.sleep(.1)
        return result

    def start(self, gui, env, image=None):
        command = [str(gui)] + ([str(image)] if image else [])
        self.proc = subprocess.Popen(command, cwd=self.root, env=env, stdout=self.log, stderr=self.log)
        def window():
            require(self.proc.poll() is None, 'Installed GUI exited; see gui.log')
            ids = self.xd('search', '--onlyvisible', '--pid', self.proc.pid, '--name', '^spektrafilm$', check=False)
            return ids.splitlines()[-1] if ids else None
        self.window = wait_for(window, 'installed native window', 45)
        self.xd('windowactivate', '--sync', self.window)
        with self.mss.mss() as screen:
            desktop = screen.monitors[0]
        require(desktop['height'] >= 1000, 'Native GUI probe requires a desktop at least 1000px high')
        self.xd('windowsize', '--sync', self.window, 1460, 980)
        self.xd('windowmove', '--sync', self.window, 0, 0)
        wait_for(lambda: self.match(self.read()[2], 'MAIN', True)
                 and self.match(self.read()[2], 'Open', True), 'rendered native MAIN controls', 30)

    def image(self):
        require(self.proc.poll() is None, 'Installed GUI exited; see gui.log')
        # xdotool getwindowgeometry adds the reparented frame offset twice
        # under Openbox. xwininfo reports the true root-relative client origin.
        info = subprocess.run(['xwininfo', '-id', self.window], check=True,
                              capture_output=True, text=True, timeout=10).stdout
        def field(name):
            return int(re.search(rf'{name}:\s+(-?\d+)', info).group(1))
        bbox = dict(left=field('Absolute upper-left X'), top=field('Absolute upper-left Y'),
                    width=field('Width'), height=field('Height'))
        with self.mss.mss() as screen:
            shot = screen.grab(bbox)
            return self.Image.frombytes('RGB', shot.size, shot.rgb), bbox

    def read(self):
        image, bbox = self.image()
        # The sidebar occupies the right portion of the client, but its
        # first control can move left with native font/theme metrics.
        # Use a proportional boundary rather than a fixed pixel coordinate.
        self.right_control_x = image.width * 0.65
        from PIL import ImageOps, ImageStat
        lines = []
        # Segment the sidebar from the viewer and normalize each region to
        # dark text on a light background, including native light themes.
        def normalize(region):
            grayscale = ImageOps.grayscale(region)
            background = grayscale.crop((0, 0, grayscale.width, min(50, grayscale.height)))
            return ImageOps.invert(grayscale) if ImageStat.Stat(background).median[0] < 128 else grayscale

        for offset, region in ((0, image.crop((0, 0, image.width - 420, image.height))),
                               (image.width - 420, image.crop((image.width - 420, 0, image.width, image.height)))):
            prepared = normalize(region)
            data = self.ocr.image_to_data(prepared.resize((region.width * 3, region.height * 3)),
                                          config='--psm 11', output_type=self.ocr.Output.DICT, timeout=15)
            grouped = {}
            for i, text in enumerate(data['text']):
                if text.strip():
                    key = (data['block_num'][i], data['par_num'][i], data['line_num'][i])
                    grouped.setdefault(key, []).append((text, offset + data['left'][i] / 3, data['top'][i] / 3,
                                                         data['width'][i] / 3, data['height'][i] / 3))
            lines.extend(grouped.values())
        # Read the full footer independently: zoom/rotation and job status now
        # live below the canvas rather than at the bottom of the sidebar.
        top = image.height - 50
        region = image.crop((0, top, image.width, image.height))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 3, region.height * 3)),
                                      config='--psm 6', output_type=self.ocr.Output.DICT, timeout=15)
        grouped = {}
        for i, text in enumerate(data['text']):
            if text.strip():
                key = (data['block_num'][i], data['par_num'][i], data['line_num'][i])
                grouped.setdefault(key, []).append((text, data['left'][i] / 3, top + data['top'][i] / 3,
                                                    data['width'][i] / 3, data['height'][i] / 3))
        lines.extend(grouped.values())
        region = image.crop((370, image.height - 30, image.width - 420, image.height))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 4, region.height * 4)),
                                      config='--psm 7', output_type=self.ocr.Output.DICT, timeout=15)
        words = [(text, 370 + data['left'][i] / 4, image.height - 30 + data['top'][i] / 4,
                  data['width'][i] / 4, data['height'][i] / 4)
                 for i, text in enumerate(data['text']) if text.strip()]
        lines.append(words)
        return image, bbox, lines

    def match(self, lines, label, right=False):
        # OCR may transliterate the ellipsis; match words, with explicit boundaries.
        wanted = re.findall(r'[a-z0-9]+', label.lower())
        aliases = {
            'main': {'main', 'mb', 'iain', 'jain', 'nn', 'n'},
            'open': {'open', 'pen', 'per', 'pe'},
            'save': {'save', 'ave', 'saye'},
            'config': {'config', 'confic'},
        }
        matches = []
        for line in lines:
            for start in range(len(line)):
                words = []
                selected = []
                for word in line[start:]:
                    selected.append(word)
                    words.extend(re.findall(r'[a-z0-9]+', word[0].lower()))
                    if len(words) >= len(wanted):
                        break
                # Toggled controls render their state inside the label, e.g.
                # "Scan-for-print:ON"; accept that single-token suffix.
                suffixed = (len(selected) == 1 and len(words) == len(wanted) + 1
                            and words[:-1] == wanted and words[-1] == 'on')
                equivalent = words == wanted or suffixed or (
                    len(words) == len(wanted)
                    and all(word == expected or word in aliases.get(expected, set())
                            for word, expected in zip(words, wanted))
                )
                if equivalent or (label.lower() == 'auto exposure' and words == ['aueo', 'exposure']) or (label.lower() == '16 bit' and words == ['16', 'bir']):
                    x = selected[0][1]
                    if right and x < self.right_control_x:
                        continue
                    if label.lower() == 'save' and selected[0][2] < 100:
                        continue
                    # Save must not select Save state or Save startup.
                    # Preview must select the footer action button, not the
                    # "Preview workflow" header or the "preview pack:" stat.
                    following = line[start + len(selected):start + len(selected) + 1]
                    if label.lower() == 'save' and following and following[0][0].lower() in ('state', 'startup'):
                        continue
                    if label.lower() == 'preview':
                        line_words = [w for word in line for w in re.findall(r'[a-z0-9]+', word[0].lower())]
                        if 'scan' not in line_words:
                            continue
                    matches.append((x + sum(w[3] for w in selected) / 2,
                                    selected[0][2] + selected[0][4] / 2))
        return matches

    def snap(self, label, image=None, lines=None):
        if image is None:
            image, _, lines = self.read()
        path = self.root / f'{len(self.records):02d}-{label}.png'
        image.save(path)
        text = '\n'.join(' '.join(word[0] for word in line) for line in (lines or []))
        self.records.append({'surface': label, 'screenshot': path.name, 'ocr': text})
        return text

    def click(self, label, right=True):
        # Buttons in the MAIN sidebar below collapsible sections (Open…,
        # Save…, Export…, 16/32-bit depth) and the footer Preview/Scan
        # cluster shift position with loaded state and status text, so they
        # are located from rendered text.
        fixed = {
            'Input': (26, 16),
            'Output': (78, 16),
            'Paper back': (145, 16),
            'Reveal': (1055, 180),
            'Crossfade': (1117, 180),
            'Save state': (1085, 70),
            'Load state': (1165, 70),
            'Save startup default': (1275, 70),
            'Restore factory default': (1120, 92),
            'ccw rotate': (42, 963),
            'cw rotate': (114, 963),
            '100%': (174, 963),
            '200%': (221, 963),
            '400%': (268, 963),
            'reset view': (328, 963),
        }
        if label in fixed:
            x, y = fixed[label]
            if sys.platform == 'darwin' and y < 300:
                # AX window bounds include the 30px macOS title bar; the
                # fixed points above are content-relative Linux coordinates.
                y += 30
            if label in ('ccw rotate', 'cw rotate', '100%', '200%', '400%', 'reset view'):
                image, _ = self.image()
                y = image.height - 17
            self.xd('mousemove', '--window', self.window, x, y)
            self.xd('click', 1)
            time.sleep(.8 if label.endswith('%') or label in ('Input', 'Output', 'Paper back', 'Reveal', 'Crossfade') else .15)
            return
        def locate():
            image, _, lines = self.read()
            matches = self.match(lines, label, right)
            if not matches and label == 'Cancel':
                # During export, GTK renders Cancel where Export was. OCR
                # intermittently misses this short label; Save remains the
                # adjacent, same-row anchor.
                anchor = self.match(lines, 'Save', True)
                if anchor:
                    matches = [(anchor[0][0] + 57, anchor[0][1])]
            if matches:
                self.snap('control-' + label.replace(' ', '-'), image, lines)
                return matches[0]
        x, y = wait_for(locate, f'visible control {label}', 20)
        if label == 'Cancel':
            self.require_export_in_flight('Cancel')
        # Toggled controls can hide their label (checkbox glyph changes OCR);
        # remember the position so the same control can be re-clicked.
        self.last_control = (x, y)
        self.xd('mousemove', '--window', self.window, int(x), int(y))
        self.xd('click', 1)
        if label in {'MAIN', 'CONFIG', 'FILM', 'PRINT', 'ADVANCED'}:
            self.current_tab = label
        time.sleep(.15)

    def reclick_last_control(self):
        """Toggle-back helper for controls whose label OCR cannot re-find."""
        x, y = self.last_control
        self.xd('mousemove', '--window', self.window, int(x), int(y))
        self.xd('click', 1)
        time.sleep(.8)

    def tab(self, name):
        positions = {'MAIN': 1070, 'FILM': 1110, 'PRINT': 1160, 'ADVANCED': 1230, 'CONFIG': 1295}
        require(name in positions, f'Unknown GUI tab: {name}')
        y = 40 + (30 if sys.platform == 'darwin' else 0)
        self.xd('mousemove', '--window', self.window, positions[name], y)
        self.xd('click', 1)
        time.sleep(.8)
        self.scroll(False)

    def scroll(self, bottom):
        image, _, _ = self.read()
        require(image.width >= 1460, 'Native GUI acceptance requires the pinned 1460px window width')
        # The sidebar is a fixed 420px panel on the right. Move the wheel
        # inside that measured panel instead of depending on OCR for a tab.
        sidebar_x = image.width - 180
        self.xd('mousemove', '--window', self.window, int(sidebar_x), int(image.height / 2))
        self.xd('click', '--repeat', 35, '--delay', 8, 5 if bottom else 4)
        time.sleep(.15)

    def observe_tabs(self, label):
        for name in ('MAIN', 'FILM', 'PRINT', 'ADVANCED', 'CONFIG'):
            self.tab(name)
            for bottom in (False, True):
                self.scroll(bottom)
                image, _, lines = self.read()
                for control in ('Preview', 'Scan'):
                    require(self.match(lines, control, True),
                            f'{control} not visible in {name} at {"bottom" if bottom else "top"}')
                self.snap(f'{label}-{name}-{"bottom" if bottom else "top"}', image, lines)
        self.tab('MAIN')
    def measure_viewer(self, label):
        """Record displayed image bounds from screenshot pixels, not OCR."""
        image, _, lines = self.read()
        pixels = np.asarray(image.convert('RGB'))
        canvas = pixels[80:max(81, image.height - 80), :min(1030, image.width)]
        channels = [canvas[..., i].astype(np.int16) for i in range(3)]
        chroma = np.maximum.reduce(channels) - np.minimum.reduce(channels)
        mask = (chroma > 25) & (np.maximum.reduce(channels) > 35)
        ys, xs = np.where(mask)
        require(xs.size > 100, f'No rendered viewer pixels found in screenshot for {label}')
        bounds = [int(xs.min()), int(ys.min() + 80), int(xs.max() + 1), int(ys.max() + 81)]
        screenshot = f'{len(self.records):02d}-{label}.png'
        image.save(self.root / screenshot)
        self.records.append({'surface': label, 'screenshot': screenshot,
                             'pixel_viewer_bounds': bounds})
        return bounds

    def wait_text(self, pattern, label, timeout=120):
        def ready():
            image, _, lines = self.read()
            text = '\n'.join(' '.join(w[0] for w in line) for line in lines)
            require(not re.search(r'(?:Render|Load|Save|Export|Startup state|Preview state)\s*(?:error|failed)', text, re.I),
                    f'GUI failure on {label}: {text}')
            if re.search(pattern, text, re.I):
                self.snap(label, image, lines)
                return text
        return wait_for(ready, label, timeout)

    def dialog(self, path, save=False):
        dialog_classes = ('zenity', 'yad', 'xdg-desktop-portal-gtk')
        def find():
            ids = []
            for dialog_class in dialog_classes:
                found = self.xd('search', '--onlyvisible', '--class', dialog_class, check=False)
                ids.extend(found.splitlines())
            if not ids:
                return None
            active = self.xd('getactivewindow', check=False)
            return active if active in ids else ids[-1]
        dialog = wait_for(find, 'native file chooser (zenity/yad/portal)', 25)
        for child in self.psutil.Process(self.proc.pid).children(recursive=True):
            try:
                if 'zenity' in child.name():
                    self.records.append({'native_chooser_argv': child.cmdline()})
            except self.psutil.NoSuchProcess:
                pass
        self.xd('windowsize', '--sync', dialog, 900, 650)
        self.xd('windowmove', '--sync', dialog, 100, 100)
        # Native chooser can exceed the Xvfb desktop; capture the visible screen.
        with self.mss.mss() as screen:
            shot = screen.grab(screen.monitors[0])
            chooser = self.Image.frombytes('RGB', shot.size, shot.rgb)
        chooser_path = self.root / f'{len(self.records):02d}-native-file-chooser.png'
        chooser.save(chooser_path)
        self.records.append({'surface': 'native-file-chooser', 'screenshot': chooser_path.name,
                             'requested_path': str(path), 'save': save})

        def paste_location(value):
            nonlocal dialog
            dialog = wait_for(find, 'active native file chooser', 15)
            self.xd('windowactivate', '--sync', dialog)
            time.sleep(.3)
            self.xd('key', 'ctrl+l')
            time.sleep(.2)
            self.xd('key', 'ctrl+a')
            subprocess.run(['xclip', '-selection', 'clipboard'], input=value,
                           text=True, check=True, timeout=10)
            self.xd('key', 'ctrl+v')
            time.sleep(.5)
            self.xd('key', 'Return')

        if save:
            paste_location(str(path))
        else:
            # GTK portal choosers navigate to a directory before accepting a
            # file path.  Make that transition explicit so Return cannot act
            # on the previous selection or fall through to the main window.
            paste_location(str(path.parent) + '/')
            time.sleep(.8)
            paste_location(str(path))

        # Save/open choosers may leave the selected path visible while waiting
        # for the final action button.  Click only the active chooser's button.
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            dialog = find()
            if not dialog:
                return
            with self.mss.mss() as screen:
                shot = screen.grab(screen.monitors[0])
            surface = self.Image.frombytes('RGB', shot.size, shot.rgb)
            buttons = self.ocr.image_to_data(surface, config='--psm 11', output_type=self.ocr.Output.DICT)
            clicked = False
            for i, word in enumerate(buttons['text']):
                if word.strip() in ('OK', 'Open', 'Select', 'Save'):
                    self.xd('windowactivate', '--sync', dialog)
                    self.xd('mousemove', buttons['left'][i] + buttons['width'][i] // 2,
                            buttons['top'][i] + buttons['height'][i] // 2)
                    self.xd('click', 1)
                    clicked = True
                    break
            if not clicked:
                self.xd('key', 'Return')
            time.sleep(.25)
        raise RuntimeError(f'Native chooser did not accept {path}')

    def file_action(self, control, path, save=False):
        self.tab('CONFIG' if control in ('Save state', 'Load state', 'Save startup default',
                                         'Restore factory default') else 'MAIN')
        if control == 'Export':
            self.watch_export()
        self.click(control)
        self.dialog(path, save)

    def rendered(self, label, previous_count=None):
        self.scroll(True)
        def ready():
            image, _, lines = self.read()
            text = '\n'.join(' '.join(w[0] for w in line) for line in lines)
            require(not re.search(r'(?:Render|Load|Save|Export|Startup state|Preview state)\s*(?:error|failed)', text, re.I),
                    f'GUI failure on {label}: {text}')
            if re.search(r'Rendered\s+\d+', text, re.I):
                if previous_count is None or render_count(text) != previous_count:
                    self.snap(label, image, lines)
                    return text
        return wait_for(ready, label, 120)

    def watch_export(self):
        self.observed_child = threading.Event()
        self.watch_error = None
        self.watch_stop = threading.Event()
        def watch():
            try:
                deadline = time.monotonic() + 60
                while not self.watch_stop.is_set() and time.monotonic() < deadline:
                    for child in self.psutil.Process(self.proc.pid).children(recursive=True):
                        try:
                            command = child.cmdline()
                            if 'process' not in command or child.pid in self.children:
                                continue
                            executable = Path(child.exe())
                            created = child.create_time()
                            self.children[child.pid] = created
                            self.current_export_child = (child.pid, created)
                            actual_hash = hashlib.sha256(executable.read_bytes()).hexdigest()
                            require(actual_hash == self.exporter_hash,
                                    f'GUI discovered exporter outside delivered package: {executable}')
                            self.records.append({'export_child_pid': child.pid, 'executable': str(executable),
                                                 'sha256': actual_hash, 'argv': command})
                            staged_input = Path(command[command.index('process') + 1])
                            if staged_input.name.startswith('spektrafilm-export-input-'):
                                self.staged_export_input = read_image(staged_input)
                            self.observed_child.set()
                            return
                        except (self.psutil.NoSuchProcess, self.psutil.ZombieProcess):
                            continue
                    self.watch_stop.wait(.01)
            except Exception as error:
                self.watch_error = error
                self.observed_child.set()
        self.watcher = threading.Thread(target=watch, daemon=True)
        self.watcher.start()

    def capture_export_child(self, exporter):
        require(self.observed_child.wait(20), 'Actual bundled f64 export child was not observed')
        self.watch_stop.set()
        self.watcher.join(timeout=5)
        if self.watch_error:
            raise self.watch_error

    def require_export_in_flight(self, action):
        pid, created = self.current_export_child
        try:
            child = self.psutil.Process(pid)
            alive = child.create_time() == created and child.is_running() and child.status() != self.psutil.STATUS_ZOMBIE
        except self.psutil.NoSuchProcess:
            alive = False
        require(alive, f'{action} did not target an actual in-flight export child: {pid}')
        self.records.append({'in_flight_export_action': action, 'export_child_pid': pid})

    def no_children(self):
        def gone():
            for pid, created in self.children.items():
                try:
                    child = self.psutil.Process(pid)
                    if child.create_time() == created:
                        return False
                except self.psutil.NoSuchProcess:
                    pass
            return True
        wait_for(gone, 'export children reaped', 15)
        self.records.append({'export_children_reaped': list(self.children)})

    def close(self, exporting=False):
        self.xd('windowactivate', '--sync', self.window)
        if exporting:
            self.require_export_in_flight('close')
        self.xd('key', 'alt+F4')
        self.proc.wait(timeout=30)
        require(self.proc.returncode == 0, f'GUI closed with status {self.proc.returncode}')
        self.no_children()
        self.proc = None

    def finish(self):
        if hasattr(self, 'watch_stop'):
            self.watch_stop.set()
            self.watcher.join(timeout=5)
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=10)
        # Failed acceptance must also leave no package subprocesses running.
        for pid, created in self.children.items():
            try:
                child = self.psutil.Process(pid)
                if child.create_time() == created:
                    child.kill()
                    child.wait(timeout=10)
            except self.psutil.NoSuchProcess:
                pass
        if self.wm:
            self.wm.terminate()
            self.wm.wait(timeout=10)
        self.log.close()
        (self.root / 'observations.json').write_text(json.dumps(self.records, indent=2) + '\n')


class Desktop(X11):
    """Native Windows/macOS desktop automation with the shared screen OCR."""
    def __init__(self, root):
        require(sys.platform in ('win32', 'darwin'), f'Unsupported desktop: {sys.platform}')
        import psutil
        import pytesseract
        import pyautogui
        from PIL import Image
        self.psutil, self.ocr, self.Image, self.input = psutil, pytesseract, Image, pyautogui
        self.root, self.window, self.proc = root, None, None
        self.records, self.children, self.wm = [], {}, None
        self.log = (root / 'gui.log').open('w')
        executable = shutil.which('tesseract')
        if sys.platform == 'win32':
            import ctypes
            from ctypes import wintypes
            self.ctypes, self.types, self.os = ctypes, wintypes, ctypes.windll.user32
            self.os.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
            self.os.SetCursorPos.restype = wintypes.BOOL
            class MOUSEINPUT(ctypes.Structure):
                _fields_ = [('dx', ctypes.c_long), ('dy', ctypes.c_long),
                            ('mouseData', ctypes.c_ulong), ('dwFlags', ctypes.c_ulong),
                            ('time', ctypes.c_ulong), ('dwExtraInfo', ctypes.c_size_t)]
            class INPUTUNION(ctypes.Union):
                _fields_ = [('mi', MOUSEINPUT)]
            class INPUT(ctypes.Structure):
                _anonymous_ = ('input',)
                _fields_ = [('type', wintypes.DWORD), ('input', INPUTUNION)]
            self.input_type = INPUT
            self.mouse_input = MOUSEINPUT
            self.os.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
            self.os.SendInput.restype = wintypes.UINT
            self.mouse_wheel_flag = 0x0800
            # HWND is pointer-sized; ctypes' default int conversion truncates
            # handles on 64-bit Windows without explicit argument signatures.
            for name in ('ShowWindow', 'SetForegroundWindow', 'GetWindowTextW',
                         'GetClassNameW', 'GetClientRect', 'ClientToScreen',
                         'GetWindowThreadProcessId', 'GetWindowLongW', 'MoveWindow'):
                function = getattr(self.os, name)
                function.argtypes = [wintypes.HWND] + {
                    'ShowWindow': [ctypes.c_int], 'SetForegroundWindow': [],
                    'GetWindowTextW': [wintypes.LPWSTR, ctypes.c_int],
                    'GetClassNameW': [wintypes.LPWSTR, ctypes.c_int],
                    'GetClientRect': [ctypes.POINTER(wintypes.RECT)],
                    'ClientToScreen': [ctypes.POINTER(wintypes.POINT)],
                    'GetWindowThreadProcessId': [ctypes.POINTER(wintypes.DWORD)],
                    'GetWindowLongW': [ctypes.c_int],
                    'MoveWindow': [ctypes.c_int] * 4 + [wintypes.BOOL],
                }[name]
            self.os.SetProcessDPIAware()
            candidate = Path(os.environ.get('ProgramFiles', r'C:\Program Files')) / 'Tesseract-OCR/tesseract.exe'
            executable = executable or (str(candidate) if candidate.is_file() else None)
        require(executable, 'Tesseract OCR executable must be installed')
        self.ocr.pytesseract.tesseract_cmd = executable
        self.original_display = None
        screen = self.input.size()
        if sys.platform == 'win32' and (screen.width < 1500 or screen.height < 1100):
            self.prepare_display()
            screen = self.input.size()
        require(screen.width >= 1500 and screen.height >= 1100,
                f'Native acceptance needs a desktop at least 1500x1100; actual {screen.width}x{screen.height}. '
                'Configure the runner display before invoking package smoke.')

    def prepare_display(self):
        """Select a supported temporary display mode without changing the registry."""
        import struct
        # DEVMODEW is 220 bytes; size, pixel width and height have ABI offsets
        # 68, 172 and 176 on both supported Windows architectures.
        self.os.EnumDisplaySettingsW.argtypes = [self.types.LPCWSTR, self.types.DWORD, self.ctypes.c_void_p]
        self.os.ChangeDisplaySettingsW.argtypes = [self.ctypes.c_void_p, self.types.DWORD]
        def mode(index):
            buffer = self.ctypes.create_string_buffer(220)
            struct.pack_into('<H', buffer, 68, 220)
            return buffer if self.os.EnumDisplaySettingsW(None, index, buffer) else None
        original = mode(0xFFFFFFFF)
        require(original is not None, 'Cannot inspect Windows desktop display mode')
        candidates = []
        index = 0
        while True:
            candidate = mode(index)
            if candidate is None:
                break
            width, height = struct.unpack_from('<II', candidate, 172)
            if width >= 1500 and height >= 1100:
                candidates.append((width * height, candidate))
            index += 1
        require(candidates, 'Windows display exposes no supported mode at least 1500x1100')
        selected = min(candidates, key=lambda item: item[0])[1]
        require(self.os.ChangeDisplaySettingsW(selected, 4) == 0,
                'Windows refused the temporary desktop display resolution')
        self.original_display = original
        time.sleep(.5)

    def finish(self):
        try:
            super().finish()
        finally:
            if self.original_display is not None:
                require(self.os.ChangeDisplaySettingsW(self.original_display, 0) == 0,
                        'Failed restoring Windows desktop display resolution')
                self.original_display = None

    def apple(self, body):
        script = f'tell application "System Events" to tell (first application process whose unix id is {self.proc.pid})\n{body}\nend tell'
        result = subprocess.run(['osascript', '-e', script], capture_output=True, text=True, timeout=15)
        if result.returncode:
            self.records.append({'macos_applescript_error': result.stderr.strip(),
                                 'applescript': body, 'stdout': result.stdout.strip()})
        require(result.returncode == 0,
                f'macOS accessibility command failed ({result.returncode}): {result.stderr.strip()}')
        return result.stdout.strip()

    def mac_diagnostics(self, stage):
        """Keep native AX evidence even when window cropping or OCR fails."""
        record = {'macos_accessibility_stage': stage}
        try:
            record['accessibility'] = self.apple('''set report to ""
repeat with w in windows
    set w to contents of w
    set report to report & "WINDOW " & (name of w as text) & " | " & (subrole of w as text) & " | position=" & (position of w as text) & " | size=" & (size of w as text) & linefeed
    set elements to (get entire contents of w)
    repeat with elementReference in elements
        set element to contents of elementReference
        try
            set report to report & (role of element as text) & " | " & (description of element as text)
            try
                set report to report & " | name=" & (name of element as text)
            end try
            try
                set report to report & " | value=" & (value of element as text)
            end try
            try
                set report to report & " | identifier=" & (value of attribute "AXIdentifier" of element as text)
            end try
            set report to report & linefeed
        end try
    end repeat
end repeat
return report''')
        except Exception as error:
            record['accessibility_error'] = str(error)
        try:
            target = self.root / f'{len(self.records):02d}-macos-{stage}.png'
            self.input.screenshot().save(target)
            record['screenshot'] = target.name
        except Exception as error:
            record['screenshot_error'] = str(error)
        self.records.append(record)

    def windows(self):
        found = []
        callback_type = self.ctypes.WINFUNCTYPE(self.types.BOOL, self.types.HWND, self.types.LPARAM)
        def visit(hwnd, _):
            pid = self.types.DWORD()
            self.os.GetWindowThreadProcessId(hwnd, self.ctypes.byref(pid))
            if pid.value == self.proc.pid and self.os.IsWindowVisible(hwnd):
                found.append(hwnd)
            return True
        self.os.EnumWindows(callback_type(visit), 0)
        return found

    def focus(self):
        if sys.platform == 'win32':
            self.os.ShowWindow(self.window, 9)
            self.os.SetForegroundWindow(self.window)
        else:
            self.apple('set frontmost to true')

    def start(self, gui, env, image=None):
        self.proc = subprocess.Popen([str(gui)] + ([str(image)] if image else []), cwd=self.root,
                                     env=env, stdout=self.log, stderr=self.log)
        def ready():
            require(self.proc.poll() is None, 'GUI exited; see gui.log')
            if sys.platform == 'darwin':
                return 1 if self.apple('get exists (first window whose name is "spektrafilm")') == 'true' else None
            for hwnd in self.windows():
                title = self.ctypes.create_unicode_buffer(256)
                self.os.GetWindowTextW(hwnd, title, 256)
                if title.value.lower() == 'spektrafilm':
                    return hwnd
        self.window = wait_for(ready, 'native desktop window', 45)
        self.focus()
        if sys.platform == 'darwin':
            self.apple('set w to first window whose name is "spektrafilm"\nset position of w to {0, 30}\nset size of w to {1460, 980}')
        else:
            rect = self.types.RECT(0, 0, 1460, 980)
            self.os.AdjustWindowRect(self.ctypes.byref(rect), self.os.GetWindowLongW(self.window, -16), False)
            self.os.MoveWindow(self.window, 0, 30, rect.right - rect.left, rect.bottom - rect.top, True)
        wait_for(lambda: self.match(self.read()[2], 'MAIN', True)
                 and self.match(self.read()[2], 'Open', True), 'rendered desktop MAIN controls', 45)

    def bounds(self):
        if sys.platform == 'darwin':
            # The modal rfd panel becomes window 1. Keep OCR and coordinates on
            # the application window, and serialize AX pairs explicitly rather
            # than relying on AppleScript's nested-list coercion.
            raw = self.apple('''set w to first window whose name is "spektrafilm"
set p to position of w
set s to size of w
return (item 1 of p as integer as text) & "," & (item 2 of p as integer as text) & "," & (item 1 of s as integer as text) & "," & (item 2 of s as integer as text)''')
            values = [int(v) for v in raw.split(',')]
            if len(values) != 4 or values[2] <= 0 or values[3] <= 0:
                self.mac_diagnostics('invalid-bounds')
                raise RuntimeError(f'Unexpected macOS window bounds: {raw}')
            return values
        rect, point = self.types.RECT(), self.types.POINT(0, 0)
        self.os.GetClientRect(self.window, self.ctypes.byref(rect))
        self.os.ClientToScreen(self.window, self.ctypes.byref(point))
        return point.x, point.y, rect.right, rect.bottom

    def image(self):
        require(self.proc.poll() is None, 'GUI exited; see gui.log')
        try:
            x, y, width, height = self.bounds()
        except Exception:
            if sys.platform == 'darwin':
                self.mac_diagnostics('window-bounds-failure')
            raise
        shot = self.input.screenshot()
        sw, sh = self.input.size()
        sx, sy = shot.width / sw, shot.height / sh
        if width <= 0 or height <= 0:
            if sys.platform == 'darwin':
                self.mac_diagnostics('invalid-image-bounds')
            raise RuntimeError(f'Invalid native screenshot bounds: {(x, y, width, height)}')
        cropped = shot.crop((int(x * sx), int(y * sy), int((x + width) * sx), int((y + height) * sy)))
        return cropped.resize((width, height)), dict(left=x, top=y, width=width, height=height)

    def send_wheel(self, delta):
        event = self.input_type()
        event.type = 0  # INPUT_MOUSE
        event.mi = self.mouse_input(mouseData=delta, dwFlags=self.mouse_wheel_flag)
        require(self.os.SendInput(1, self.ctypes.byref(event), self.ctypes.sizeof(event)) == 1,
                'Windows rejected native mouse wheel input')

    def xd(self, *args, check=True):
        if args[0] == 'mousemove':
            x, y, _, _ = self.bounds()
            target = (x + int(args[-2]), y + int(args[-1]))
            if sys.platform == 'win32':
                require(self.os.SetCursorPos(*target), 'Windows rejected native cursor movement')
            else:
                self.input.moveTo(*target)
            time.sleep(.1)
        elif args[0] == 'click':
            button = int(args[-1])
            count = int(args[args.index('--repeat') + 1]) if '--repeat' in args else 1
            if button in (4, 5):
                for _ in range(count):
                    if sys.platform == 'win32':
                        self.send_wheel(120 if button == 4 else -120)
                    else:
                        self.input.scroll(1 if button == 4 else -1)
                    time.sleep(.02)
            else:
                for _ in range(count):
                    self.input.mouseDown()
                    time.sleep(.08)
                    self.input.mouseUp()
                    time.sleep(.08)
        elif args[0] in ('mousedown', 'mouseup'):
            require(int(args[-1]) == 1, 'Native slider drag requires the left mouse button')
            if args[0] == 'mousedown':
                self.input.mouseDown()
            else:
                self.input.mouseUp()
            time.sleep(.08)
        else:
            raise RuntimeError(f'Unsupported desktop input: {args}')
        return ''

    def dialog_visible(self):
        if sys.platform == 'darwin':
            # Synchronous rfd dialogs without set_parent use runModal(), so
            # NSSavePanel/NSOpenPanel are standalone AXDialog windows.
            return self.apple('''repeat with i from 1 to count windows
    set w to window i
    if exists sheet 1 of w then return "sheet 1 of window " & i
    set panelSubrole to subrole of w
    if panelSubrole is "AXDialog" or panelSubrole is "AXSystemDialog" then return "window " & i
end repeat
return ""''') or None
        for hwnd in self.windows():
            name = self.ctypes.create_unicode_buffer(256)
            self.os.GetClassNameW(hwnd, name, 256)
            if name.value == '#32770':
                return hwnd
        return None

    def paste(self, text):
        # Use OS clipboard to preserve arbitrary Unicode filesystem paths.
        if sys.platform == 'darwin':
            subprocess.run(['pbcopy'], input=str(text), text=True, check=True, timeout=10)
            self.input.hotkey('command', 'v')
        else:
            # PowerShell stdin avoids interpolating file paths into code.
            subprocess.run(['powershell', '-NoProfile', '-Command', '$input | Set-Clipboard'],
                           input=str(text), text=True, check=True, timeout=15)
            self.input.hotkey('ctrl', 'v')

    def dialog(self, path, save=False):
        try:
            chooser = wait_for(self.dialog_visible, 'native file chooser', 25)
            if sys.platform == 'win32':
                self.os.SetForegroundWindow(chooser)
            target = self.root / f'{len(self.records):02d}-native-file-chooser.png'
            self.input.screenshot().save(target)
            self.records.append({'surface': 'native-file-chooser', 'screenshot': target.name,
                                 'requested_path': str(path), 'save': save, 'chooser': chooser})
            if sys.platform == 'darwin':
                self.input.hotkey('command', 'shift', 'g')
                wait_for(
                    lambda: self.apple(f'get exists sheet 1 of {chooser}') == 'true',
                    'macOS Go to Folder sheet', 15)
                self.apple(
                    f'set value of text field 1 of sheet 1 of {chooser} to '
                    f'{json.dumps(str(path.parent if save else path))}')
                self.input.press('enter')
                # Go to Folder is its own sheet, including on a standalone
                # NSSavePanel. Do not write the name until that sheet closes.
                def navigated():
                    return self.apple(f'if not (exists sheet 1 of {chooser}) then return "{chooser}"') or None
                chooser = wait_for(navigated, 'macOS Go to Folder accepted path', 15)
                if save:
                    self.apple(f'''set candidates to {{}}
set nameField to missing value
set chooserElements to (get entire contents of {chooser})
repeat with elementReference in chooserElements
    set element to contents of elementReference
    if role of element is "AXTextField" then
        set end of candidates to element
        try
            if (description of element contains "Save As") or (name of element contains "Save As") then set nameField to element
        end try
    end if
end repeat
if nameField is missing value and (count candidates) is 1 then set nameField to item 1 of candidates
if nameField is missing value then error ("Cannot identify Save As field; AXTextField count=" & (count candidates))
set focused of nameField to true
set value of nameField to {json.dumps(path.name)}''')
            else:
                self.input.hotkey('alt', 'n')
                self.input.hotkey('ctrl', 'a')
                self.paste(path)
            self.input.press('enter')
            wait_for(lambda: not self.dialog_visible(), 'native chooser accepted path', 20)
        except Exception:
            if sys.platform == 'darwin':
                self.mac_diagnostics('file-chooser-failure')
            raise

    def close(self, exporting=False):
        self.focus()
        if exporting:
            self.require_export_in_flight('close')
        self.input.hotkey('command', 'q') if sys.platform == 'darwin' else self.input.hotkey('alt', 'f4')
        self.proc.wait(timeout=30)
        require(self.proc.returncode == 0, f'GUI close status {self.proc.returncode}')
        self.no_children()
        self.proc = None


def read_image(path):
    reader = oiio.ImageInput.open(str(path))
    require(reader, f'GUI output cannot be decoded: {path}')
    spec = reader.spec()
    format_name = str(spec.format)
    image = np.asarray(reader.read_image(oiio.FLOAT))
    reader.close()
    require(format_name == 'float', f'GUI float save has depth {format_name}: {path}')
    require(np.isfinite(image).all(), f'Nonfinite GUI output: {path}')
    require(float(image.max() - image.min()) > .001, f'Blank GUI output: {path}')
    return image


def accept_gui(gui, exporter, source, raw, evidence, environment):
    root = evidence / 'gui-acceptance'
    root.mkdir()
    for name in ('config', 'cache', 'temp', 'home'):
        (root / name).mkdir()
    env = dict(environment)
    for key in ('SPEKTRAFILM_F64_CLI', 'SPEKTRAFILM_GUI_STATE', 'SPEKTRAFILM_DATA_DIR',
                'SPEKTRAFILM_PY', 'SPEKTRAFILM_PY_REPO'):
        env.pop(key, None)
    env.update(SPEKTRAFILM_CONFIG_DIR=str(root / 'config'), XDG_CACHE_HOME=str(root / 'cache'),
               XDG_CONFIG_HOME=str(root / 'config'), HOME=str(root / 'home'),
               TMPDIR=str(root / 'temp'))
    env.pop('SPEKTRAFILM_GUI_RENDERER', None)
    if sys.platform.startswith('linux'):
        env['SPEKTRAFILM_GUI_RENDERER'] = 'glow'
    elif sys.platform == 'win32':
        env['SPEKTRAFILM_GUI_RENDERER'] = 'wgpu'
    env.update(TEMP=str(root / 'temp'), TMP=str(root / 'temp'),
               LOCALAPPDATA=str(root / 'cache'), APPDATA=str(root / 'config'))
    # Keep native/OCR dependencies on PATH; observed child hash enforces provenance.
    env['PATH'] = str(exporter.parent) + os.pathsep + env.get('PATH', os.defpath)
    driver = X11(root) if sys.platform.startswith('linux') else Desktop(root)
    native_exporter = exporter
    if exporter.read_bytes().startswith(b'#!'):
        native_exporter = exporter.parent.parent / 'lib' / 'spektrafilm' / 'spektrafilm-f64'
        require(native_exporter.is_file(), f'Installed launcher lacks native exporter: {native_exporter}')
    driver.exporter_hash = hashlib.sha256(native_exporter.read_bytes()).hexdigest()
    rust_repo = Path(__file__).resolve().parents[2]
    rust_commit = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=rust_repo,
                                 capture_output=True, text=True, check=True).stdout.strip()
    rust_worktree_dirty = bool(subprocess.run(
        ['git', 'status', '--porcelain', '--untracked-files=all'], cwd=rust_repo,
        capture_output=True, text=True, check=True).stdout.strip())
    driver.records.append({'rust_commit': rust_commit,
                           'rust_worktree_dirty': rust_worktree_dirty,
                           'gui_executable': str(gui.resolve()),
                           'gui_sha256': hashlib.sha256(gui.read_bytes()).hexdigest(),
                           'exporter_executable': str(native_exporter.resolve()),
                           'exporter_sha256': driver.exporter_hash,
                           'platform': sys.platform})
    try:
        driver.start(gui, env)
        driver.observe_tabs('fresh-launch')
        startup_image, _, startup_lines = driver.read()
        startup_text = '\n'.join(' '.join(word[0] for word in line) for line in startup_lines)
        require(re.search(r'Load\s+an\s+image\s+to\s+start', startup_text, re.I),
                'Fresh GUI did not show the startup placeholder status')
        driver.snap('startup-placeholder', startup_image, startup_lines)
        driver.records.append({'parity_action': 'show_startup_placeholder',
                               'assertion': 'fresh native window shows the load-image placeholder',
                               'screenshot': driver.records[-1]['screenshot']})
        baseline = root / 'baseline.json'
        driver.file_action('Save state', baseline, True)
        wait_for(baseline.is_file, 'saved baseline state', 20)
        state = json.loads(baseline.read_text())
        state['grain']['active'] = False
        state['halation']['active'] = False
        state['glare']['active'] = False
        state['input_image']['input_color_space'] = 'sRGB'
        state['display']['preview_max_size'] = 64
        state['camera']['auto_exposure'] = False
        configured = root / 'configured.json'
        configured.write_text(json.dumps(state))
        driver.file_action('Load state', configured)
        # Use a recognisable spatial gradient for visual comparison and finite
        # spectral rendering; the separate CLI fixture covers negative/headroom.
        standard = root / 'standard.tif'
        x = np.linspace(.05, .8, 192, dtype=np.float32)[None, :]
        y = np.linspace(.1, .6, 96, dtype=np.float32)[:, None]
        pixels = np.stack((np.broadcast_to(x, (96, 192)),
                           np.broadcast_to(y, (96, 192)),
                           np.broadcast_to(.2 + x * y, (96, 192))), axis=2)
        writer = oiio.ImageOutput.create(str(standard))
        require(writer and writer.open(str(standard), oiio.ImageSpec(192, 96, 3, oiio.FLOAT)), 'Cannot create GUI input')
        require(writer.write_image(pixels) and writer.close(), 'Cannot write GUI input')
        import exiv2
        metadata = exiv2.ImageFactory.open(str(standard))
        metadata.readMetadata()
        metadata.exifData()['Exif.Image.Artist'] = 'GUI rotation artist'
        metadata.iptcData()['Iptc.Application2.Caption'] = 'GUI rotation caption'
        metadata.xmpData()['Xmp.dc.description'] = 'GUI rotation description'
        metadata.writeMetadata()
        driver.file_action('Open', standard)
        driver.click('Input', False)
        loaded_bounds = driver.measure_viewer('loaded-input-image')
        driver.records.append({'parity_action': 'load_input_image',
                               'assertion': 'opened raster produces non-empty native Input viewer pixels',
                               'source_dimensions': [192, 96],
                               'pixel_viewer_bounds': loaded_bounds,
                               'screenshot': driver.records[-1]['screenshot']})
        scan_before = root / 'scan-for-print-before.json'
        driver.file_action('Save state', scan_before, True)
        wait_for(scan_before.is_file, 'scan-for-print baseline state', 20)
        baseline_state = json.loads(scan_before.read_text())
        baseline_scan = {'scanner': baseline_state.get('scanner', {}),
                         'glare': baseline_state.get('glare', {})}
        driver.click('Scan-for-print')
        driver.rendered('scan-for-print-on')
        scan_on = root / 'scan-for-print-on.json'
        driver.file_action('Save state', scan_on, True)
        wait_for(scan_on.is_file, 'scan-for-print enabled state', 20)
        enabled_state = json.loads(scan_on.read_text())
        enabled_scan = {'scanner': enabled_state.get('scanner', {}),
                        'glare': enabled_state.get('glare', {})}
        require(enabled_scan['scanner'].get('white_correction') is True and
                enabled_scan['scanner'].get('black_correction') is True and
                enabled_scan['glare'].get('active') is False,
                'Scan-for-print did not force correction/glare settings')
        driver.click('Scan-for-print')
        driver.rendered('scan-for-print-off')
        scan_after = root / 'scan-for-print-after.json'
        driver.file_action('Save state', scan_after, True)
        wait_for(scan_after.is_file, 'scan-for-print restored state', 20)
        restored_state = json.loads(scan_after.read_text())
        restored_scan = {'scanner': restored_state.get('scanner', {}),
                         'glare': restored_state.get('glare', {})}
        require(restored_scan == baseline_scan,
                'Scan-for-print did not restore exact pre-toggle settings')
        driver.records.append({'parity_action': 'scan_for_print',
                               'scenario': 'scan_for_print',
                               'assertion': 'forced settings and exact restoration',
                               'baseline': baseline_scan, 'forced': enabled_scan,
                               'restored': restored_scan})
        driver.file_action('Load state', scan_on)
        driver.rendered('scan-for-print-loaded-state')
        driver.click('Scan-for-print')
        driver.rendered('scan-for-print-loaded-toggle')
        loaded_toggle = root / 'scan-for-print-loaded-toggle.json'
        driver.file_action('Save state', loaded_toggle, True)
        wait_for(loaded_toggle.is_file, 'scan-for-print loaded toggle state', 20)
        loaded_state = json.loads(loaded_toggle.read_text())
        require(loaded_state['scanner']['white_correction'] is True and
                loaded_state['scanner']['black_correction'] is True and
                loaded_state['glare']['active'] is False,
                'Loaded state retained the transient scan-for-print snapshot')
        driver.file_action('Load state', scan_before)
        driver.rendered('scan-for-print-baseline-restored')
        # The render-count proxy is the reported output width: preview
        # renders at preview_max_size (64) while Scan renders full size, so
        # run Scan first, save its full-resolution output, then Preview —
        # each click then observably changes the reported size.
        _, _, lines = driver.read()
        before_scan = render_count('\n'.join(' '.join(w[0] for w in line) for line in lines))
        driver.click('Scan')
        scan_status = driver.rendered('explicit-scan', before_scan)
        scan_output = root / 'scan-output.exr'
        driver.file_action('Save', scan_output, True)
        wait_for(scan_output.is_file, 'full-resolution scan output', 30)
        scan_reader = oiio.ImageInput.open(str(scan_output))
        require(scan_reader, f'Cannot decode Scan output: {scan_output}')
        scan_spec = scan_reader.spec()
        scan_reader.close()
        require((scan_spec.width, scan_spec.height) == (192, 96),
                f'Scan output dimensions differ from source: {(scan_spec.width, scan_spec.height)}')
        driver.records.append({'parity_action': 'run_scan',
                               'assertion': 'Scan click increments render counter and saves source dimensions',
                               'status': scan_status,
                               'render_count_before': before_scan,
                               'render_count_after': render_count(scan_status),
                               'output_dimensions': [scan_spec.width, scan_spec.height]})
        # The GUI saves asynchronously and overwrites the status line once
        # done; wait for that before Preview so the preview render's own
        # status is the last one written.
        driver.wait_text(r'Saved\s+scan-output\.exr', 'scan-output-saved', 30)
        driver.click('Preview')
        preview_status = driver.rendered('explicit-preview', render_count(scan_status))
        driver.records.append({'parity_action': 'run_preview',
                               'assertion': 'Preview click re-renders at the preview size',
                               'status': preview_status,
                               'render_count_before': render_count(scan_status),
                               'render_count_after': render_count(preview_status)})
        driver.tab('CONFIG')
        driver.scroll(True)
        driver.click('Restore factory default')
        factory_state = root / 'factory-state.json'
        driver.file_action('Save state', factory_state, True)
        wait_for(factory_state.is_file, 'factory state saved', 20)
        factory = json.loads(factory_state.read_text())
        canonical_factory_path = Path(__file__).resolve().parents[2] / 'crates/spektrafilm-gui/src/factory_state.json'
        canonical_factory = json.loads(canonical_factory_path.read_text())
        configured_state = json.loads(configured.read_text())
        require(any(factory.get(section) != configured_state.get(section)
                    for section in canonical_factory),
                'Factory reset did not change the configured non-default state')
        # Saved state round-trips RuntimeParams (f32), so canonical f64
        # literals differ in the last digits; compare with float tolerance.
        # The dichroic neutrals are additionally re-derived from the neutral
        # print filter database at render time, so a saved factory state may
        # carry the raw defaults there instead of the database values.
        # RuntimeParams uses 32-bit neutral defaults when the render-time
        # database lookup has no persisted database value. Keep that
        # normalization explicit; do not accept arbitrary alternatives to the
        # canonical factory JSON.
        neutral_defaults = {'c_filter_neutral': 0.0, 'm_filter_neutral': 65.0, 'y_filter_neutral': 55.0}
        factory_normalizations = []
        def section_matches(section, actual, expected, normalizations):
            for key, value in (expected or {}).items():
                saved = (actual or {}).get(key)
                if isinstance(value, bool) or isinstance(saved, bool):
                    if saved is not value:
                        return False
                elif isinstance(value, (int, float)) and isinstance(saved, (int, float)):
                    tolerant = math.isclose(saved, value, rel_tol=1e-6, abs_tol=1e-6)
                    normalized = (key in neutral_defaults and
                                  math.isclose(saved, neutral_defaults[key], rel_tol=1e-6, abs_tol=1e-6))
                    if tolerant:
                        continue
                    if normalized:
                        normalizations.append({'section': section, 'key': key,
                                               'canonical': value, 'runtime_default': saved})
                        continue
                    return False
                elif isinstance(value, list) and isinstance(saved, list):
                    if len(value) != len(saved):
                        return False
                    for item, saved_item in zip(value, saved):
                        if isinstance(item, (int, float)) and isinstance(saved_item, (int, float)):
                            if not math.isclose(saved_item, item, rel_tol=1e-6, abs_tol=1e-6):
                                return False
                        elif item != saved_item:
                            return False
                elif saved != value:
                    return False
            return True
        for section, expected in canonical_factory.items():
            require(section_matches(section, factory.get(section), expected, factory_normalizations),
                    f'Factory reset changed canonical section {section}')
        driver.records.append({'factory_state_neutral_normalizations': factory_normalizations})
        driver.scroll(True)
        driver.click('Save startup default')
        driver.close()
        driver.start(gui, env, standard)
        restarted_factory = root / 'factory-restarted.json'
        driver.file_action('Save state', restarted_factory, True)
        wait_for(restarted_factory.is_file, 'factory state after restart', 20)
        restarted = json.loads(restarted_factory.read_text())
        for section, expected in canonical_factory.items():
            require(section_matches(section, restarted.get(section), expected, factory_normalizations),
                    f'Factory reset restart changed canonical section {section}')
        driver.records.append({'parity_action': 'restore_factory_default',
                               'factory_reset': True,
                               'assertion': 'reset matches canonical factory state and survives restart',
                               'canonical_sections': sorted(canonical_factory),
                               'factory_restart_preserved_sections': sorted(canonical_factory)})
        # Zoom/rotation assertions need full-size viewer rasters; raise the
        # preview bound above the fixture's long edge so preview renders are
        # no longer downscaled.
        viewing_state = json.loads(configured.read_text())
        viewing_state['display']['preview_max_size'] = 256
        viewing = root / 'viewing.json'
        viewing.write_text(json.dumps(viewing_state))
        driver.file_action('Load state', viewing)
        driver.rendered('configured-after-factory-reset')
        driver.click('100%', False)
        zoom_100 = driver.measure_viewer('zoom-100-percent')
        driver.click('200%', False)
        zoom_200 = driver.measure_viewer('zoom-200-percent')
        driver.click('400%', False)
        zoom_400 = driver.measure_viewer('zoom-400-percent')
        widths = [item[2] - item[0] for item in (zoom_100, zoom_200, zoom_400)]
        require(widths == [192, 384, 768], f'Exact zoom pixel widths differ: {widths}')
        driver.click('reset view', False)
        driver.click('cw rotate', False)
        # Tesseract renders the × separator as = or * at footer sizes.
        driver.wait_text(r'Rendered\s+96\s*[x×=*]\s*192', 'clockwise-render-complete')
        cw_bounds = driver.measure_viewer('clockwise-rotation')
        driver.tab('MAIN')
        driver.click('16 bit')
        driver.click('32 bit')
        rotated_export = root / 'rotated-export.tif'
        driver.file_action('Export', rotated_export, True)
        driver.capture_export_child(exporter)
        driver.wait_text(r'Exported.*f64', 'rotated-f64-export', timeout=240)
        require(read_image(rotated_export).shape == (192, 96, 3),
                'Rotated f64 export has incorrect dimensions')
        rotation_error = float(np.max(np.abs(driver.staged_export_input - np.rot90(pixels, -1))))
        require(rotation_error == 0, f'Rotated export input differs from NumPy: {rotation_error}')
        driver.records.append({'parity_action': 'rotate_input_image_clockwise',
                               'assertion': 'clockwise export stages an exact quarter-turned input',
                               'rotated_export_shape': [192, 96, 3],
                               'rotated_input_max_error': rotation_error})
        metadata = exiv2.ImageFactory.open(str(rotated_export))
        metadata.readMetadata()
        for key, expected in (('Exif.Image.Artist', 'GUI rotation artist'),
                              ('Exif.Image.Orientation', '1'),
                              ('Exif.Photo.PixelXDimension', '96'),
                              ('Exif.Photo.PixelYDimension', '192')):
            require(metadata.exifData()[key].toString() == expected, f'Rotated metadata differs: {key}')
        require(metadata.iptcData()['Iptc.Application2.Caption'].toString() == 'GUI rotation caption', 'Rotated IPTC lost')
        require('GUI rotation description' in metadata.xmpData()['Xmp.dc.description'].toString(), 'Rotated XMP lost')
        driver.records.append({'rotated_metadata_preserved': True, 'orientation': 1, 'dimensions': [96, 192]})
        driver.no_children()
        driver.tab('CONFIG')
        driver.click('ccw rotate', False)
        driver.wait_text(r'Rendered\s+192\s*[x×=*]\s*96', 'counterclockwise-render-complete')
        ccw_bounds = driver.measure_viewer('counterclockwise-rotation')
        require(cw_bounds[3] - cw_bounds[1] > cw_bounds[2] - cw_bounds[0],
                f'Clockwise rotation did not produce portrait pixels: {cw_bounds}')
        require(ccw_bounds[2] - ccw_bounds[0] > ccw_bounds[3] - ccw_bounds[1],
                f'Counterclockwise rotation did not restore landscape pixels: {ccw_bounds}')
        driver.records.append({'parity_action': 'rotate_input_image_counterclockwise',
                               'assertion': 'counterclockwise rotation restores landscape viewer bounds',
                               'zoom_pixel_widths': widths,
                               'rotation_pixel_bounds': {'cw': cw_bounds, 'ccw': ccw_bounds}})
        from gui_viewer_acceptance import accept_viewer
        driver.records.extend(accept_viewer(driver, root, state, pixels, exporter))
        driver.click('100%', False)
        driver.click('Input', False)
        input_image, _ = driver.image()
        driver.snap('input-view')
        driver.click('Output', False)
        time.sleep(.5)
        output_image, _ = driver.image()
        driver.snap('output-view')
        roi = (100, 100, 900, 800)
        input_crop, output_crop = input_image.crop(roi), output_image.crop(roi)
        input_crop.save(root / 'input-viewer-raster.png')
        output_crop.save(root / 'output-viewer-raster.png')
        input_pixels, output_pixels = np.asarray(input_crop, dtype=float), np.asarray(output_crop, dtype=float)
        difference = float(np.mean(np.abs(input_pixels - output_pixels)))
        require(difference > .1, 'Input/output controls did not change viewer pixels')
        output_chroma = output_pixels.max(axis=2) - output_pixels.min(axis=2)
        output_mask = (output_chroma > 25) & (output_pixels.max(axis=2) > 35)
        output_content = output_pixels[output_mask]
        require(output_content.shape[0] > 100, 'Output viewer has no measurable rendered pixels')
        output_content_std = float(output_content.std())
        require(output_content_std > 5, 'Output viewer lost gradient contrast')
        driver.records.append({'viewer_roi': list(roi), 'input_raster': 'input-viewer-raster.png',
                               'output_raster': 'output-viewer-raster.png', 'mean_abs_pixel_change': difference,
                               'output_pixel_std': output_content_std,
                               'output_rendered_pixel_count': int(output_content.shape[0])})
        driver.tab('CONFIG')
        driver.scroll(False)
        display_image, _, display_lines = driver.read()
        display_text = '\n'.join(' '.join(word[0] for word in line) for line in display_lines)
        require(re.search(r'Display\s+transform:', display_text, re.I),
                'Native GUI did not report display transform status')
        driver.snap('display-transform-status', display_image, display_lines)
        driver.records.append({'parity_action': 'report_display_transform_status',
                               'assertion': 'CONFIG reports the active display transform status',
                               'screenshot': driver.records[-1]['screenshot'],
                               'status': display_text})
        driver.tab('MAIN')
        display_before = {key: state['display'].get(key) for key in
                          ('use_display_transform', 'gray_18_canvas', 'white_padding', 'output_interpolation')}
        before_auto_image, _ = driver.image()
        before_auto_pixels = np.asarray(before_auto_image.crop((0, 80, 1030, 900)), dtype=float)
        driver.click('Auto exposure')
        def auto_render_ready():
            image, _, lines = driver.read()
            text = '\n'.join(' '.join(w[0] for w in line) for line in lines)
            require(not re.search(r'(?:Render|Load|Save|Export|Startup state|Preview state)\s*(?:error|failed)', text, re.I),
                    f'GUI failure on changed-auto-exposure-preview: {text}')
            current_pixels = np.asarray(image.crop((0, 80, 1030, 900)), dtype=float)
            if re.search(r'Rendered\s+\d+', text, re.I) and np.mean(np.abs(current_pixels - before_auto_pixels)) > 0.01:
                driver.snap('changed-auto-exposure-preview', image, lines)
                return text
        auto_preview_status = wait_for(auto_render_ready, 'changed-auto-exposure-preview', 120)
        driver.records.append({'parity_action': 'request_auto_preview',
                               'assertion': 'runtime editor change updates the native rendered pixels',
                               'status': auto_preview_status})
        display_probe = root / 'display-after-exposure.json'
        driver.file_action('Save state', display_probe, True)
        wait_for(display_probe.is_file, 'display probe state', 20)
        display_after = json.loads(display_probe.read_text())['display']
        require(all(display_after.get(key) == value for key, value in display_before.items()),
                'Display settings changed while rendering exposure')
        driver.records.append({'display_float_invariance': True,
                               'checked_display_fields': list(display_before)})
        driver.tab('MAIN')
        # The depth control is a native combo: the selected value is the only
        # visible label until the combo is opened.
        driver.click('32 bit')
        driver.click('16 bit')
        driver.click('16 bit')
        driver.click('32 bit')
        saved_state = root / 'roundtrip.json'
        driver.file_action('Save state', saved_state, True)
        wait_for(saved_state.is_file, 'saved changed state', 20)
        saved = json.loads(saved_state.read_text())
        require(saved['camera']['auto_exposure'] is True, 'Control change absent from saved state')
        require(saved['rust']['save_bit_depth'] == 32, 'Float depth selection absent from state')
        driver.records.append({'parity_action': 'save_current_state_to_file',
                               'assertion': 'Save state writes the changed control values to JSON',
                               'state': saved_state.name,
                               'auto_exposure': saved['camera']['auto_exposure'],
                               'save_bit_depth': saved['rust']['save_bit_depth']})
        driver.file_action('Load state', configured)
        driver.rendered('reloaded-original-state')
        driver.file_action('Load state', saved_state)
        loaded_state_status = driver.rendered('loaded-changed-state')
        loaded_probe = root / 'loaded-state-observed.json'
        driver.file_action('Save state', loaded_probe, True)
        wait_for(loaded_probe.is_file, 'observed loaded state', 20)
        loaded = json.loads(loaded_probe.read_text())
        require(loaded['camera']['auto_exposure'] == saved['camera']['auto_exposure'],
                'Load state did not restore auto exposure')
        require(loaded['rust']['save_bit_depth'] == saved['rust']['save_bit_depth'],
                'Load state did not restore save depth')
        driver.records.append({'parity_action': 'load_state_from_file',
                               'assertion': 'Load state restores observed control values and renders',
                               'status': loaded_state_status,
                               'auto_exposure': loaded['camera']['auto_exposure'],
                               'save_bit_depth': loaded['rust']['save_bit_depth'],
                               'observed_state': loaded_probe.name})
        driver.tab('CONFIG')
        driver.click('Save startup default')
        startup = root / 'config' / 'gui_default_state.json'
        wait_for(startup.is_file, 'saved startup default', 20)
        driver.close()
        driver.start(gui, env, standard)
        driver.rendered('restart-startup-restored')
        restored_path = root / 'restored.json'
        driver.file_action('Save state', restored_path, True)
        wait_for(restored_path.is_file, 'restart state saved', 20)
        restored = json.loads(restored_path.read_text())
        require(restored == saved, 'Startup default did not restore the complete saved state')
        driver.records.append({'parity_action': 'save_current_as_default',
                               'assertion': 'startup default restores the complete saved state after restart',
                               'persisted_state_keys': sorted(saved),
                               'save_bit_depth': restored['rust']['save_bit_depth']})
        float_path = root / 'preview.exr'
        driver.file_action('Save', float_path, True)
        wait_for(float_path.is_file, 'real float image save', 30)
        preview = read_image(float_path)
        exported_path = root / 'full-export.exr'
        driver.file_action('Export', exported_path, True)
        driver.capture_export_child(exporter)
        driver.scroll(True)
        driver.wait_text(r'Exported.*f64', 'successful-bundled-f64-export', timeout=240)
        full = read_image(exported_path)
        require(full.shape[:2] == pixels.shape[:2],
                f'GUI f64 export dimensions differ from source: {full.shape} vs {pixels.shape}')
        require(preview.shape[0] <= full.shape[0] and preview.shape[1] <= full.shape[1],
                f'GUI preview exceeds source dimensions: {preview.shape} vs {full.shape}')
        driver.no_children()
        driver.records.append({'preview_shape': list(preview.shape), 'export_shape': list(full.shape),
                               'source_shape': list(pixels.shape),
                               'preview_sha256': hashlib.sha256(float_path.read_bytes()).hexdigest(),
                               'export_sha256': hashlib.sha256(exported_path.read_bytes()).hexdigest()})
        raw_input = root / ('raw-input' + raw.suffix.lower())
        shutil.copyfile(raw, raw_input)
        driver.file_action('Open', raw_input)
        driver.rendered('raw-image-preview')
        driver.tab('CONFIG')
        driver.scroll(False)
        driver.wait_text(r'Lens correction (?:not applied|applied)',
                         'raw-lens-correction-status', timeout=20)
        driver.file_action('Save state', root / 'raw-state.json', True)
        wait_for((root / 'raw-state.json').is_file, 'RAW state', 20)
        raw_state = json.loads((root / 'raw-state.json').read_text())
        require(raw_state['input_image']['input_color_space'] == 'ACES2065-1', 'RAW load did not configure ACES input')
        driver.records.append({'parity_action': 'load_raw_image',
                               'scenario': 'raw_status',
                               'assertion': 'RAW load configures ACES2065-1 and reports lens correction result',
                               'raw_input_space': raw_state['input_image']['input_color_space']})
        # A real large image keeps the spectral child in flight long enough to
        # exercise Cancel and window close. No fake exporter or timing hook.
        large = root / 'large.tif'
        pixels = np.tile(np.linspace(.05, .8, 4096, dtype=np.float32)[None, :, None], (3072, 1, 3))
        writer = oiio.ImageOutput.create(str(large))
        require(writer and writer.open(str(large), oiio.ImageSpec(4096, 3072, 3, oiio.FLOAT)), 'Cannot create cancellation input')
        require(writer.write_image(pixels) and writer.close(), 'Cannot write cancellation input')
        driver.file_action('Load state', saved_state)
        driver.tab('MAIN')
        driver.file_action('Open', large)
        driver.rendered('large-image-preview')
        cancelled = root / 'cancelled.exr'
        driver.file_action('Export', cancelled, True)
        driver.capture_export_child(exporter)
        driver.snap('cancel-export-in-flight')
        driver.click('Cancel')
        driver.scroll(True)
        driver.wait_text(r'Export cancel(?:ed|led)', 'cancelled-export', timeout=30)
        driver.no_children()
        require(not cancelled.exists(), 'Cancelled export left an output image')
        leftovers = list(root.rglob('spektrafilm-export-*'))
        require(not leftovers, f'Cancelled export temporary files survived: {leftovers}')
        closed = root / 'closed.exr'
        driver.file_action('Export', closed, True)
        driver.capture_export_child(exporter)
        driver.snap('close-export-in-flight')
        driver.close(exporting=True)
        require(not closed.exists(), 'Closing in-flight export left an output image')
        leftovers = list(root.rglob('spektrafilm-export-*'))
        require(not leftovers, f'Export temporary files survived: {leftovers}')
        driver.records.append({'startup_restore': True, 'raw_input_space': 'ACES2065-1',
                               'cancel_output_absent': True, 'close_output_absent': True,
                               'export_temp_files': []})
        return {'gui': str(gui), 'evidence': str(root / 'observations.json'), 'native_acceptance': 'passed'}
    except Exception as error:
        driver.records.append({'failure': str(error)})
        if driver.proc and driver.proc.poll() is None:
            try:
                driver.snap('failure')
            except Exception as screenshot_error:
                driver.records.append({'failure_screenshot_error': str(screenshot_error)})
        raise
    finally:
        driver.finish()
