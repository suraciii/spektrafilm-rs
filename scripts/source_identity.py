"""Exact Git worktree identity shared by gates and build evidence."""
import hashlib
import os
from pathlib import Path
import subprocess


def git(root, *args):
    return subprocess.check_output(['git', *args], cwd=root)


def snapshot(root, evidence, label):
    root, evidence = Path(root), Path(evidence)
    diff = git(root, 'diff', '--binary', 'HEAD', '--')
    (evidence / f'{label}.diff').write_bytes(diff)
    untracked = {}
    for name in git(root, 'ls-files', '--others', '--exclude-standard', '-z').split(b'\0'):
        if name:
            path = root / os.fsdecode(name)
            content = os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes()
            untracked[os.fsdecode(name)] = hashlib.sha256(content).hexdigest()
    return {
        'commit': git(root, 'rev-parse', 'HEAD').decode().strip(),
        'tree': git(root, 'rev-parse', 'HEAD^{tree}').decode().strip(),
        'status': git(root, 'status', '--porcelain=v1').decode(),
        'diff_file': f'{label}.diff',
        'diff_sha256': hashlib.sha256(diff).hexdigest(),
        'untracked_sha256': untracked,
    }


def source_identity(state):
    return {key: value for key, value in state.items() if key != 'diff_file'}
