#!/usr/bin/env python3
"""Fail-closed tests for the selected-target execution boundary."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test_selected as runner


def artifact():
    return {
        'reason': 'compiler-artifact', 'package_id': 'gpu-id',
        'target': {'name': 'spektrafilm_gpu', 'kind': ['lib']},
        'profile': {'test': True}, 'executable': '/unused/test-gpu',
    }


def package():
    return {
        'id': 'gpu-id', 'name': 'spektrafilm-gpu',
        'manifest_path': '/unused/Cargo.toml',
        'targets': [{'name': 'spektrafilm_gpu', 'kind': ['lib'], 'test': True}],
    }


class SelectionTests(unittest.TestCase):
    def test_old_or_non_test_binaries_cannot_satisfy_a_missing_target(self):
        message = artifact()
        message['profile']['test'] = False
        artifacts = runner.collect_artifacts([message])
        with self.assertRaisesRegex(ValueError, 'Missing selected'):
            runner.selected_artifacts(artifacts, {'spektrafilm-gpu': package()},
                                      runner.SELECTIONS[:1])

    def test_duplicate_artifacts_are_rejected(self):
        message = artifact()
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            runner.collect_artifacts([message, message])

    def test_same_target_name_in_another_package_is_not_accepted(self):
        message = artifact()
        message['package_id'] = 'different-package'
        with self.assertRaisesRegex(ValueError, 'Missing selected'):
            runner.selected_artifacts(runner.collect_artifacts([message]),
                                      {'spektrafilm-gpu': package()}, runner.SELECTIONS[:1])

    def test_cargo_failure_does_not_execute_partial_build_artifacts(self):
        with patch.object(runner.subprocess, 'Popen') as popen:
            popen.return_value.stdout = [json.dumps(artifact()) + '\n']
            popen.return_value.wait.return_value = 101
            with self.assertRaises(subprocess.CalledProcessError):
                runner.build(['--no-run'])

    def test_missing_build_completion_fails(self):
        with patch.object(runner.subprocess, 'Popen') as popen:
            popen.return_value.stdout = [json.dumps(artifact()) + '\n']
            popen.return_value.wait.return_value = 0
            with self.assertRaisesRegex(ValueError, 'successful build'):
                runner.build(['--no-run'])

    def test_execution_preserves_policy_cwd_and_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / 'target' / 'debug' / 'deps' / 'gpu-test'
            executable.parent.mkdir(parents=True)
            executable.touch()
            message = artifact()
            message['executable'] = str(executable)
            metadata = package()
            metadata['manifest_path'] = str(root / 'gpu' / 'Cargo.toml')
            with patch.object(runner.subprocess, 'check_output', return_value='/rust/lib\n'), \
                    patch.object(runner.subprocess, 'run') as run:
                run.side_effect = subprocess.CalledProcessError(1, ['gpu-test'])
                with self.assertRaises(subprocess.CalledProcessError):
                    runner.run_selected(runner.collect_artifacts([message]), [],
                                        {'packages': [metadata], 'target_directory': str(root / 'target')},
                                        runner.SELECTIONS[:1])
                args, kwargs = run.call_args
                self.assertEqual(args[0], [str(executable), 'logic_tests::'])
                self.assertEqual(kwargs['cwd'], root / 'gpu')
                self.assertEqual(kwargs['env']['SPEKTRAFILM_TEST_NO_DEVICE'], '1')
                self.assertTrue(kwargs['check'])


if __name__ == '__main__':
    unittest.main()
