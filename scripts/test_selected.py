#!/usr/bin/env python3
"""Build selected libtest targets together; Cargo still owns the full suite."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
# Package, target kind/name, and the original positive libtest selections.
SELECTIONS = (
    ('gpu', 'lib', 'spektrafilm_gpu', ('logic_tests::',)),
    ('math', 'lib', 'spektrafilm_math', ()),
    ('model', 'lib', 'spektrafilm_model', (
        'density_curves::', 'illuminants::', 'couplers::', 'glare::',
        'grain::spektra::', 'grain::fotufilm::', 'diffusion::')),
    ('core', 'lib', 'spektrafilm_core', (
        'params::sources::', 'params::tests::', 'params::validation::',
        'params_builder::', 'pipeline_calibration::tests::', 'resizing::',
        'suggest::', 'cmyk::', 'input_gamut::', 'gamut_compression::',
        'lut_formats::', 'lut_ocio::', 'lut_delivery::', '--skip', 'process_params::')),
    ('core', 'test', 'telemetry', ()),
    ('gui', 'bin', 'spektrafilm-gui', ('state::tests::', 'numeric::tests::')),
)


def artifact_key(message):
    target = message['target']
    return message['package_id'], tuple(target['kind']), target['name']


def collect_artifacts(messages):
    artifacts = {}
    for message in messages:
        if (message.get('reason') == 'compiler-artifact'
                and message['profile']['test'] and message.get('executable')):
            key = artifact_key(message)
            if key in artifacts:
                raise ValueError(f'Duplicate test artifact: {key}')
            artifacts[key] = message
    return artifacts


def build(arguments):
    command = ['cargo', 'test', '--locked', *arguments, '--message-format=json']
    print('+ ' + ' '.join(command), flush=True)
    # A file avoids buffering the whole workspace output or hiding test failures.
    started = time.monotonic()
    with tempfile.TemporaryFile(mode='w+t') as records:
        process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE,
                                   text=True)
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line, end='', flush=True)
                continue
            if not isinstance(message, dict) or 'reason' not in message:
                print(line, end='', flush=True)
                continue
            records.write(line)
            if message['reason'] == 'compiler-message':
                rendered = message['message'].get('rendered')
                if rendered:
                    print(rendered, end='', file=sys.stderr, flush=True)
        result = process.wait()
        if result:
            raise subprocess.CalledProcessError(result, command)
        records.seek(0)
        messages = [json.loads(line) for line in records]
    if not any(m.get('reason') == 'build-finished' and m.get('success')
               for m in messages):
        raise ValueError('Cargo did not report a successful build')
    print(f'Cargo invocation wall: {time.monotonic() - started:.3f}s', flush=True)
    return collect_artifacts(messages), messages


def selected_artifacts(artifacts, packages, selections):
    selected = []
    for package, kind, name, filters in selections:
        metadata = packages['spektrafilm-' + package]
        key = metadata['id'], (kind,), name
        if key not in artifacts:
            raise ValueError(f'Missing selected test artifact: {key}')
        artifact = artifacts[key]
        # These explicit targets are libtest harnesses; never invoke other
        # workspace executables or infer targets from old files in target/.
        if not any(t['name'] == name and t['kind'] == [kind] and t['test']
                   for t in metadata['targets']):
            raise ValueError(f'Not a declared test target: {key}')
        selected.append((metadata, artifact, filters, package == 'gpu'))
    return selected


def run_selected(artifacts, messages, metadata, selections):
    packages = {p['name']: p for p in metadata['packages']}
    # Resolve the entire selection before running any target.
    selected = selected_artifacts(artifacts, packages, selections)
    target_dir = Path(metadata['target_directory']).resolve()
    rust_libraries = subprocess.check_output(
        ['rustc', '--print', 'target-libdir'], cwd=ROOT, text=True).strip()
    for package, artifact, filters, device_free in selected:
        executable = Path(artifact['executable']).resolve(strict=True)
        if not executable.is_relative_to(target_dir):
            raise ValueError(f'Test executable outside Cargo target directory: {executable}')
        environment = os.environ.copy()
        environment['CARGO_MANIFEST_DIR'] = str(Path(package['manifest_path']).parent)
        if device_free:
            environment['SPEKTRAFILM_TEST_NO_DEVICE'] = '1'
        # Mirror Cargo's native-library search additions; retain external paths
        # supplied by the fixed image (notably /opt/libraw/lib).
        paths = [str(executable.parent), str(executable.parent.parent), rust_libraries]
        for message in messages:
            if message.get('reason') == 'build-script-executed':
                for value in message.get('linked_paths', []):
                    path = Path(value.split('=', 1)[-1]).resolve()
                    if path.is_relative_to(target_dir):
                        paths.append(str(path))
        variable = ('PATH' if os.name == 'nt' else
                    'DYLD_FALLBACK_LIBRARY_PATH' if sys.platform == 'darwin' else
                    'LD_LIBRARY_PATH')
        existing = environment.get(variable)
        if existing is None and sys.platform == 'darwin':
            existing = os.path.expanduser('~/lib') + ':/usr/local/lib:/usr/lib'
        environment[variable] = os.pathsep.join(dict.fromkeys(paths + ([existing] if existing else [])))
        command = [str(executable), *filters]
        print('+ ' + ('SPEKTRAFILM_TEST_NO_DEVICE=1 ' if device_free else '')
              + ' '.join(command), flush=True)
        started = time.monotonic()
        subprocess.run(command, cwd=Path(package['manifest_path']).parent,
                       env=environment, check=True)
        print(f'Selected target wall: {time.monotonic() - started:.3f}s', flush=True)


def group_arguments(precision):
    arguments = []
    for package in dict.fromkeys(item[0] for item in SELECTIONS):
        arguments.extend(['-p', 'spektrafilm-' + package])
    arguments.extend(['--lib', '--bin', 'spektrafilm-gui', '--test', 'telemetry', '--no-run'])
    if precision == 'f64':
        arguments.extend(['--features', 'precision-f64'])
    return arguments


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('ci', 'fast', 'tail', 'logic'))
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=ROOT))
    if args.mode == 'logic':
        for features in ([], ['--features', 'precision-f64']):
            artifacts, messages = build(
                ['-p', 'spektrafilm-gpu', '--lib', '--no-run', *features])
            run_selected(artifacts, messages, metadata, SELECTIONS[:1])
        return
    if args.mode == 'ci':
        artifacts, messages = build(['--workspace'])
        run_selected(artifacts, messages, metadata, SELECTIONS[:1])
    elif args.mode == 'tail':
        artifacts, messages = build(['--workspace', '--no-run'])
        run_selected(artifacts, messages, metadata, SELECTIONS[:1])
    else:
        artifacts, messages = build(group_arguments('f32'))
        run_selected(artifacts, messages, metadata, SELECTIONS)
    artifacts, messages = build(group_arguments('f64'))
    run_selected(artifacts, messages, metadata, SELECTIONS)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'Selected tests failed: {error}', file=sys.stderr)
        sys.exit(1)
