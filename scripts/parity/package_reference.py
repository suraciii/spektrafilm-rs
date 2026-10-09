#!/usr/bin/env python3
"""Generate the package spectral oracle using only pinned upstream Python.

Run with the pinned runtime interpreter:
  /tmp/spektrafilm-upstream-034/.venv/bin/python scripts/parity/package_reference.py \
    --repo /tmp/spektrafilm-upstream-28bf \
    --out-dir scripts/parity/fixtures/package_28bf883
"""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys

UPSTREAM = '28bf883e1672e884307edc75852549376e13644e'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n')


def generate(repo, out_dir):
    repo = repo.resolve()
    head = subprocess.run(['git', '-C', str(repo), 'rev-parse', 'HEAD'],
                          capture_output=True, text=True, check=True).stdout.strip()
    if head != UPSTREAM:
        raise ValueError(f'Upstream is {head}; expected {UPSTREAM}')
    subprocess.run(['git', '-C', str(repo), 'diff', '--quiet', 'HEAD', '--', 'src'], check=True)
    sys.path.insert(0, str(repo / 'src'))
    import numpy as np
    from spektrafilm import digest_params, init_params
    from spektrafilm.runtime.pipeline import SimulationPipeline

    # Read the actual package scenario, keeping its parameter overrides authoritative.
    smoke = Path(__file__).with_name('package_smoke.py')
    tree = ast.parse(smoke.read_text())
    overrides = [ast.literal_eval(node.args[0].args[0]) for node in ast.walk(tree)
                 if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                 and isinstance(node.func.value, ast.Name)
                 and node.func.value.id == 'spectral_params' and node.func.attr == 'write_text']
    if len(overrides) != 1:
        raise ValueError('Expected exactly one spectral parameter literal in package_smoke.py')
    overrides = overrides[0]
    params = init_params('kodak_portra_400', 'kodak_portra_endura')

    def apply(obj, fields):
        for name, value in fields.items():
            if isinstance(value, dict):
                apply(getattr(obj, name), value)
            else:
                setattr(obj, name, tuple(value) if isinstance(getattr(obj, name), tuple) else value)

    apply(params, overrides)
    pixels = np.full((1, 1, 3), 0.184, dtype=np.float32)
    result = SimulationPipeline(digest_params(params)).process(pixels)
    module_sources = {}
    for name, module in tuple(sys.modules.items()):
        if name == 'spektrafilm' or name.startswith('spektrafilm.'):
            source = getattr(module, '__file__', None)
            if source:
                path = Path(source).resolve()
                if not path.is_relative_to(repo / 'src'):
                    raise ValueError(f'{name} loaded outside pinned upstream: {source}')
                module_sources[str(path.relative_to(repo))] = sha(path)
    reference = {'upstream_commit': UPSTREAM, 'film': 'kodak_portra_400',
                 'paper': 'kodak_portra_endura', 'params': overrides,
                 'input_dtype': str(pixels.dtype), 'input_rgb': pixels.tolist(),
                 'output_dtype': str(result.dtype), 'output_rgb': result.tolist()}
    out_dir.mkdir(parents=True, exist_ok=True)
    save_json(out_dir / 'spectral.json', reference)
    data_root = repo / 'src' / 'spektrafilm' / 'data'
    data_files = sorted(path for path in data_root.rglob('*')
                        if path.is_file() and '__pycache__' not in path.parts)
    data_hash = hashlib.sha256()
    for path in data_files:
        data_hash.update(str(path.relative_to(data_root)).encode() + b'\0')
        data_hash.update(path.read_bytes())
        data_hash.update(b'\0')
    provenance = {
        'upstream_url': 'https://github.com/andreavolpato/spektrafilm',
        'upstream_commit': UPSTREAM,
        'generation_command': shlex.join([sys.executable, *sys.argv]),
        'python_version': sys.version, 'numpy_version': np.__version__,
        'oracle': 'SimulationPipeline(digest_params(init_params with package spectral overrides)).process(float32 input)',
        'source_sha256': module_sources,
        'scenario_source_sha256': sha(smoke),
        'generator_sha256': sha(Path(__file__)),
        'profile_and_preset_data': {'root': str(data_root.relative_to(repo)),
                                    'files': len(data_files), 'tree_sha256': data_hash.hexdigest()},
        'artifact_sha256': {'spectral.json': sha(out_dir / 'spectral.json')},
    }
    save_json(out_dir / 'provenance.json', provenance)
    print(json.dumps({'reference': reference, 'artifact_sha256': provenance['artifact_sha256'],
                      'profile_and_preset_data': provenance['profile_and_preset_data']}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--out-dir', type=Path, required=True)
    args = parser.parse_args()
    generate(args.repo, args.out_dir)
