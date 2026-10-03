#!/usr/bin/env python3
"""Installed egui acceptance through native desktop input, file dialogs and OCR.

Linux: Xvfb/openbox/xdotool/xprop/xwininfo/xclip/zenity/tesseract; Windows/macOS: pyautogui
and Tesseract. macOS requires runner Accessibility/Automation/Screen Recording
permissions and an active desktop. No GUI hooks, exporter override, source-tree
executable or fabricated child is used.
"""
import hashlib
import json
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


def wait_for(predicate, description, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.15)
    raise RuntimeError(f'Timed out waiting for {description} ({timeout}s)')


class X11:
    def __init__(self, root):
        require(sys.platform.startswith('linux'),
                f'Native GUI acceptance driver unavailable for {sys.platform}; this gate cannot be skipped')
        require(os.environ.get('DISPLAY'), 'GUI acceptance requires X11; run under xvfb-run')
        for command in ('xdotool', 'xprop', 'xwininfo', 'xclip', 'openbox', 'zenity', 'tesseract'):
            require(shutil.which(command), f'GUI acceptance requires native dependency: {command}')
        import mss
        import psutil
        import pytesseract
        from PIL import Image
        self.mss, self.psutil, self.ocr, self.Image = mss, psutil, pytesseract, Image
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
        return subprocess.run(['xdotool', *map(str, args)], check=check,
                              capture_output=True, text=True, timeout=15).stdout.strip()

    def start(self, gui, env, image=None):
        command = [str(gui)] + ([str(image)] if image else [])
        self.proc = subprocess.Popen(command, cwd=self.root, env=env, stdout=self.log, stderr=self.log)
        def window():
            require(self.proc.poll() is None, 'Installed GUI exited; see gui.log')
            ids = self.xd('search', '--onlyvisible', '--pid', self.proc.pid, '--name', '^spektrafilm$', check=False)
            return ids.splitlines()[-1] if ids else None
        self.window = wait_for(window, 'installed native window', 45)
        self.xd('windowactivate', '--sync', self.window)
        self.xd('windowsize', '--sync', self.window, 1400, 900)
        self.xd('windowmove', '--sync', self.window, 0, 30)
        wait_for(lambda: self.match(self.read()[2], 'Save state', True)
                 and self.match(self.read()[2], 'Open', True), 'rendered native controls', 30)

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
        from PIL import ImageOps, ImageStat
        lines = []
        # Segment the sidebar from the viewer and normalize each region to
        # dark text on a light background, including native light themes.
        def normalize(region):
            grayscale = ImageOps.grayscale(region)
            background = grayscale.crop((0, 0, grayscale.width, min(50, grayscale.height)))
            return ImageOps.invert(grayscale) if ImageStat.Stat(background).median[0] < 128 else grayscale

        for offset, region in ((0, image.crop((0, 0, 1060, image.height))),
                               (1060, image.crop((1060, 0, image.width, image.height)))):
            prepared = normalize(region)
            if offset:
                prepared = prepared.point(lambda value: 255 if value > 190 else 0)
            data = self.ocr.image_to_data(prepared.resize((region.width * 3, region.height * 3)),
                                          config='--psm 11', output_type=self.ocr.Output.DICT, timeout=15)
            grouped = {}
            for i, text in enumerate(data['text']):
                if text.strip():
                    key = (data['block_num'][i], data['par_num'][i], data['line_num'][i])
                    grouped.setdefault(key, []).append((text, offset + data['left'][i] / 3, data['top'][i] / 3,
                                                         data['width'][i] / 3, data['height'][i] / 3))
            lines.extend(grouped.values())
        region = image.crop((1170, 315, 1240, 350))
        data = self.ocr.image_to_data(normalize(region).resize((420, 210)),
                                      config='--psm 7', output_type=self.ocr.Output.DICT, timeout=15)
        cancel = [(text, 1170 + data['left'][i] / 6, 315 + data['top'][i] / 6,
                   data['width'][i] / 6, data['height'][i] / 6)
                  for i, text in enumerate(data['text']) if text.strip() == 'Cancel']
        if cancel:
            lines.append(cancel)
        region = image.crop((1060, image.height - 15, image.width, image.height))
        data = self.ocr.image_to_data(normalize(region).resize((region.width * 6, 90)),
                                      config='--psm 7', output_type=self.ocr.Output.DICT, timeout=15)
        status = [(text, 1060 + data['left'][i] / 6, image.height - 15 + data['top'][i] / 6,
                   data['width'][i] / 6, data['height'][i] / 6)
                  for i, text in enumerate(data['text']) if text.strip()]
        if status:
            lines.append(status)
        return image, bbox, lines

    @staticmethod
    def match(lines, label, right=False):
        # OCR may transliterate the ellipsis; match words, with explicit boundaries.
        wanted = re.findall(r'[a-z0-9]+', label.lower())
        matches = []
        for line in lines:
            for start in range(len(line)):
                words = [re.sub(r'[^a-z0-9]', '', w[0].lower()) for w in line[start:start + len(wanted)]]
                if words == wanted or (label.lower() == 'auto exposure' and words == ['aueo', 'exposure']):
                    selected = line[start:start + len(wanted)]
                    x = selected[0][1]
                    if right and x < 1000:
                        continue
                    if label.lower() == 'save' and selected[0][2] < 100:
                        continue
                    # Save must not select Save state or Save startup.
                    following = line[start + len(wanted):start + len(wanted) + 1]
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

    def click(self, label, right=True):
        def locate():
            image, _, lines = self.read()
            matches = self.match(lines, label, right)
            if matches:
                self.snap('control-' + label.replace(' ', '-'), image, lines)
                return matches[0]
        x, y = wait_for(locate, f'visible control {label}', 20)
        if label == 'Cancel':
            self.require_export_in_flight('Cancel')
        self.xd('mousemove', '--window', self.window, int(x), int(y))
        self.xd('click', 1)
        time.sleep(.15)

    def scroll(self, bottom):
        self.xd('mousemove', '--window', self.window, 1320, 650)
        self.xd('click', '--repeat', 35, '--delay', 8, 5 if bottom else 4)
        time.sleep(.15)

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
        def find():
            found = self.xd('search', '--onlyvisible', '--class', 'zenity|yad', check=False)
            return found.splitlines()[-1] if found else None
        dialog = wait_for(find, 'native file chooser (zenity/yad)', 25)
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
        self.xd('windowactivate', '--sync', dialog)
        time.sleep(.6)
        self.xd('key', 'ctrl+l')
        time.sleep(.2)
        self.xd('key', 'ctrl+a')
        text = str(path if save else path.parent) + ('' if save else '/')
        # GTK completion consumes synthetic per-character input; paste the
        # complete path atomically through the real desktop clipboard.
        subprocess.run(['xclip', '-selection', 'clipboard'], input=text,
                       text=True, check=True, timeout=10)
        self.xd('key', 'ctrl+v')
        time.sleep(.3)
        self.xd('key', 'Return')
        if not save:
            time.sleep(.6)
            self.xd('key', 'ctrl+l')
            self.xd('key', 'ctrl+a')
            subprocess.run(['xclip', '-selection', 'clipboard'], input=str(path), text=True, check=True)
            self.xd('key', 'ctrl+v')
            time.sleep(.3)
            self.xd('key', 'Return')
        # Save choosers may first navigate the entered full path, then require Save.
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if not self.xd('search', '--onlyvisible', '--class', 'zenity|yad', check=False):
                return
            with self.mss.mss() as screen:
                shot = screen.grab(screen.monitors[0])
            surface = self.Image.frombytes('RGB', shot.size, shot.rgb)
            buttons = self.ocr.image_to_data(surface, config='--psm 11', output_type=self.ocr.Output.DICT)
            for i, word in enumerate(buttons['text']):
                if word.strip() == 'OK':
                    self.xd('mousemove', buttons['left'][i] + buttons['width'][i] // 2,
                            buttons['top'][i] + buttons['height'][i] // 2)
                    self.xd('click', 1)
                    break
            time.sleep(.25)
        raise RuntimeError(f'Native chooser did not accept {path}')

    def file_action(self, control, path, save=False):
        self.scroll(False)
        if control == 'Export':
            self.watch_export()
        self.click(control)
        self.dialog(path, save)

    def rendered(self, label):
        self.scroll(True)
        self.wait_text(r'Rendered\s+\d+', label)

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
        if sys.platform == 'win32' and (screen.width < 1500 or screen.height < 1000):
            self.prepare_display()
            screen = self.input.size()
        require(screen.width >= 1500 and screen.height >= 1000,
                f'Native acceptance needs a desktop at least 1500x1000; actual {screen.width}x{screen.height}. '
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
            if width >= 1500 and height >= 1000:
                candidates.append((width * height, candidate))
            index += 1
        require(candidates, 'Windows display exposes no supported mode at least 1500x1000')
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
            self.apple('set w to first window whose name is "spektrafilm"\nset position of w to {0, 30}\nset size of w to {1400, 900}')
        else:
            rect = self.types.RECT(0, 0, 1400, 900)
            self.os.AdjustWindowRect(self.ctypes.byref(rect), self.os.GetWindowLongW(self.window, -16), False)
            self.os.MoveWindow(self.window, 0, 30, rect.right - rect.left, rect.bottom - rect.top, True)
        wait_for(lambda: self.match(self.read()[2], 'Save state', True)
                 and self.match(self.read()[2], 'Open', True), 'rendered desktop controls', 45)

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
                self.input.click(clicks=count, interval=.01)
        else:
            raise RuntimeError(f'Unsupported desktop input: {args}')
        return ''

    def dialog_visible(self):
        if sys.platform == 'darwin':
            # Synchronous rfd dialogs without set_parent use runModal(), so
            # NSSavePanel/NSOpenPanel are standalone AXDialog windows.
            return self.apple('''repeat with i from 1 to count windows
    set w to window i
    if exists sheet 1 of w then
        set sheetElements to (get entire contents of sheet 1 of w)
        repeat with elementReference in sheetElements
            set element to contents of elementReference
            if role of element is "AXButton" then
                if name of element is "Save" or name of element is "Open" then return "sheet 1 of window " & i
            end if
        end repeat
    end if
    set panelSubrole to subrole of w
    if panelSubrole is "AXDialog" or panelSubrole is "AXSystemDialog" or name of w is not "spektrafilm" then
        set hasAction to false
        set hasCancel to false
        set windowElements to (get entire contents of w)
        repeat with elementReference in windowElements
            set element to contents of elementReference
            if role of element is "AXButton" then
                if name of element is "Save" or name of element is "Open" then set hasAction to true
                if name of element is "Cancel" then set hasCancel to true
            end if
        end repeat
        if hasAction and hasCancel then return "window " & i
    end if
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
                self.mac_diagnostics('file-chooser')
                self.input.hotkey('command', 'shift', 'g')
                time.sleep(.3)
                self.input.hotkey('command', 'a')
                self.paste(path.parent if save else path)
                self.input.press('enter')
                # Go to Folder is its own sheet, including on a standalone
                # NSSavePanel. Do not write the name until that sheet closes.
                def navigated():
                    return self.apple(f'not (exists sheet 1 of {chooser})') == 'true'
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
            if description of element contains "Save As" then set nameField to element
        end try
    end if
end repeat
if nameField is missing value and (count candidates) is 1 then set nameField to item 1 of candidates
if nameField is missing value then error ("Cannot identify Save As field; AXTextField count=" & (count candidates))
set focused of nameField to true''')
                    self.input.hotkey('command', 'a')
                    self.paste(path.name)
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
                'SPEKTRAFILM_PY', 'SPEKTRAFILM_PY_REPO', 'DBUS_SESSION_BUS_ADDRESS'):
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
    try:
        driver.start(gui, env)
        baseline = root / 'baseline.json'
        driver.file_action('Save state', baseline, True)
        wait_for(baseline.is_file, 'saved baseline state', 20)
        state = json.loads(baseline.read_text())
        state['grain']['active'] = False
        state['halation']['active'] = False
        state['glare']['active'] = False
        state['input_image']['input_color_space'] = 'sRGB'
        state['display']['preview_max_size'] = 256
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
        driver.file_action('Open', standard)
        driver.rendered('standard-image-preview')
        driver.scroll(False)
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
        require(difference > .2, 'Input/output controls did not change viewer pixels')
        require(float(output_pixels.std()) > 5, 'Output viewer lost gradient contrast')
        driver.records.append({'viewer_roi': list(roi), 'input_raster': 'input-viewer-raster.png',
                               'output_raster': 'output-viewer-raster.png', 'mean_abs_pixel_change': difference,
                               'output_pixel_std': float(output_pixels.std())})
        driver.click('Auto exposure')
        driver.rendered('changed-auto-exposure-preview')
        driver.scroll(False)
        driver.click('16 bit')
        driver.click('32 bit')
        saved_state = root / 'roundtrip.json'
        driver.file_action('Save state', saved_state, True)
        wait_for(saved_state.is_file, 'saved changed state', 20)
        saved = json.loads(saved_state.read_text())
        require(saved['camera']['auto_exposure'] is True, 'Control change absent from saved state')
        require(saved['rust']['save_bit_depth'] == 32, 'Float depth selection absent from state')
        driver.file_action('Load state', configured)
        driver.rendered('reloaded-original-state')
        driver.file_action('Load state', saved_state)
        driver.rendered('loaded-changed-state')
        driver.scroll(False)
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
        for section in ('camera', 'simulation', 'input_image', 'grain', 'halation'):
            require(restored[section] == saved[section], f'Restart changed {section}')
        require(restored['rust']['save_bit_depth'] == 32, 'Restart lost save depth')
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
        require(full.shape == preview.shape, 'GUI save/full-export image dimensions differ')
        driver.no_children()
        driver.records.append({'preview_shape': list(preview.shape), 'export_shape': list(full.shape),
                               'preview_sha256': hashlib.sha256(float_path.read_bytes()).hexdigest(),
                               'export_sha256': hashlib.sha256(exported_path.read_bytes()).hexdigest()})
        raw_input = root / ('raw-input' + raw.suffix.lower())
        shutil.copyfile(raw, raw_input)
        driver.file_action('Open', raw_input)
        driver.rendered('raw-image-preview')
        driver.file_action('Save state', root / 'raw-state.json', True)
        wait_for((root / 'raw-state.json').is_file, 'RAW state', 20)
        raw_state = json.loads((root / 'raw-state.json').read_text())
        require(raw_state['input_image']['input_color_space'] == 'ACES2065-1', 'RAW load did not configure ACES input')
        # A real large image keeps the spectral child in flight long enough to
        # exercise Cancel and window close. No fake exporter or timing hook.
        large = root / 'large.tif'
        pixels = np.tile(np.linspace(.05, .8, 4096, dtype=np.float32)[None, :, None], (3072, 1, 3))
        writer = oiio.ImageOutput.create(str(large))
        require(writer and writer.open(str(large), oiio.ImageSpec(4096, 3072, 3, oiio.FLOAT)), 'Cannot create cancellation input')
        require(writer.write_image(pixels) and writer.close(), 'Cannot write cancellation input')
        driver.file_action('Load state', saved_state)
        driver.file_action('Open', large)
        driver.rendered('large-image-preview')
        cancelled = root / 'cancelled.exr'
        driver.file_action('Export', cancelled, True)
        driver.capture_export_child(exporter)
        driver.snap('cancel-export-in-flight')
        driver.click('Cancel')
        driver.scroll(True)
        driver.wait_text(r'Export cancelled', 'cancelled-export', timeout=30)
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
