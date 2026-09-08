"""Offline guards for the extended experiment; no model calls."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from provenance import evaluator_files
from scenarios import ORIGINAL, GROUPED, CONTRACT, freeze_contract, source_variant
from batch import main as batch
from verify import verify


class DurableTests(unittest.TestCase):
    def test_separate_recovery_cannot_replace_a_declared_trial(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); run = root/'runs/recovery'; run.mkdir(parents=True)
            (root/'environment.json').write_text(json.dumps({'node':'unused'}))
            (root/'batch-one.json').write_text(json.dumps({'plan':[{'name':'recovery'}]}))
            (run/'outcome.json').write_text(json.dumps({'status':'completed'}))
            (run/'manifest.json').write_text(json.dumps({'preflight':False, 'recovery':True,
                                                       'excludedFromTimedComparison':True}))
            with patch('verify.subprocess.run') as execute, patch('builtins.print'):
                self.assertEqual(verify(root, True), 1)
            execute.assert_not_called()
            report = json.loads((root/'verification.json').read_text())
            self.assertEqual(report['missingDeclaredRuns'], ['recovery'])
            self.assertEqual(report['separateRecoveryRuns'], ['recovery'])

    def test_failed_precheck_starts_no_timed_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = root/'plan.json'
            plan.write_text(json.dumps([
                {'name': 'contract-one', 'client': 'jecode', 'task': 'durable', 'variant': 'contract'},
                {'name': 'codex-one', 'client': 'codex', 'task': 'durable', 'variant': 'baseline'},
            ]))
            args = ['batch.py', '--root', str(root), '--prefix', 'guard', '--plan', str(plan), '--precheck']
            with patch('sys.argv', args), patch('batch.precheck', return_value=1) as preparation, \
                 patch('batch.subprocess.run') as launch:
                self.assertEqual(batch(), 1)
            preparation.assert_called_once_with(root.resolve(), ['durable'], ['contract'])
            launch.assert_not_called()
            self.assertFalse((root/'batch-guard.json').exists())
            self.assertFalse((root/'runs').exists())

    def test_contract_variant_preserves_grouping_and_refuses_double_insertion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root/'jecode'
            (source/'src').mkdir(parents=True); (source/'node_modules').mkdir()
            original = '['+', '.join(json.dumps(line) for line in GROUPED.splitlines())+'].join("\\n")'
            (source/'src/prompt.ts').write_text(original)
            files = {'src/prompt.ts': hashlib.sha256(original.encode()).hexdigest()}
            (root/'snapshot.json').write_text(json.dumps({'baseCommit':'synthetic','sha256':'baseline','files':files}))
            freeze_contract(root)
            variant, snapshot = source_variant(root, 'contract')
            text = (variant/'src/prompt.ts').read_text()
            lines = json.loads(text[:text.index('].join')+1])
            self.assertEqual('\n'.join(lines), CONTRACT+'\n'+GROUPED)
            self.assertEqual(lines.count(ORIGINAL), 1)
            self.assertEqual((source/'src/prompt.ts').read_text(), original)
            self.assertEqual(snapshot['changedFiles'], ['src/prompt.ts'])
            (source/'src/prompt.ts').write_text(text)
            with self.assertRaisesRegex(ValueError, 'already contains'): freeze_contract(root)

    def test_evaluator_hashes_include_nested_helpers_and_exclude_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root/'nested').mkdir(); (root/'fixture').mkdir()
            (root/'acceptance.mjs').write_text('import "./nested/checks.mjs"')
            helper = root/'nested/checks.mjs'; helper.write_text('original')
            (root/'fixture/work.mjs').write_text('participant')
            first = evaluator_files(root)
            helper.write_text('changed')
            second = evaluator_files(root)
            self.assertEqual(set(first), {'acceptance.mjs', 'nested/checks.mjs'})
            self.assertEqual(first['acceptance.mjs'], second['acceptance.mjs'])
            self.assertNotEqual(first['nested/checks.mjs'], second['nested/checks.mjs'])
