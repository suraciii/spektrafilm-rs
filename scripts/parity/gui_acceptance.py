#!/usr/bin/env python3
"""Installed egui acceptance through native desktop input, file dialogs and OCR.

Linux: Xvfb/openbox/xdotool/xprop/xwininfo/xclip/zenity/tesseract; Windows/macOS: pyautogui
and Tesseract. macOS requires runner Accessibility/Automation/Screen Recording
permissions and an active desktop. Acceptance uses rendered controls, status,
and produced image files without GUI implementation hooks.

For an isolated Linux desktop, create the session bus inside Xvfb:
    xvfb-run -a -s '-screen 0 1600x1100x24' dbus-run-session -- <acceptance command>
Install/start the GTK file chooser portal on that same display. An inherited
host session bus can leave synchronous rfd dialogs blocked for 120 seconds.

Export controls are located from the rendered modal and confirmed by closed-row
OCR after selection. Popup OCR is confined to the observed button's inner edges
and adjacent changed pixels. A failed popup observation preserves closed/open
frames, a difference mask, and candidate bounds in the evidence directory.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

import numpy as np
import OpenImageIO as oiio


def viewer_canvas_bounds(image):
    """Default 260px preset / 320px inspector edges, excluding viewer overlays."""
    return (260, 80, image.width - 320, max(81, image.height - 80))


def require(value, message):
    if not value:
        raise RuntimeError(message)





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
        for command in ('xdotool', 'xprop', 'xwininfo', 'xclip', 'openbox', 'zenity', 'tesseract', 'ffmpeg', 'gdbus'):
            require(shutil.which(command), f'GUI acceptance requires native dependency: {command}')
        import psutil
        from PIL import Image
        self.mss, self.psutil, self.ocr, self.Image = _desktop_capture(), psutil, _desktop_ocr(), Image
        self.root, self.window, self.proc = root, None, None
        self.records, self.wm = [], None
        self.last_report_id = 0
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

    def accept_numeric_and_click(self, text, point):
        """Accept selected numeric text and click before automatic debounce."""
        self.xd('type', '--delay', '0', '--clearmodifiers', '--', text)
        self.xd('key', 'Return', 'mousemove', '--window', self.window,
                int(point[0]), int(point[1]),
                'mousedown', '1', 'sleep', '.03', 'mouseup', '1')

    def chooser_ready(self, env):
        guidance = ('Native chooser needs a session bus and GTK portal on the GUI display. '
                    "Run xvfb-run -a -s '-screen 0 1600x1100x24' "
                    'dbus-run-session -- <acceptance command>; do not reuse the host session bus.')
        require(env.get('DBUS_SESSION_BUS_ADDRESS'), guidance)
        try:
            portal = subprocess.run(
                ['gdbus', 'introspect', '--session', '--dest', 'org.freedesktop.portal.Desktop',
                 '--object-path', '/org/freedesktop/portal/desktop'],
                env=env, capture_output=True, text=True, timeout=15)
            require(portal.returncode == 0 and 'org.freedesktop.portal.FileChooser' in portal.stdout,
                    f'{guidance} Portal probe: {portal.stderr.strip()}')
            owner = subprocess.run(
                ['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
                 '--object-path', '/org/freedesktop/DBus', '--method',
                 'org.freedesktop.DBus.GetConnectionUnixProcessID', 'org.freedesktop.portal.Desktop'],
                env=env, capture_output=True, text=True, timeout=10, check=True)
            pid = int(re.search(r'uint32\s+(\d+)', owner.stdout).group(1))
            portal_display = self.psutil.Process(pid).environ().get('DISPLAY')
            require(portal_display == env.get('DISPLAY'),
                    f'{guidance} Portal DISPLAY={portal_display!r}, GUI DISPLAY={env.get("DISPLAY")!r}')
            self.records.append({'native_chooser_ready': True, 'portal_pid': pid,
                                 'portal_display': portal_display})
        except (subprocess.SubprocessError, self.psutil.Error, AttributeError, ValueError) as error:
            raise RuntimeError(f'{guidance} Portal probe failed: {error}') from error

    def gui_environment(self, env):
        """Environment for a GUI launch, including the report publication directory.

        The GUI publishes every completed operation's report there, so readiness
        and operation facts come from the report instead of the status line.
        """
        directory = self.root / 'diagnostics'
        directory.mkdir(exist_ok=True)
        self.report_directory = Path(tempfile.mkdtemp(prefix='session-', dir=directory))
        self.records.append({'diagnostic_session': str(self.report_directory.relative_to(self.root))})
        env = dict(env)
        # OCR scenarios use the production English presentation, independent of
        # the runner's locale; language preferences remain outside GUI state.
        env.update(LANG='C', LC_ALL='C', LANGUAGE='en')
        self.backend_requested = env.get('SPEKTRAFILM_BACKEND', 'auto').lower()
        if env.get('SPEKTRAFILM_CONFIG_DIR'):
            preference = Path(env['SPEKTRAFILM_CONFIG_DIR']) / 'gui_preferences.json'
            preference.write_text(json.dumps({'language': 'english'}) + '\n')
        env['SPEKTRAFILM_GUI_DIAGNOSTICS_DIR'] = str(self.report_directory)
        self.last_report_id = 0
        return env



    def resize_window(self, width, height):
        self.xd('windowsize', '--sync', self.window, width, height)

    def window_geometry(self, window):
        # xdotool getwindowgeometry adds the reparented frame offset twice
        # under Openbox. xwininfo reports the true root-relative client origin.
        info = subprocess.run(['xwininfo', '-id', str(window)], check=True,
                              capture_output=True, text=True, timeout=10).stdout
        def field(name):
            return int(re.search(rf'{name}:\s+(-?\d+)', info).group(1))
        return dict(left=field('Absolute upper-left X'), top=field('Absolute upper-left Y'),
                    width=field('Width'), height=field('Height'))

    def image(self):
        require(self.proc.poll() is None, 'Installed GUI exited; see gui.log')
        bbox = self.window_geometry(self.window)
        with self.mss.mss() as screen:
            shot = screen.grab(bbox)
            return self.Image.frombytes('RGB', shot.size, shot.rgb), bbox

    def read(self):
        image, bbox = self.image()
        # The current native control panel is fixed at 320px. Keep viewer and
        # sidebar OCR separate so header and preset text cannot merge controls.
        self.right_control_x = image.width - 320
        self.action_top = image.height - 100
        self.action_left = image.width - 320
        from PIL import ImageOps, ImageStat
        lines = []
        # Segment the sidebar from the viewer and normalize each region to
        # dark text on a light background, including native light themes.
        def normalize(region):
            grayscale = ImageOps.grayscale(region)
            background = grayscale.crop((0, 0, grayscale.width, min(50, grayscale.height)))
            return ImageOps.invert(grayscale) if ImageStat.Stat(background).median[0] < 128 else grayscale

        for offset, region in ((0, image.crop((0, 0, 328, image.height))),
                               (328, image.crop((328, 0, image.width - 320, image.height))),
                               (image.width - 320, image.crop((image.width - 320, 0, image.width, image.height)))):
            prepared = normalize(region)
            # The sidebar is read in bands: a full-height pass loses the small
            # controls inside its collapsible sections.
            step = prepared.height // 2 if offset else prepared.height
            for index in range(2 if offset else 1):
                top = index * step
                band = prepared.crop((0, top, prepared.width, min(prepared.height, top + step + 40)))
                # The crops are large; a tight bound trips on a loaded machine.
                data = self.ocr.image_to_data(band.resize((band.width * 3, band.height * 3)),
                                              config='--psm 11', output_type=self.ocr.Output.DICT, timeout=60)
                grouped = {}
                for i, text in enumerate(data['text']):
                    if text.strip():
                        key = (data['block_num'][i], data['par_num'][i], data['line_num'][i])
                        grouped.setdefault(key, []).append((text, offset + data['left'][i] / 3,
                            top + data['top'][i] / 3, data['width'][i] / 3, data['height'][i] / 3))
                lines.extend(grouped.values())
        # A second complete-row pass preserves short parameter labels near
        # the scrolling viewport's bottom, which sparse OCR can omit.
        left, top = image.width - 320, 215
        region = image.crop((left, top, image.width, self.action_top))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 3, region.height * 3)),
                                      config='--psm 6', output_type=self.ocr.Output.DICT, timeout=15)
        grouped = {}
        for i, text in enumerate(data['text']):
            if text.strip():
                key = (data['block_num'][i], data['par_num'][i], data['line_num'][i])
                grouped.setdefault(key, []).append((text, left + data['left'][i] / 3,
                    top + data['top'][i] / 3, data['width'][i] / 3, data['height'][i] / 3))
        lines.extend(grouped.values())
        # Preserve the independently recognized panel words in their physical
        # header order too: a full-row OCR pass can misread a tiny filename.
        header_words = [word for line in lines for word in line
                        if word[2] + word[4] / 2 < 27]
        lines.append(sorted(header_words, key=lambda word: word[1]))
        # Operation status crosses the viewer/sidebar boundary in the header.
        # Read that row whole so adapter details cannot split Exported from
        # its destination filename (or hide RAW/cancellation acknowledgement).
        region = image.crop((0, 0, image.width, 27))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 4, region.height * 4)),
                                      config='--psm 7', output_type=self.ocr.Output.DICT, timeout=15)
        lines.append([(text, data['left'][i] / 4, data['top'][i] / 4,
                       data['width'][i] / 4, data['height'][i] / 4)
                      for i, text in enumerate(data['text']) if text.strip()])
        # Read the persistent sidebar action bar as complete rows so SCAN and
        # SAVE are distinguishable from nearby control labels.
        action_top = self.action_top
        action_left = self.action_left
        region = image.crop((action_left, action_top, image.width, image.height))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 4, region.height * 4)),
                                      config='--psm 6', output_type=self.ocr.Output.DICT, timeout=15)
        grouped = {}
        for i, text in enumerate(data['text']):
            if text.strip():
                key = (data['block_num'][i], data['par_num'][i], data['line_num'][i])
                grouped.setdefault(key, []).append((text, action_left + data['left'][i] / 4,
                                                    action_top + data['top'][i] / 4,
                                                    data['width'][i] / 4, data['height'][i] / 4))
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
        left = viewer_canvas_bounds(image)[0]
        top = image.height - 80
        region = image.crop((left, top, image.width - 320, image.height))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 4, region.height * 4)),
                                      config='--psm 6', output_type=self.ocr.Output.DICT, timeout=15)
        words = [(text, left + data['left'][i] / 4, top + data['top'][i] / 4,
                  data['width'][i] / 4, data['height'][i] / 4)
                 for i, text in enumerate(data['text']) if text.strip()]
        lines.append(words)
        return image, bbox, lines

    def match(self, lines, label, right=False):
        # OCR may transliterate the ellipsis; match words, with explicit boundaries.
        wanted = re.findall(r'[a-z0-9]+', label.lower())
        if label.lower() in ('scan', 'save'):
            words = [word for line in lines for word in line
                     if word[1] >= self.action_left and word[2] >= self.action_top]
            buttons = []
            for word in words:
                # Workflow buttons are uppercase; Scan for print is a separate
                # control even when OCR merges their rows.
                token = re.sub(r'[^A-Za-z0-9]', '', word[0])
                if token != label.upper():
                    continue
                y = word[2] + word[4] / 2
                row = {re.sub(r'[^A-Za-z0-9]', '', item[0]).lower() for item in words
                       if abs(item[2] + item[4] / 2 - y) < 8
                       and re.sub(r'[^A-Za-z0-9]', '', item[0]) in ('SCAN', 'SAVE')}
                # Disabled SAVE and hovered buttons can lose their OCR label.
                # The bottom action row still identifies an exact uppercase
                # token, below the checkbox and workflow selector rows.
                if {'scan', 'save'} <= row or y >= self.action_top + 50:
                    point = (word[1] + word[3] / 2, y)
                    if point not in buttons:
                        buttons.append(point)
            return buttons
        aliases = {
            'open': {'open', 'pen', 'per', 'pe'},
            'save': {'save', 'ave', 'saye'},
        }
        matches = []
        joined_wanted = ''.join(wanted)
        for line in lines:
            for start in range(len(line)):
                words = []
                selected = []
                for word in line[start:]:
                    selected.append(word)
                    words.extend(re.findall(r'[a-z0-9]+', word[0].lower()))
                    # OCR may merge adjacent words, so collect until the
                    # concatenated tokens cover the label.
                    if len(''.join(words)) >= len(joined_wanted):
                        break
                equivalent = (words == wanted
                              or ''.join(words) == joined_wanted
                              or (len(words) == len(wanted)
                                  and all(word == expected or word in aliases.get(expected, set())
                                          for word, expected in zip(words, wanted))))
                if equivalent or (label.lower() == 'auto exposure' and words == ['aueo', 'exposure']) or (label.lower() == '16 bit' and words == ['16', 'bir']):
                    x = selected[0][1]
                    if right and x < self.right_control_x:
                        continue
                    if label.lower() == 'save' and selected[0][2] < 100:
                        continue
                    # Save must not select Save state or Save startup.
                    following = line[start + len(selected):start + len(selected) + 1]
                    if label.lower() == 'save' and following and following[0][0].lower() in ('state', 'startup'):
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


    def scroll(self, bottom):
        image, _, _ = self.read()
        require(image.width >= 1460, 'Native GUI acceptance requires the pinned 1460px window width')
        # Wheel over the sidebar's left gutter, not a numeric editor: editors
        # consume wheel input as value changes instead of scrolling the panel.
        sidebar_x = image.width - 310
        self.xd('mousemove', '--window', self.window, int(sidebar_x), int(image.height / 2))
        self.xd('click', '--repeat', 35, '--delay', 8, 5 if bottom else 4)
        time.sleep(.15)

    def viewer_canvas(self):
        """Central canvas pixels, excluding panels and both status bars."""
        image, _ = self.image()
        return image, np.asarray(image.crop(viewer_canvas_bounds(image)).convert('RGB'))

    def viewer_fingerprint(self):
        """Canvas hash captured before a view change, for the next measurement."""
        _, canvas = self.viewer_canvas()
        return hashlib.sha256(canvas.tobytes()).hexdigest()

    def await_viewer(self, previous=None, timeout=60, label='viewer'):
        """Wait for the composition worker to paint a settled viewer frame.

        The raster reaches the viewer through an asynchronous composition
        worker, so the frame right after a display change still shows the
        previous settings and can stay that way for seconds under load. Wait
        until the canvas differs from `previous`, then until two consecutive
        screenshots agree. Returns the settled screenshot and canvas.
        """
        state = {'image': None, 'canvas': None, 'digest': None, 'changed': previous is None}

        def settled():
            image, canvas = self.viewer_canvas()
            digest = hashlib.sha256(canvas.tobytes()).hexdigest()
            if not state['changed']:
                if digest != previous:
                    state['changed'] = True
                else:
                    time.sleep(.5)
                    return False
            if digest == state['digest']:
                state['image'] = image
                state['canvas'] = canvas
                return True
            state['digest'] = digest
            time.sleep(.5)
            return False

        wait_for(settled, f'settled {label} frame', timeout)
        return state['image'], state['canvas']

    def measure_viewer(self, label, changed_from=None, timeout=60):
        """Measure the known colored fixture after selecting its Input raster."""
        image, canvas = self.await_viewer(changed_from, timeout, label)
        channels = [canvas[..., i].astype(np.int16) for i in range(3)]
        chroma = np.maximum.reduce(channels) - np.minimum.reduce(channels)
        mask = (chroma > 25) & (np.maximum.reduce(channels) > 35)
        ys, xs = np.where(mask)
        bounds = None
        if xs.size > 100:
            left, top, _, _ = viewer_canvas_bounds(image)
            bounds = [int(xs.min() + left), int(ys.min() + top),
                      int(xs.max() + left + 1), int(ys.max() + top + 1)]
        screenshot = f'{len(self.records):02d}-{label}.png'
        image.save(self.root / screenshot)
        self.records.append({'surface': label, 'screenshot': screenshot,
                             'pixel_viewer_bounds': bounds})
        require(bounds is not None, f'Colored Input raster is not visible for {label}: {screenshot}')
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
            for dialog_class in dialog_classes:
                found = self.xd('search', '--onlyvisible', '--class', dialog_class, check=False)
                if found:
                    return found.splitlines()[-1]
            return None

        dialog = wait_for(find, 'native file chooser (zenity/yad/portal)', 25)

        def visible():
            # Once discovered, query this window directly. Recursive class
            # searches traverse unrelated GTK windows on every key press.
            info = subprocess.run(['xwininfo', '-id', str(dialog)],
                                  capture_output=True, text=True, timeout=10)
            return info.returncode == 0 and 'Map State: IsViewable' in info.stdout

        def key(*keys):
            if not visible():
                return False
            self.xd('key', *keys)
            return True

        def geometry():
            try:
                return self.window_geometry(dialog)
            except subprocess.CalledProcessError:
                if not visible():
                    return None
                raise
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
        if not visible():
            return
        self.xd('windowactivate', '--sync', dialog)
        time.sleep(.6)
        if not key('ctrl+l'):
            return
        time.sleep(.2)
        if not key('ctrl+a'):
            return
        text = str(path if save else path.parent) + ('' if save else '/')
        # GTK completion consumes synthetic per-character input; paste the
        # complete path atomically through the real desktop clipboard.
        # xclip forks a clipboard owner; inherited output pipes would keep an
        # enclosing acceptance subprocess's communicate() waiting after exit.
        subprocess.run(['xclip', '-selection', 'clipboard'], input=text,
                       text=True, check=True, timeout=10,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not key('ctrl+v'):
            return
        time.sleep(.3)
        if not key('Return'):
            return
        if not save:
            time.sleep(.6)
            if not key('ctrl+l') or not key('ctrl+a'):
                return
            subprocess.run(['xclip', '-selection', 'clipboard'], input=str(path),
                           text=True, check=True, timeout=10,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if not key('ctrl+v'):
                return
            time.sleep(.3)
            if not key('Return'):
                return
        # Save choosers may first navigate the entered full path, then require
        # a final action button. Open dialogs likewise expose Select/Open on
        # portal backends instead of silently accepting Return.
        deadline = time.monotonic() + 15
        primary_clicked = False
        expected = {'save', 'ok'} if save else {'open', 'select', 'ok'}
        while time.monotonic() < deadline:
            if not visible():
                return
            if primary_clicked:
                time.sleep(.25)
                continue
            bounds = geometry()
            if bounds is None:
                return
            strip = dict(left=bounds['left'] + bounds['width'] - 320,
                         top=bounds['top'] + bounds['height'] - 60,
                         width=320, height=60)
            with self.mss.mss() as screen:
                action_shot = screen.grab(strip)
            actions = self.Image.frombytes('RGB', action_shot.size, action_shot.rgb)
            actions = actions.convert('L').resize((320 * 3, 60 * 3))
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            try:
                buttons = self.ocr.image_to_data(actions, config='--psm 6',
                                                 output_type=self.ocr.Output.DICT,
                                                 timeout=remaining)
            except subprocess.TimeoutExpired:
                break
            except RuntimeError as error:
                if str(error) != 'Tesseract process timeout':
                    raise
                break
            cancels = [i for i, word in enumerate(buttons['text'])
                       if word.strip().casefold() == 'cancel']
            candidates = []
            if len(cancels) == 1:
                cancel = cancels[0]
                cancel_y = buttons['top'][cancel] + buttons['height'][cancel] / 2
                cancel_right = buttons['left'][cancel] + buttons['width'][cancel]
                for i, word in enumerate(buttons['text']):
                    center_y = buttons['top'][i] + buttons['height'][i] / 2
                    same_row = abs(center_y - cancel_y) <= max(
                        buttons['height'][i], buttons['height'][cancel]) / 2
                    if (word.strip().casefold() in expected and same_row
                            and buttons['left'][i] > cancel_right):
                        candidates.append(i)
            # OCR can outlive a chooser or a move. Discard that frame rather
            # than sending a stale click to another desktop surface.
            if not visible():
                return
            if (len(candidates) == 1 and time.monotonic() < deadline
                    and self.xd('getactivewindow', check=False) == dialog
                    and geometry() == bounds):
                i = candidates[0]
                x = strip['left'] + (buttons['left'][i] + buttons['width'][i] / 2) / 3
                y = strip['top'] + (buttons['top'][i] + buttons['height'][i] / 2) / 3
                self.xd('mousemove', int(x), int(y))
                if not visible():
                    return
                if (self.xd('getactivewindow', check=False) == dialog
                        and geometry() == bounds and time.monotonic() < deadline):
                    self.xd('click', 1)
                    primary_clicked = True
            time.sleep(.25)
        if not visible():
            return
        with self.mss.mss() as screen:
            shot = screen.grab(screen.monitors[0])
        surface = self.Image.frombytes('RGB', shot.size, shot.rgb)
        surface.save(self.root / 'native-chooser-timeout.png')
        raise RuntimeError(f'Native chooser did not accept {path}')


    def drain_reports(self):
        """Advance past reports already published.

        A view or state change can leave a preview report the scenarios never
        consumed; draining before an action keeps the next wait from matching
        that stale report instead of the operation the action started.
        """
        highest = self.last_report_id
        for path in self.report_directory.glob('operation-*.json'):
            try:
                operation = json.loads(path.read_text())['operation']
            except (OSError, ValueError, KeyError):
                continue
            highest = max(highest, operation['id'])
        self.last_report_id = highest

    def wait_report(self, label, action=None, outcome='succeeded'):
        """Wait for the report the GUI publishes for a completed operation.

        The report is the diagnostic source of truth: readiness, outcome and
        dimensions come from it, so no status-line OCR is involved.
        """
        expected = (action or '').lower()
        accepted = (expected,) if expected else ('preview', 'scan')
        def ready():
            require(self.proc.poll() is None, 'GUI exited; see gui.log')
            reports = []
            for path in self.report_directory.glob('operation-*.json'):
                try:
                    report = json.loads(path.read_text())
                except (OSError, ValueError):
                    continue
                reports.append(report)
            reports.sort(key=lambda report: report['operation']['id'])
            for report in reports:
                operation = report['operation']
                if operation['id'] <= self.last_report_id:
                    continue
                if operation['kind'] not in accepted:
                    continue
                if operation['outcome'] == 'superseded':
                    continue
                require(operation['outcome'] == outcome,
                        f"GUI {operation['kind']} {operation['outcome']} on {label}: "
                        f"{report.get('diagnostic_issues')}")
                execution = report['execution']
                adapter = execution.get('adapter')
                requested = (getattr(self, 'export_backend_requested', None)
                             if operation['kind'] == 'export'
                             else getattr(self, 'backend_requested', None))
                if (requested == 'wgpu'
                        and operation['kind'] in ('preview', 'scan', 'export')
                        and outcome == 'succeeded'):
                    require(execution.get('backend_requested') == 'wgpu'
                            and execution.get('backend_selected') == 'wgpu' and adapter,
                            f'Explicit WGPU request was not honored on {label}: {execution}')
                self.records.append({
                    'operation_report': label, 'operation': operation,
                    'execution': execution,
                    'software_adapter': adapter.get('device_type') == 'cpu' if adapter else None,
                    'stage_executors': [
                        {key: stage.get(key) for key in ('name', 'executor', 'cpu_reason')}
                        for stage in report.get('stages', [])],
                })
                self.last_report_id = operation['id']
                self.last_completed_report = report
                return report
        return wait_for(ready, label, 120)

    def rendered(self, label, action=None):
        require(action in (None, 'Preview', 'Scan'), f'Unknown render action: {action}')
        self.wait_report(label, action)
        image, _, lines = self.read()
        text = '\n'.join(' '.join(w[0] for w in line) for line in lines)
        self.snap(label, image, lines)
        return text


    def require_export_in_flight(self, action, observation=None):
        if observation is None:
            image, _, lines = self.read()
        else:
            image, lines = observation
        text = '\n'.join(' '.join(w[0] for w in line) for line in lines)
        require(re.search(r'Exporting\s+with', text, re.I),
                f'{action} did not target a GUI-observed in-flight export: {text}')
        self.snap('export-in-flight-' + action.lower(), image, lines)
        self.records.append({'in_flight_export_action': action, 'status': text,
                             'screenshot': self.records[-1]['screenshot']})

    def exported(self, path, label):
        # The operation status is in the persistent header.
        status = self.wait_text(r'\bExported\b', label, 240)
        wait_for(Path(path).is_file, f'export output {path}', 30)
        self.wait_report(label + '-report', 'Export')
        self.last_export_report = self.last_completed_report
        self.records.append({'export_output': str(path), 'status': status,
                             'sha256': hashlib.sha256(Path(path).read_bytes()).hexdigest()})
        return status

    def close(self, exporting=False):
        self.xd('windowactivate', '--sync', self.window)
        if exporting:
            self.require_export_in_flight('close')
        self.xd('key', 'alt+F4')
        try:
            self.discard_photograph_changes()
        except (RuntimeError, subprocess.CalledProcessError):
            # The native window may disappear before the process finishes
            # teardown. A clean exit still must pass the check below.
            try:
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                raise
        self.proc.wait(timeout=240 if exporting else 30)
        require(self.proc.returncode == 0, f'GUI closed with status {self.proc.returncode}')
        self.proc = None

    def finish(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=10)
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
        self.records, self.wm = [], None
        self.last_report_id = 0
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
        env = self.gui_environment(env)
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
        self.resize_window(1460, 980)
        wait_for(lambda: self.locate('Search controls', right=True),
                 'rendered desktop control search', 120)

    def resize_window(self, width, height):
        if sys.platform == 'darwin':
            self.apple('set w to first window whose name is "spektrafilm"\n'
                       f'set position of w to {{0, 30}}\nset size of w to {{{width}, {height}}}')
        else:
            rect = self.types.RECT(0, 0, width, height)
            self.os.AdjustWindowRect(self.ctypes.byref(rect), self.os.GetWindowLongW(self.window, -16), False)
            self.os.MoveWindow(self.window, 0, 30, rect.right - rect.left, rect.bottom - rect.top, True)

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
        elif args[0] == 'key':
            require(len(args) == 2 and args[1] in ('ctrl+a', 'Return', 'BackSpace', 'Tab', 'Escape'),
                    f'Unsupported desktop key command: {args}')
            if args[1] == 'ctrl+a':
                self.input.hotkey('command' if sys.platform == 'darwin' else 'ctrl', 'a')
            else:
                self.input.press({'Return': 'enter', 'BackSpace': 'backspace',
                                  'Tab': 'tab', 'Escape': 'esc'}[args[1]])
        elif args[0] == 'type':
            require(len(args) >= 3 and args[1:-1] in (
                ('--clearmodifiers',), ('--clearmodifiers', '--'),
                ('--clearmodifiers', '--delay', '0')),
                f'Unsupported desktop type options: {args}')
            text = args[-1]
            require(isinstance(text, str) and text.isascii()
                    and all(character.isprintable() for character in text),
                    'Native desktop typing requires printable ASCII text')
            modifiers = ('ctrl', 'shift', 'alt',
                         'command' if sys.platform == 'darwin' else 'win')
            for modifier in modifiers:
                self.input.keyUp(modifier)
            self.input.write(text, interval=0)
        else:
            raise RuntimeError(f'Unsupported desktop input: {args}')
        return ''

    def accept_numeric_and_click(self, text, point):
        """Use native input without per-call pauses inside the debounce window."""
        x, y, _, _ = self.bounds()
        target = (x + int(point[0]), y + int(point[1]))
        self.input.write(text, interval=0, _pause=False)
        self.input.press('enter', _pause=False)
        if sys.platform == 'win32':
            self.send_pointer(*target)
        else:
            self.input.moveTo(*target, _pause=False)
        self.input.mouseDown(_pause=False)
        time.sleep(.03)
        self.input.mouseUp(_pause=False)

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
        self.proc.wait(timeout=240 if exporting else 30)
        require(self.proc.returncode == 0, f'GUI close status {self.proc.returncode}')
        self.proc = None


def read_image(path):
    reader = oiio.ImageInput.open(str(path))
    require(reader, f'GUI output cannot be decoded: {path}')
    spec = reader.spec()
    format_name = str(spec.format)
    image = np.asarray(reader.read_image(oiio.FLOAT))
    reader.close()
    expected = {'.exr': {'half', 'float'}, '.tif': {'uint16', 'float'}, '.tiff': {'uint16', 'float'},
                '.png': {'uint8'}, '.jpg': {'uint8'}, '.jpeg': {'uint8'}}.get(Path(path).suffix.lower())
    require(expected and format_name in expected, f'GUI output has unexpected depth {format_name}: {path}')
    require(np.isfinite(image).all(), f'Nonfinite GUI output: {path}')
    require(float(image.max() - image.min()) > .001, f'Blank GUI output: {path}')
    return image


def accept_gui(gui, exporter, source, raw, evidence, environment):
    """Exercise the installed continuous inspector, never the retired five pages.

    Imports stay local because the canonical stage driver and its scenarios use
    the capture, chooser and diagnostic utilities in this module.
    """
    from experimental_gui import ExperimentalDesktop, parameter_state, parameters_equal
    from gui_control_acceptance import cancel_export, enter_number, export_controls, export_image, imported_scan_illuminant
    from gui_viewer_acceptance import accept_viewer, _hover_float

    class InstalledDesktop(ExperimentalDesktop, Desktop):
        """Canonical navigation with Windows/macOS native input and choosers."""

        def start(self, gui, env, image=None):
            self.settings_open = False
            self.search_point = None
            Desktop.start(self, gui, env, image)

    root = evidence / 'gui-acceptance'
    root.mkdir()
    for name in ('config', 'cache', 'temp', 'home'):
        (root / name).mkdir()
    env = dict(environment)
    for key in ('SPEKTRAFILM_F64_CLI', 'SPEKTRAFILM_GUI_STATE', 'SPEKTRAFILM_DATA_DIR',
                'SPEKTRAFILM_PY', 'SPEKTRAFILM_PY_REPO', 'SPEKTRAFILM_GUI_RENDERER'):
        env.pop(key, None)
    env.update(SPEKTRAFILM_CONFIG_DIR=str(root / 'config'), XDG_CACHE_HOME=str(root / 'cache'),
               XDG_CONFIG_HOME=str(root / 'config'), HOME=str(root / 'home'),
               TMPDIR=str(root / 'temp'), TEMP=str(root / 'temp'), TMP=str(root / 'temp'),
               LOCALAPPDATA=str(root / 'cache'), APPDATA=str(root / 'config'))
    if sys.platform.startswith('linux'):
        env['SPEKTRAFILM_GUI_RENDERER'] = 'glow'
    elif sys.platform == 'win32':
        env['SPEKTRAFILM_GUI_RENDERER'] = 'wgpu'
    driver = ExperimentalDesktop(root) if sys.platform.startswith('linux') else InstalledDesktop(root)
    repo = Path(__file__).resolve().parents[2]
    commit = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=repo,
                            capture_output=True, text=True, check=True).stdout.strip()
    dirty = bool(subprocess.run(['git', 'status', '--porcelain', '--untracked-files=all'],
                                cwd=repo, capture_output=True, text=True, check=True).stdout.strip())
    driver.records.append({'rust_commit': commit, 'rust_worktree_dirty': dirty,
                           'gui_executable': str(gui.resolve()),
                           'gui_sha256': hashlib.sha256(gui.read_bytes()).hexdigest(),
                           'reference_exporter': str(exporter.resolve()), 'platform': sys.platform,
                           'native_surface': 'continuous-inspector',
                           'compute_backend_requested': env.get('SPEKTRAFILM_BACKEND', 'auto').lower(),
                           'gui_renderer': env.get('SPEKTRAFILM_GUI_RENDERER'),
                           'vulkan_icd': env.get('VK_ICD_FILENAMES'),
                           'retired_surface': 'MAIN/FILM/PRINT/ADVANCED/CONFIG pages'})

    def require_factory(actual):
        # Render preparation/provenance are derived runtime diagnostics, not
        # factory preferences. Compare every shared canonical section and every
        # explicitly shipped Rust factory preference rather than those counters.
        for section, expected in factory.items():
            if section != 'rust':
                require(parameters_equal(actual.get(section), expected),
                        f'Factory reset differs in canonical section {section}')
        shipped = json.loads((repo / 'crates/spektrafilm-gui/src/factory_state.json').read_text())
        for key in shipped['rust']:
            require(parameters_equal(actual['rust'].get(key), factory['rust'].get(key)),
                    f'Factory reset differs in Rust preference {key}')

    def save_pixels(path, label):
        driver.click('SAVE', bottom=True)
        driver.dialog(path, save=True)
        driver.wait_report(label, 'Save')
        wait_for(path.is_file, label + ' output', 30)
        return read_image(path)

    def scan_output(label):
        driver.click('SCAN', bottom=True)
        report = driver.wait_report(label, 'Scan')
        driver.snap(label)
        driver.records.append({'parity_action': 'run_scan', 'report': report})
        return report
    try:
        driver.start(gui, env)
        driver.wait_text(r'Load\s+an\s+image\s+to\s+start', 'startup-placeholder')
        driver.surfaces()
        state = driver.state_action('Save current to file', root / 'baseline.json', save=True)
        factory = json.loads(json.dumps(state))
        imported_scan_illuminant(driver, root)
        state['grain']['active'] = False
        state['halation']['active'] = False
        state['glare']['active'] = False
        state['input_image']['input_color_space'] = 'sRGB'
        state['display']['preview_max_size'] = 64
        state['camera']['auto_exposure'] = False
        if driver.backend_requested == 'wgpu':
            state.setdefault('rust', {})['export_backend'] = 'gpu'
        configured = root / 'configured.json'
        configured.write_text(json.dumps(state))
        driver.state_action('Load from file', configured)

        # Unlike the CLI headroom fixture, this spatial gradient measures the
        # real Input/Output viewer and retained Preview/Scan dimensions.
        standard = root / 'standard.tif'
        x = np.linspace(.05, .8, 192, dtype=np.float32)[None, :]
        y = np.linspace(.1, .6, 96, dtype=np.float32)[:, None]
        pixels = np.stack((np.broadcast_to(x, (96, 192)),
                           np.broadcast_to(y, (96, 192)),
                           np.broadcast_to(.2 + x * y, (96, 192))), axis=2)
        writer = oiio.ImageOutput.create(str(standard))
        require(writer and writer.open(str(standard), oiio.ImageSpec(192, 96, 3, oiio.FLOAT)),
                'Cannot create GUI input')
        require(writer.write_image(pixels) and writer.close(), 'Cannot write GUI input')
        import exiv2
        metadata = exiv2.ImageFactory.open(str(standard))
        metadata.readMetadata()
        metadata.exifData()['Exif.Image.Artist'] = 'GUI rotation artist'
        metadata.iptcData()['Iptc.Application2.Caption'] = 'GUI rotation caption'
        metadata.xmpData()['Xmp.dc.description'] = 'GUI rotation description'
        metadata.writeMetadata()
        preview_report = driver.open_image(standard, 'imported-rgb')
        preview_path = root / 'preview.exr'
        preview = save_pixels(preview_path, 'preview-output-saved')
        preview_reader = oiio.ImageInput.open(str(preview_path))
        require(preview_reader, 'Cannot inspect Preview Save depth')
        require(str(preview_reader.spec().format) == 'half',
                'EXR Save did not use extension-default 16-bit depth')
        preview_reader.close()
        scan_report = scan_output('explicit-scan')
        scan = save_pixels(root / 'scan-output.exr', 'scan-output-saved')
        require(scan.shape == pixels.shape, f'Scan dimensions differ: {scan.shape}')
        require(preview.shape[1] == 64 and preview.shape[0] < scan.shape[0],
                f'Preview did not use its bounded size: {preview.shape} vs {scan.shape}')
        driver.records.append({'preview_shape': list(preview.shape), 'scan_shape': list(scan.shape),
                               'source_shape': list(pixels.shape),
                               'preview_report': preview_report, 'scan_report': scan_report})
        from gui_control_acceptance import scanner_readout
        scanner_readout(driver, root)

        # Fotufilm is selected through the actual grain owner, not a loaded
        # runtime patch masquerading as control evidence.
        driver.reveal('film_render.grain.active', 'Film', 'Grain')
        driver.toggle('Enable grain')
        driver.reveal('film_render.grain.engine', 'Film', 'Grain')
        driver.choose('Engine', 'Fotufilm Film')
        for path, label in (
                ('film_render.grain.fotufilm.amount', 'Amount'),
                ('film_render.grain.fotufilm.size', 'Size'),
                ('film_render.grain.fotufilm.softness', 'Softness'),
                ('film_render.grain.fotufilm.color', 'Color'),
                ('film_render.grain.fotufilm.layers', 'Layer amounts')):
            driver.reveal(path, 'Film', 'Grain')
            require(driver.locate(label, right=True), f'Fotufilm control missing: {label}')
            observed = driver.snap('fotufilm-control-' + label.replace(' ', '-'))
            require(not re.search(r'Dye\s+support', observed, re.I),
                    'Dye-only control remained visible for Fotufilm')
        driver.reveal('viewer.layer.output', 'Viewer', 'Viewer')
        driver.click('Output', bottom=True)
        driver.reveal('film_render.grain.fotufilm.amount', 'Film', 'Grain')
        before_amount = driver.viewer_fingerprint()
        driver.drain_reports()
        enter_number(driver, 'Amount', '1.65')
        driver.wait_report('fotufilm-amount-change', 'Preview')
        driver.await_viewer(before_amount, label='fotufilm amount render update')
        fotufilm = driver.state_action('Save current to file', root / 'fotufilm-state.json', save=True)
        grain = fotufilm['rust']['runtime']['film_render']['grain']
        require(grain['active'] is True and grain['engine'] == 'fotufilm'
                and parameters_equal(grain['fotufilm']['amount'], 1.65),
                f'Fotufilm control change not persisted: {grain}')
        fotufilm_preview = driver.state_action('Load from file', root / 'fotufilm-state.json')
        loaded = driver.state_action('Save current to file', root / 'fotufilm-loaded.json', save=True)
        require(loaded['rust']['runtime']['film_render']['grain'] == grain,
                'Fotufilm state did not round-trip')
        for action in ('Preview', 'Scan'):
            report = fotufilm_preview if action == 'Preview' else scan_output('fotufilm-scan')
            require(report['configuration']['render']['grain']['active'] is True
                    and report['configuration']['render']['grain']['engine'] == 'fotufilm',
                    'Fotufilm render did not execute the active selected engine')
            # The photo-first header reports action/dimensions only. Execution
            # disclosure belongs to Settings/Diagnostics, not retired status prose.
            driver.reveal('action.save_report', 'Settings', 'Diagnostics')
            disclosure = driver.snap('fotufilm-' + action.lower() + '-diagnostics')
            executor = report['execution']['backend_selected']
            require(re.search(r'requested[^\n]*' + executor, disclosure, re.I)
                    and re.search(r'selected[^\n]*' + executor, disclosure, re.I),
                    f'Fotufilm diagnostics differs from executed backend: {disclosure}')
            driver.close_settings()
            driver.records.append({'parity_action': 'fotufilm_' + action.lower(),
                                   'report': report, 'executor_disclosure': executor})
        float_export = root / 'fotufilm-float-export.exr'
        driver.click('Export', header=True)
        driver.export_options(float_export, float_export=True)
        driver.dialog(float_export, save=True)
        driver.exported(float_export, 'fotufilm-float-export')
        require(read_image(float_export).shape == pixels.shape, 'Fotufilm export dimensions differ')

        scan_seed = json.loads(json.dumps(state))
        scan_seed['glare']['active'] = True
        scan_seed_path = root / 'scan-for-print-seed.json'
        scan_seed_path.write_text(json.dumps(scan_seed))
        driver.state_action('Load from file', scan_seed_path)
        driver.reveal('action.scan_for_print', 'Scan', 'Scanner')
        driver.click('scan for print', right=True)
        forced = driver.state_action('Save current to file', root / 'scan-for-print-on.json', save=True)
        require(forced['glare']['active'] is False, 'Scan for print did not disable print glare')
        driver.reveal('action.scan_for_print', 'Scan', 'Scanner')
        driver.click('scan for print', right=True)
        restored = driver.state_action('Save current to file', root / 'scan-for-print-off.json', save=True)
        require(restored['glare']['active'] is True,
                'Scan for print did not restore prior glare setting')
        driver.state_action('Load from file', root / 'scan-for-print-on.json')
        driver.reveal('action.scan_for_print', 'Scan', 'Scanner')
        driver.click('scan for print', right=True)
        loaded_toggle = driver.state_action('Save current to file', root / 'scan-for-print-loaded.json', save=True)
        require(loaded_toggle['glare']['active'] is False,
                'Loading state resurrected the transient scan-for-print snapshot')
        driver.records.append({'parity_action': 'scan_for_print',
                               'assertion': 'current print-glare override and restoration',
                               'retired_assertion': 'removed white-and-black-correction control'})

        # The viewer suite retains actual pixel/float/animation/cache/isolation
        # checks, using the same canonical driver as this installed gate.
        driver.state_action('Load from file', configured)
        driver.records.extend(accept_viewer(driver, root, state, pixels, exporter))
        viewing = json.loads(json.dumps(state))
        viewing['display']['preview_max_size'] = 256
        viewing_path = root / 'viewing.json'
        viewing_path.write_text(json.dumps(viewing))
        driver.state_action('Load from file', viewing_path)
        driver.drain_reports()
        driver.click('cw rotate')
        clockwise = driver.wait_report('clockwise-render-complete')
        require(clockwise['configuration']['working_dimensions'] == [96, 192],
                'Clockwise rotation did not render portrait')
        driver.click('Input')
        _hover_float(driver, np.rot90(pixels, -1), driver.records)
        cw_bounds = driver.measure_viewer('clockwise-rotation')
        require(cw_bounds[3] - cw_bounds[1] > cw_bounds[2] - cw_bounds[0],
                f'Clockwise rotation did not produce portrait Input pixels: {cw_bounds}')
        rotated_export = root / 'rotated-export.tif'
        export_image(driver, rotated_export)
        require(read_image(rotated_export).shape == (192, 96, 3), 'Rotated export dimensions differ')
        metadata = exiv2.ImageFactory.open(str(rotated_export))
        metadata.readMetadata()
        for key, expected in (('Exif.Image.Artist', 'GUI rotation artist'),
                              ('Exif.Image.Orientation', '1'),
                              ('Exif.Photo.PixelXDimension', '96'),
                              ('Exif.Photo.PixelYDimension', '192')):
            require(metadata.exifData()[key].toString() == expected, f'Rotated metadata differs: {key}')
        require(metadata.iptcData()['Iptc.Application2.Caption'].toString() == 'GUI rotation caption',
                'Rotated IPTC lost')
        require('GUI rotation description' in metadata.xmpData()['Xmp.dc.description'].toString(),
                'Rotated XMP lost')
        driver.drain_reports()
        driver.click('ccw rotate')
        counterclockwise = driver.wait_report('counterclockwise-render-complete')
        require(counterclockwise['configuration']['working_dimensions'] == [192, 96],
                'Counterclockwise rotation did not restore landscape')
        driver.click('Input', bottom=True)
        ccw_bounds = driver.measure_viewer('counterclockwise-rotation')
        require(ccw_bounds[2] - ccw_bounds[0] > ccw_bounds[3] - ccw_bounds[1],
                f'Counterclockwise rotation did not restore landscape Input pixels: {ccw_bounds}')
        driver.records.append({'rotated_metadata_preserved': True, 'orientation': 1,
                               'dimensions': [96, 192]})
        driver.state_action('Load from file', configured)
        driver.reveal('camera.auto_exposure', 'Input and capture', 'Camera')
        driver.drain_reports()
        driver.toggle('Auto exposure')
        driver.wait_report('changed-auto-exposure-preview', 'Preview')
        changed = driver.state_action('Save current to file', root / 'roundtrip.json', save=True)
        require(changed['camera']['auto_exposure'] is True, 'Auto exposure control was not saved')
        require(all(changed['display'].get(key) == state['display'].get(key) for key in
                    ('use_display_transform', 'gray_18_canvas', 'white_padding', 'output_interpolation')),
                'Display settings changed while rendering exposure')
        driver.state_action('Load from file', configured)
        driver.state_action('Load from file', root / 'roundtrip.json')
        loaded = driver.state_action('Save current to file', root / 'loaded-state.json', save=True)
        require(parameter_state(loaded) == parameter_state(changed), 'Complete state roundtrip differs')
        driver.startup_action('Save current as default')
        startup = root / 'config' / 'gui_default_state.json'
        wait_for(startup.is_file, 'saved startup default', 20)
        driver.close()
        driver.start(gui, env, standard)
        restored = driver.state_action('Save current to file', root / 'restarted.json', save=True)
        require(parameter_state(restored) == parameter_state(changed), 'Startup default changed state')
        driver.startup_action('Restore factory default')
        restored_factory = driver.state_action('Save current to file', root / 'factory-restored.json', save=True)
        require_factory(restored_factory)
        driver.startup_action('Save current as default')
        driver.close()
        driver.start(gui, env, standard)
        restarted_factory = driver.state_action('Save current to file', root / 'factory-restarted.json', save=True)
        require_factory(restarted_factory)

        driver.state_action('Load from file', configured)
        export_controls(driver)
        exported_path = root / 'full-export.tif'
        export_image(driver, exported_path)
        require(read_image(exported_path).shape == pixels.shape, 'Full export dimensions differ')
        driver.state_action('Load from file', configured)
        raw_input = root / ('raw-input' + raw.suffix.lower())
        shutil.copyfile(raw, raw_input)
        raw_preview = driver.open_image(raw_input, 'raw-image-loaded')
        require(raw_preview['configuration']['working_dimensions'] == [768, 512],
                'RAW automatic preview did not retain the full source dimensions')
        driver.reveal('extras.load_raw.white_balance', 'Input and capture', 'Import Raw')
        raw_status = driver.wait_text(r'RAW decode settings match', 'raw-loader-settings-applied')
        raw_state = driver.state_action('Save current to file', root / 'raw-state.json', save=True)
        require(raw_state['input_image']['input_color_space'] == 'ACES2065-1',
                'RAW load did not configure ACES input')
        require(raw_state['input_image']['input_cctf_decoding'] is False, 'RAW load enabled CCTF decode')
        scan_output('raw-scan')
        raw_pixels = save_pixels(root / 'raw-scan.tif', 'raw-scan-saved')
        require(raw_pixels.shape == (512, 768, 3), 'RAW scan dimensions differ')
        driver.records.append({'parity_action': 'load_raw_image', 'scenario': 'raw_status',
                               'raw_input_space': 'ACES2065-1', 'raw_settings_status': raw_status,
                               'automatic_preview_report': raw_preview,
                               'lens_status_scope': 'transient loader text is not retained or asserted',
                               'saved_shape': list(raw_pixels.shape)})
        large = root / 'large.tif'
        large_pixels = np.tile(np.linspace(.05, .8, 4096, dtype=np.float32)[None, :, None], (3072, 1, 3))
        writer = oiio.ImageOutput.create(str(large))
        require(writer and writer.open(str(large), oiio.ImageSpec(4096, 3072, 3, oiio.FLOAT)),
                'Cannot create cancellation input')
        require(writer.write_image(large_pixels) and writer.close(), 'Cannot write cancellation input')
        driver.state_action('Load from file', configured)
        cancelled = root / 'cancelled.exr'
        cancel_export(driver, large, cancelled)
        closed = root / 'closed.exr'
        driver.click('Export', header=True)
        driver.export_options(closed)
        driver.dialog(closed, save=True)
        driver.wait_text(r'Exporting\s+with', 'close-export-in-flight', timeout=20)
        driver.close(exporting=True)
        require(not closed.exists(), 'Closing in-flight export left an output image')
        driver.records.append({'startup_restore': True, 'factory_reset': True,
                               'cancel_output_absent': True, 'close_output_absent': True})
        return {'gui': str(gui), 'evidence': str(root / 'observations.json'),
                'native_acceptance': 'passed', 'native_surface': 'continuous-inspector'}
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
