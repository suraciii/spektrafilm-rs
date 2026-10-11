#!/usr/bin/env python3
"""Reject Python bytecode and cache directories in the Git index."""

import os
from pathlib import PurePosixPath
import subprocess
import sys


def main():
    tracked = subprocess.check_output(['git', 'ls-files', '-z']).split(b'\0')
    caches = []
    for name in tracked:
        if not name:
            continue
        path = PurePosixPath(os.fsdecode(name))
        if '__pycache__' in path.parts or path.suffix in ('.pyc', '.pyo'):
            caches.append(str(path))
    if caches:
        print('Python cache files must not be tracked:', file=sys.stderr)
        for path in caches:
            print(f'  {path}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
