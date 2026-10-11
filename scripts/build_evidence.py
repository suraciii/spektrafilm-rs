#!/usr/bin/env python3
"""Build Cargo artifacts and bind builds, packages and native runs to exact source."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile

from source_identity import snapshot, source_identity

ROOT = Path(__file__).resolve().parents[1]
BINS = {'gui': 'spektrafilm-gui', 'cli': 'spektrafilm',
        'exporter': 'spektrafilm-f64', 'raw_helper': 'decode_raw_gui'}


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def tree_hash(path):
    digest = hashlib.sha256()
    for item in sorted(p for p in Path(path).rglob('*') if p.is_file()):
        digest.update(item.relative_to(path).as_posix().encode())
        digest.update(b'\0')
        digest.update(bytes.fromhex(sha256(item)))
    return digest.hexdigest()


def current_source(root=ROOT):
    with tempfile.TemporaryDirectory(prefix='spektrafilm-source-') as directory:
        return source_identity(snapshot(Path(root), Path(directory), 'source'))


def revision(source):
    if not source['status']:
        return source['commit']
    digest = hashlib.sha256(json.dumps(source, sort_keys=True).encode()).hexdigest()
    return source['commit'] + '-dirty-' + digest


def verify_manifest(manifest, *, verify_source=True):
    if manifest.get('schema_version') != 1:
        raise ValueError('Unsupported build manifest schema')
    if manifest['source_revision'] != revision(manifest['source']):
        raise ValueError('Manifest source revision does not match its source identity')
    source_root = Path(manifest['repository'])
    if verify_source and manifest['kind'] == 'build' and current_source(source_root) != manifest['source']:
        raise ValueError('Current source differs from the binary build source')
    if set(manifest['binaries']) != set(BINS):
        raise ValueError('Build manifest must contain all four executables')
    for label, binary in manifest['binaries'].items():
        if sha256(binary['path']) != binary['sha256']:
            raise ValueError(f'Binary changed: {label}: {binary["path"]}')
    if tree_hash(Path(manifest['data_dir'])) != manifest['data_sha256']:
        raise ValueError('Build data changed')
    for relative, expected in manifest.get('package_files', {}).items():
        if sha256(Path(manifest['package_root']) / relative) != expected:
            raise ValueError(f'Package launcher changed: {relative}')
    for label, path in manifest.get('launch_paths', {}).items():
        binary_path = manifest['binaries'][label]['path']
        if path != binary_path:
            relative = Path(path).relative_to(manifest['package_root']).as_posix()
            if relative not in manifest.get('package_files', {}):
                raise ValueError(f'Package launch path is not attested: {label}')


def load_manifest(path, *, verify_source=True):
    path = Path(path).resolve()
    manifest = json.loads(path.read_text(encoding='utf-8'))
    if manifest['kind'] == 'package':
        manifest['package_root'] = str(path.parent)
    # Packages retain portable relative paths; build artifacts have absolute paths.
    for binary in manifest['binaries'].values():
        binary['path'] = str((path.parent / binary['path']).resolve())
    manifest['launch_paths'] = {label: str((path.parent / value).resolve())
                                for label, value in manifest.get('launch_paths', {}).items()}
    manifest['data_dir'] = str((path.parent / manifest['data_dir']).resolve())
    verify_manifest(manifest, verify_source=verify_source)
    return manifest


def executable_path(manifest, label):
    return Path(manifest.get('launch_paths', {}).get(label, manifest['binaries'][label]['path']))


RECORDED_ENVIRONMENT = frozenset({
    'SPEKTRAFILM_DATA_DIR', 'SPEKTRAFILM_CONFIG_DIR', 'SPEKTRAFILM_BACKEND',
    'SPEKTRAFILM_GUI_RENDERER', 'SPEKTRAFILM_GUI_DIAGNOSTICS_DIR',
    'DISPLAY', 'WAYLAND_DISPLAY', 'XDG_CURRENT_DESKTOP', 'XDG_SESSION_TYPE',
    'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'XDG_DATA_HOME', 'XDG_RUNTIME_DIR',
    'HOME', 'TMPDIR', 'TEMP', 'TMP', 'APPDATA', 'LOCALAPPDATA',
    'LD_LIBRARY_PATH', 'DYLD_LIBRARY_PATH', 'VK_ICD_FILENAMES',
    'LIBGL_ALWAYS_SOFTWARE', 'WGPU_BACKEND', 'LANG', 'LC_ALL',
})


def freeze_run(manifest, evidence_dir, *, driver_paths=(), fixtures=(), environment=None):
    verify_manifest(manifest)
    files = []
    for fixture in fixtures:
        entry = dict(fixture) if isinstance(fixture, dict) else {'path': fixture}
        entry['path'] = str(Path(entry['path']).resolve())
        entry['sha256'] = sha256(entry['path'])
        files.append(entry)
    identity = {'build': manifest, 'drivers': [
        {'path': str(Path(path).resolve()), 'sha256': sha256(path)} for path in driver_paths],
        'fixtures': files, 'environment': {
            name: {key: value for key, value in values.items() if key in RECORDED_ENVIRONMENT}
            for name, values in (environment or {}).items()},
        'driver_source': current_source(), 'driver_repository': str(ROOT),
        'process_environment_sha256': hashlib.sha256(
            json.dumps(dict(os.environ), sort_keys=True).encode()).hexdigest(),
        'platform': platform.platform(), 'python_version': platform.python_version()}
    Path(evidence_dir, 'run_identity.json').write_text(json.dumps(identity, indent=2) + '\n')
    return identity


def finalize_run(identity, *, report_paths=()):
    verify_manifest(identity['build'])
    if current_source(Path(identity['driver_repository'])) != identity['driver_source']:
        raise ValueError('Native driver source changed during run')
    environment_hash = hashlib.sha256(json.dumps(dict(os.environ), sort_keys=True).encode()).hexdigest()
    if environment_hash != identity['process_environment_sha256']:
        raise ValueError('Driver process environment changed during run')
    for entry in identity['drivers'] + identity['fixtures']:
        if sha256(entry['path']) != entry['sha256']:
            raise ValueError(f'Run input changed: {entry["path"]}')
    for path in report_paths:
        report = json.loads(Path(path).read_text(encoding='utf-8'))
        if report.get('build', {}).get('source_revision') != identity['build']['source_revision']:
            raise ValueError(f'Native report build source differs: {path}')


def build(out_dir, profile='release', target=None, require_clean=False):
    out_dir = Path(out_dir).resolve()
    if out_dir == ROOT or ROOT in out_dir.parents:
        raise ValueError('Build evidence must be outside the repository')
    out_dir.mkdir(parents=True, exist_ok=False)
    before = source_identity(snapshot(ROOT, out_dir, 'before'))
    if require_clean and before['status']:
        raise ValueError('Build requires a clean source worktree')
    source_revision = revision(before)
    env = dict(os.environ, SPEKTRAFILM_SOURCE_REVISION=source_revision)
    versions = {tool: subprocess.check_output(command, text=True).strip() for tool, command in
                {'cargo': ['cargo', '-V'], 'rustc': ['rustc', '-Vv']}.items()}
    host = next(line.split(': ', 1)[1] for line in versions['rustc'].splitlines()
                if line.startswith('host: '))
    selected_target = target or env.get('CARGO_BUILD_TARGET')
    target = selected_target or host
    binaries = {}
    for labels, package, features in [(['gui'], 'spektrafilm-gui', []),
                                      (['cli', 'raw_helper'], 'spektrafilm-cli', []),
                                      (['exporter'], 'spektrafilm-cli', ['precision-f64'])]:
        command = ['cargo', 'build', '--locked', '--message-format=json-render-diagnostics',
                   '--profile', profile, '-p', package,
                   '--target-dir', str(Path(str(env.get('CARGO_TARGET_DIR', out_dir / 'native-target'))
                                           + ('-f64' if features else '')))]
        if selected_target:
            command += ['--target', selected_target]
        for label in labels:
            command += ['--bin', BINS[label]]
        if features:
            command += ['--features', ','.join(features)]
        log_path = out_dir / (labels[0] + '.cargo.jsonl')
        with log_path.open('w', encoding='utf-8') as log:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log)
        if result.returncode:
            raise RuntimeError(f'Cargo failed ({result.returncode}); see {log_path}')
        for line in log_path.read_text().splitlines():
            artifact = json.loads(line)
            if artifact.get('reason') != 'compiler-artifact' or not artifact.get('executable'):
                continue
            for label in labels:
                if artifact['target']['name'] == BINS[label]:
                    path = Path(artifact['executable']).resolve()
                    binaries[label] = {'path': str(path), 'sha256': sha256(path),
                                       'features': artifact['features'], 'target': target,
                                       'profile': artifact['profile'], 'command': command}
        if not all(label in binaries for label in labels):
            raise RuntimeError(f'Cargo omitted executable artifacts: {labels}')
    after = source_identity(snapshot(ROOT, out_dir, 'after'))
    if before != after:
        raise ValueError('Source changed during build; artifacts cannot be attested')
    manifest = {'schema_version': 1, 'kind': 'build', 'source': before,
                'source_revision': source_revision, 'repository': str(ROOT),
                'binaries': binaries, 'data_dir': str(ROOT / 'data'),
                'data_sha256': tree_hash(ROOT / 'data'), 'tool_versions': versions,
                'target': target, 'profile': profile,
                'created_at': datetime.now(timezone.utc).isoformat()}
    verify_manifest(manifest)
    path = out_dir / 'manifest.json'
    path.write_text(json.dumps(manifest, indent=2) + '\n')
    return path


def package_manifest(source_path, package_root, layout, transformations):
    source = load_manifest(source_path)
    root = Path(package_root).resolve()
    prefixes = {'flat': ('', 'data'), 'linux': ('lib/spektrafilm', 'share/data'),
                'macos': ('Contents/MacOS', 'Contents/Resources/data')}
    binary_dir, data_dir = prefixes[layout]
    if layout == 'macos':
        binary_dir = str(Path(root.name) / binary_dir)
        data_dir = str(Path(root.name) / data_dir)
        root = root.parent
    manifest = dict(source, kind='package', data_dir=data_dir)
    manifest['binaries'] = {}
    for label, original in source['binaries'].items():
        suffix = Path(original['path']).suffix if Path(original['path']).suffix == '.exe' else ''
        relative = Path(binary_dir) / (BINS[label] + suffix)
        final_hash = sha256(root / relative)
        if final_hash != original['sha256'] and not transformations:
            raise ValueError(f'Unrecorded package transformation: {label}')
        manifest['binaries'][label] = dict(original, path=relative.as_posix(),
            sha256=final_hash, source_sha256=original['sha256'],
            source_path=original['path'], transformations=transformations)
    manifest['package_files'] = {}
    manifest['launch_paths'] = {label: binary['path'] for label, binary in manifest['binaries'].items()}
    if layout == 'linux':
        for label, name in BINS.items():
            relative = 'bin/' + name
            manifest['package_files'][relative] = sha256(root / relative)
            manifest['launch_paths'][label] = relative
    if tree_hash(root / data_dir) != source['data_sha256']:
        raise ValueError('Packaged data differs from build data')
    path = root / 'build-manifest.json'
    path.write_text(json.dumps(manifest, indent=2) + '\n')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out-dir', type=Path)
    parser.add_argument('--profile', choices=['dev', 'release'], default='release')
    parser.add_argument('--target')
    parser.add_argument('--require-clean', action='store_true')
    parser.add_argument('--manifest', type=Path)
    parser.add_argument('--binary', choices=list(BINS))
    parser.add_argument('--package-root', type=Path)
    parser.add_argument('--layout', choices=['flat', 'linux', 'macos'], default='flat')
    parser.add_argument('--transformation', action='append', default=[])
    args = parser.parse_args()
    if args.manifest:
        manifest = load_manifest(args.manifest)
        if args.package_root:
            print(package_manifest(args.manifest, args.package_root, args.layout, args.transformation))
        elif args.binary:
            print(manifest['binaries'][args.binary]['path'])
        else:
            print(args.manifest.resolve())
    elif args.out_dir:
        print(build(args.out_dir, args.profile, args.target, args.require_clean))
    else:
        parser.error('--out-dir or --manifest is required')


if __name__ == '__main__':
    main()
