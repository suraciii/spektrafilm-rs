#!/usr/bin/env python3
"""Compile repository Python source in memory without writing bytecode."""

import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]


def main():
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=ROOT,
    ).split(b'\0')
    failures = 0
    checked = 0
    for name in sorted(set(names)):
        path = ROOT / os.fsdecode(name)
        if not name or path.suffix != '.py' or not path.is_file():
            continue
        checked += 1
        try:
            compile(path.read_bytes(), str(path.relative_to(ROOT)), 'exec', dont_inherit=True)
        except (SyntaxError, UnicodeError, ValueError) as error:
            failures += 1
            print(f'{path.relative_to(ROOT)}: {error}', file=sys.stderr)
    print(f'Python syntax: {checked} files checked, {failures} failed; no bytecode written.')
    return int(failures != 0)


if __name__ == '__main__':
    sys.exit(main())
