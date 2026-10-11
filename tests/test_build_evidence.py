"""Offline provenance regressions; no Cargo or native desktop required."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import build_evidence as evidence


class BuildEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.data = self.root / 'data'
        self.data.mkdir()
        (self.data / 'profile').write_bytes(b'asset')
        self.source = {'commit': 'a' * 40, 'tree': 'b' * 40, 'status': '',
                       'diff_sha256': evidence.hashlib.sha256(b'').hexdigest(),
                       'untracked_sha256': {}}
        binaries = {}
        for label, name in evidence.BINS.items():
            path = self.root / name
            path.write_bytes(label.encode())
            binaries[label] = {'path': str(path), 'sha256': evidence.sha256(path)}
        self.manifest = {'schema_version': 1, 'kind': 'build', 'source': self.source,
                         'source_revision': evidence.revision(self.source),
                         'repository': str(self.root), 'binaries': binaries,
                         'data_dir': str(self.data), 'data_sha256': evidence.tree_hash(self.data)}

    def test_old_binary_source_cannot_be_labeled_current_head(self):
        changed = dict(self.source, commit='c' * 40)
        with patch.object(evidence, 'current_source', return_value=changed):
            with self.assertRaisesRegex(ValueError, 'build source'):
                evidence.verify_manifest(self.manifest)
        forged = copy.deepcopy(self.manifest)
        forged['source_revision'] = changed['commit']
        with self.assertRaisesRegex(ValueError, 'source revision'):
            evidence.verify_manifest(forged, verify_source=False)

    def test_replaced_binary_and_data_are_rejected(self):
        Path(self.manifest['binaries']['gui']['path']).write_bytes(b'replaced')
        with self.assertRaisesRegex(ValueError, 'Binary changed'):
            evidence.verify_manifest(self.manifest, verify_source=False)
        self.manifest['binaries']['gui']['sha256'] = evidence.sha256(self.root / 'spektrafilm-gui')
        (self.data / 'profile').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'data changed'):
            evidence.verify_manifest(self.manifest, verify_source=False)

    def test_run_rejects_source_mutation_even_without_native_reports(self):
        with patch.object(evidence, 'current_source', return_value=self.source):
            identity = evidence.freeze_run(self.manifest, self.root)
        with patch.object(evidence, 'current_source', return_value=dict(self.source, status=' M input')):
            with self.assertRaisesRegex(ValueError, 'source'):
                evidence.finalize_run(identity)

    def test_frozen_environment_excludes_inherited_credentials(self):
        secret = 'credential-that-must-not-enter-evidence'
        environment = {'DISPLAY': ':99', 'GH_TOKEN': secret, 'AWS_SECRET_ACCESS_KEY': secret}
        with patch.dict(os.environ, {'GH_TOKEN': secret}), \
                patch.object(evidence, 'current_source', return_value=self.source):
            identity = evidence.freeze_run(self.manifest, self.root,
                                           environment={'chooser': environment})
        serialized = (self.root / 'run_identity.json').read_text()
        self.assertNotIn(secret, serialized)
        self.assertNotIn('GH_TOKEN', serialized)
        self.assertEqual(identity['environment'], {'chooser': {'DISPLAY': ':99'}})
        self.assertEqual(len(identity['process_environment_sha256']), 64)

    def test_package_wrapper_is_admitted_and_replacement_rejected(self):
        launcher = self.root / 'bin/spektrafilm-gui'
        launcher.parent.mkdir()
        launcher.write_text('#!/bin/sh\nexec ../spektrafilm-gui "$@"\n')
        package = dict(self.manifest, kind='package', package_root=str(self.root),
                       launch_paths={'gui': str(launcher)},
                       package_files={'bin/spektrafilm-gui': evidence.sha256(launcher)})
        evidence.verify_manifest(package)
        self.assertEqual(evidence.executable_path(package, 'gui'), launcher)
        launcher.write_text('#!/bin/sh\nexit 0\n')
        with self.assertRaisesRegex(ValueError, 'Package launcher changed'):
            evidence.verify_manifest(package)

    def test_run_rejects_native_report_from_different_compilation(self):
        report = self.root / 'operation.json'
        report.write_text(json.dumps({'build': {'source_revision': 'c' * 40}}))
        with patch.object(evidence, 'current_source', return_value=self.source):
            identity = evidence.freeze_run(self.manifest, self.root)
            with self.assertRaisesRegex(ValueError, 'Native report build source differs'):
                evidence.finalize_run(identity, report_paths=[report])

    def test_run_rejects_replacement_after_launch(self):
        with patch.object(evidence, 'current_source', return_value=self.source):
            identity = evidence.freeze_run(self.manifest, self.root)
            Path(self.manifest['binaries']['cli']['path']).write_bytes(b'new compilation')
            with self.assertRaisesRegex(ValueError, 'Binary changed'):
                evidence.finalize_run(identity)

    def test_packaged_transform_retains_source_hash_and_is_relocatable(self):
        source_path = self.root / 'manifest.json'
        source_path.write_text(json.dumps(self.manifest))
        package = self.root / 'package'
        package.mkdir()
        (package / 'data').mkdir()
        (package / 'data/profile').write_bytes(b'asset')
        for label, name in evidence.BINS.items():
            (package / name).write_bytes(label.encode() + b'patchelf')
        with patch.object(evidence, 'current_source', return_value=self.source):
            with self.assertRaisesRegex(ValueError, 'Unrecorded'):
                evidence.package_manifest(source_path, package, 'flat', [])
            path = evidence.package_manifest(source_path, package, 'flat', ['patchelf'])
        relocated = self.root / 'deployed'
        package.rename(relocated)
        path = relocated / path.name
        for original in self.manifest['binaries'].values():
            Path(original['path']).unlink()
        source_path.unlink()
        packed = json.loads(path.read_text())
        packed['repository'] = str(self.root / 'absent-build-checkout')
        path.write_text(json.dumps(packed))
        # The original source and build artifacts are unnecessary on a deployed machine.
        with patch.object(evidence, 'current_source', side_effect=AssertionError('source checkout queried')):
            loaded = evidence.load_manifest(path)
        gui = loaded['binaries']['gui']
        self.assertEqual(gui['source_sha256'], self.manifest['binaries']['gui']['sha256'])
        self.assertNotEqual(gui['source_sha256'], gui['sha256'])
        self.assertEqual(Path(gui['path']).parent, relocated)

    def test_dirty_snapshot_tracks_untracked_and_tracked_changes(self):
        repo = self.root / 'repo'
        repo.mkdir()
        def git(*args):
            return subprocess.run(['git', *args], cwd=repo, check=True, capture_output=True)
        git('init', '-q')
        hooks = self.root / 'empty-hooks'
        hooks.mkdir()
        git('config', 'core.hooksPath', str(hooks))
        git('config', 'commit.gpgsign', 'false')
        git('config', 'user.email', 'test@example.invalid')
        git('config', 'user.name', 'Test')
        (repo / 'tracked').write_text('initial')
        git('add', 'tracked')
        git('commit', '-qm', 'initial')
        clean = evidence.current_source(repo)
        (repo / 'untracked').write_text('first')
        first = evidence.current_source(repo)
        (repo / 'untracked').write_text('second')
        second = evidence.current_source(repo)
        self.assertNotEqual(evidence.revision(clean), evidence.revision(first))
        self.assertNotEqual(evidence.revision(first), evidence.revision(second))
        (repo / 'tracked').write_text('changed')
        third = evidence.current_source(repo)
        self.assertNotEqual(second['diff_sha256'], third['diff_sha256'])


if __name__ == '__main__':
    unittest.main()
