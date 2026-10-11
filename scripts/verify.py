#!/usr/bin/env python3
"""Run the existing local gate and retain its result with source provenance."""

import argparse
import json
from pathlib import Path
import platform
import subprocess
import sys
from datetime import datetime, timezone
from source_identity import snapshot, source_identity

def timestamp():
    return datetime.now(timezone.utc).isoformat()




def run(root, evidence):
    root = root.resolve()
    evidence = evidence.resolve()
    if evidence == root or root in evidence.parents:
        raise ValueError('Evidence must be outside the repository')
    evidence.mkdir(parents=True, exist_ok=False)
    report = {
        'status': 'running',
        'started_at': timestamp(),
        'repository': str(root),
        'command': ['just', 'ci'],
        'platform': platform.platform(),
        'python_version': platform.python_version(),
        'log_file': 'gate.log',
        'exit_code': None,
    }
    process = None
    exit_code = 1
    try:
        report['before'] = snapshot(root, evidence, 'before')
        report['tool_versions'] = {
            tool: subprocess.check_output(command, cwd=root, stderr=subprocess.STDOUT).decode().strip()
            for tool, command in {
                'rustc': ['rustc', '-Vv'],
                'cargo': ['cargo', '-V'],
                'just': ['just', '--version'],
            }.items()
        }
        (evidence / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        with (evidence / 'gate.log').open('wb') as log:
            process = subprocess.Popen(['just', 'ci'], cwd=root, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT)
            while chunk := process.stdout.read1(65536):
                log.write(chunk)
                log.flush()
                sys.stdout.buffer.write(chunk)
                sys.stdout.buffer.flush()
            returncode = process.wait()
        report['gate_exit_code'] = returncode
        exit_code = returncode if returncode >= 0 else 128 - returncode
        report['after'] = snapshot(root, evidence, 'after')
        report['source_changed'] = source_identity(report['before']) != source_identity(report['after'])
        if report['source_changed'] and exit_code == 0:
            exit_code = 1
        report['status'] = 'pass' if exit_code == 0 else 'fail'
    except BaseException as error:
        exit_code = 130 if isinstance(error, KeyboardInterrupt) else 1
        report['status'] = 'error'
        report['error'] = f'{type(error).__name__}: {error}'
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait()
        raise
    finally:
        report['finished_at'] = timestamp()
        report['exit_code'] = exit_code
        (evidence / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    return exit_code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence_dir', type=Path, help='New directory outside the repository')
    args = parser.parse_args()
    return run(Path(__file__).resolve().parents[1], args.evidence_dir)


if __name__ == '__main__':
    sys.exit(main())
