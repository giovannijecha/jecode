"""Quality evidence copies must preserve provenance and never mutate source runs."""
import io
import json
from contextlib import redirect_stdout
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

from quality import prepare, workspace_files
from verify import verify
from provenance import evaluator_files


class QualityTests(unittest.TestCase):
    def test_passing_artifact_checks_do_not_promote_a_failed_turn(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root/'jecode'; source.mkdir()
            (source/'source.js').write_text('unchanged')
            task = root/'task'; (task/'fixture').mkdir(parents=True)
            (task/'PROMPT.md').write_text('complete the task')
            (task/'acceptance.mjs').write_text('synthetic evaluator')
            (task/'fixture/package.json').write_text('{}')
            run = root/'runs/trial'; (run/'workspace').mkdir(parents=True)
            (run/'workspace/package.json').write_text('{}')
            (root/'environment.json').write_text('{"node":"/synthetic/node"}')
            (root/'snapshot.json').write_text(json.dumps({'sha256': 'snapshot', 'files': workspace_files(source)}))
            task_hashes = workspace_files(task)
            (run/'manifest.json').write_text(json.dumps({
                'preflight': False, 'snapshotHash': 'snapshot',
                'fixture': workspace_files(task/'fixture'),
                'promptHash': task_hashes['PROMPT.md'], 'acceptanceHash': task_hashes['acceptance.mjs'],
            }))
            # Independent evaluators and project checks succeed; only settlement differs.
            def successful(command, **_kwargs):
                return SimpleNamespace(returncode=0, stdout='1' if command[:2] == ['git', 'rev-list'] else '')
            for status, exit_code in [('failed', 1), ('interrupted', 1), ('completed', 0)]:
                (run/'outcome.json').write_text(json.dumps({'status': status}))
                with patch('verify.task_directory', return_value=task), patch('verify.subprocess.run', side_effect=successful), redirect_stdout(io.StringIO()):
                    self.assertEqual(verify(root, True), exit_code)
                report = json.loads((run/'verification.json').read_text())
                self.assertEqual(report['runStatus'], status)
                self.assertEqual(report['acceptanceExit'], 0)
                self.assertEqual(report['ownTestsExit'], 0)
            (root/'batch-declared.json').write_text(json.dumps({'plan': [
                {'name': 'trial'}, {'name': 'not-started'}]}))
            with patch('verify.task_directory', return_value=task), patch('verify.subprocess.run', side_effect=successful), redirect_stdout(io.StringIO()):
                self.assertEqual(verify(root, True), 1)
            aggregate = json.loads((root/'verification.json').read_text())
            self.assertEqual(aggregate['missingDeclaredRuns'], ['not-started'])

            # An unchanged entrypoint does not prove an imported evaluator is unchanged.
            (root/'batch-declared.json').unlink()
            helper = task/'checks.mjs'; helper.write_text('original checks')
            manifest = json.loads((run/'manifest.json').read_text())
            manifest['evaluatorFiles'] = evaluator_files(task)
            (run/'manifest.json').write_text(json.dumps(manifest))
            helper.write_text('weakened checks')
            with patch('verify.task_directory', return_value=task), patch('verify.subprocess.run', side_effect=successful), redirect_stdout(io.StringIO()):
                self.assertEqual(verify(root, True), 1)
            report = json.loads((run/'verification.json').read_text())
            self.assertTrue(report['sameAcceptance'])
            self.assertFalse(report['sameEvaluatorFiles'])

    def fixture(self, root):
        for task in ('cache', 'planner'):
            for client, variant in (('jecode', 'baseline'), ('jecode', 'grouped'), ('codex', 'baseline')):
                run = root/'runs'/f'{task}-{client}-{variant}'
                (run/'workspace/src').mkdir(parents=True)
                (run/'workspace/src/index.js').write_text('export const synthetic = true;')
                (run/'workspace/.git').mkdir(); (run/'workspace/.git/config').write_text('private metadata')
                (run/'home').mkdir(); (run/'home/accounts.json').write_text('must not be copied')
                (run/'manifest.json').write_text(json.dumps({'task': task, 'client': client,
                    'variant': variant, 'preflight': False}))
                (run/'outcome.json').write_text('{"status":"completed"}')

    def test_masks_metadata_preserves_content_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)/'lab'; destination = Path(folder)/'review'; self.fixture(root)
            before = {str(f.relative_to(root)): f.read_bytes() for f in root.rglob('*') if f.is_file()}
            with redirect_stdout(io.StringIO()): prepare(root, destination)
            aliases = json.loads((destination/'identity.json').read_text())
            self.assertEqual(len(aliases), 6)
            for alias, entry in aliases.items():
                self.assertEqual(workspace_files(destination/'outputs'/alias), entry['files'])
                self.assertEqual(set(entry['files']), {'src/index.js'})
            self.assertEqual(before, {str(f.relative_to(root)): f.read_bytes() for f in root.rglob('*') if f.is_file()})
            with self.assertRaises(FileExistsError): prepare(root, destination)

    def test_incomplete_trial_is_not_silently_dropped(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)/'lab'; self.fixture(root)
            next((root/'runs').glob('*/outcome.json')).write_text('{"status":"failed"}')
            with self.assertRaisesRegex(ValueError, 'incomplete'): prepare(root, Path(folder)/'review')
            self.assertFalse((Path(folder)/'review').exists())

    def test_links_and_nested_destination_are_rejected_before_copying(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)/'lab'; self.fixture(root)
            with self.assertRaises(ValueError): prepare(root, root/'review')
            workspace = next((root/'runs').glob('*/workspace'))
            (workspace/'src/escape').symlink_to('/etc/passwd')
            with self.assertRaisesRegex(ValueError, 'symbolic link'): prepare(root, Path(folder)/'review')
            self.assertFalse((Path(folder)/'review').exists())


if __name__ == '__main__': unittest.main()
