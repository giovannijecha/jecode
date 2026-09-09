"""Linux-only harness checks; no live provider calls."""
import json
import io
import os
from contextlib import redirect_stdout
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from telemetry import Collector
from terminal import Terminal
from analyze import event_ns, lines, response_windows, task_events
from cleanup import cleanup
from run import codex_config, fixture_copy, jecode_completed
from scenarios import task_directory, freeze_grouped, source_variant, ORIGINAL, GROUPED
from batch import validate_plan
from recover import successful_writes
from planning_report import edit_batches
import hashlib


class CollectorTests(unittest.TestCase):
    def test_null_body_and_private_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            file=Path(directory)/'events.jsonl'
            collector=Collector(file)
            try:
                data={'resourceLogs':[{'scopeLogs':[{'logRecords':[{
                    'body':None,
                    'attributes':[
                        {'key':'event.name','value':{'stringValue':'codex.api_request'}},
                        {'key':'duration_ms','value':{'intValue':'42'}},
                        {'key':'event.timestamp','value':{'stringValue':'2026-09-07T00:00:00Z'}},
                        {'key':'user.email','value':{'stringValue':'private@example.invalid'}},
                        {'key':'output','value':{'stringValue':'private output'}},
                    ],
                }]}]}]}
                request=Request(collector.endpoint,json.dumps(data).encode(),{'Content-Type':'application/json'})
                with urlopen(request,timeout=3) as response: self.assertEqual(response.status,200)
                record=json.loads(file.read_text())
                self.assertEqual(record['fields']['duration_ms'],'42')
                self.assertNotIn('private@',file.read_text())
                self.assertNotIn('private output',file.read_text())
                self.assertEqual(collector.records,1)
                self.assertEqual(collector.errors,0)
            finally: collector.close()

    def test_malformed_payload_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            collector=Collector(Path(directory)/'events.jsonl')
            try:
                with self.assertRaises(HTTPError) as error:
                    urlopen(Request(collector.endpoint,b'not json'),timeout=3)
                self.assertEqual(error.exception.code,400)
                error.exception.close()
                self.assertEqual(collector.errors,1)
            finally: collector.close()


class DriverTests(unittest.TestCase):
    def test_pre_submission_completion_is_paired_before_filtering_task_usage(self):
        def event(second, name, **fields):
            return {'fields': {'event.name': name,
                    'event.timestamp': f'2026-09-08T00:00:{second:02d}Z', **fields}}
        send = lambda second: event(second, 'codex.websocket_request')
        done = lambda second: event(second, 'codex.sse_event', **{'event.kind': 'response.completed', 'output_token_count': '0'})
        events = [send(1), done(3), send(4), done(5)]
        started = event_ns(send(2))
        selected, count = task_events(events, started)
        self.assertEqual(selected, events[2:])
        self.assertEqual(count, 1)
        self.assertEqual(response_windows(selected)['unmatched'], 0)
        # Unknown or ambiguous boundaries remain visible rather than guessed away.
        selected, count = task_events([done(3), send(4), done(5)], started)
        self.assertEqual(len(selected), 3)
        self.assertEqual(count, 0)
        selected, count = task_events([send(0), *events], started)
        self.assertEqual(len(selected), 3)
        self.assertEqual(count, 0)
        unknown_send = {'fields': {'event.name': 'codex.websocket_request'}}
        selected, count = task_events([unknown_send, *events], started)
        self.assertEqual(len(selected), 3)
        self.assertEqual(count, 0)

    def test_network_control_is_opt_in_and_keeps_workspace_permissions(self):
        import tomllib
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            for enabled in (False, True):
                codex_config(home, home/'workspace', home/'complete.json', 'http://127.0.0.1:1', enabled)
                config = tomllib.loads((home/'config.toml').read_text())
                self.assertEqual(config['model_reasoning_effort'], 'high')
                self.assertEqual(config['service_tier'], 'default')
                self.assertFalse(config['agents']['enabled'])
                if enabled:
                    self.assertEqual(config['default_permissions'], 'benchmark')
                    self.assertNotIn('sandbox_mode', config)
                    self.assertEqual(config['permissions']['benchmark'], {
                        'extends': ':workspace', 'network': {'enabled': True}})
                else:
                    self.assertEqual(config['sandbox_mode'], 'workspace-write')
                    self.assertNotIn('default_permissions', config)
                    self.assertNotIn('permissions', config)

    def test_interruption_uses_the_production_settlement_and_only_finished_writes_trigger(self):
        with tempfile.TemporaryDirectory() as directory:
            home=Path(directory); target=home/'sessions/workspace/conversation/nodes/1.json'
            target.parent.mkdir(parents=True)
            target.write_text(json.dumps({'node':{'settlement':'interrupted'}}))
            self.assertEqual(jecode_completed(home)['settlement'],'interrupted')
        self.assertEqual(successful_writes({'blocks':[
            {'kind':'tool','name':'edit_file','tone':'ok'},
            {'kind':'tool','name':'write_file','tone':'pending'},
            {'kind':'tool','name':'run_command','tone':'ok'},
        ]}),1)

    def test_batch_analysis_does_not_assign_timing_when_responses_are_missing(self):
        with tempfile.TemporaryDirectory() as directory:
            run=Path(directory); target=run/'home/sessions/workspace/conversation/nodes/1.json'
            target.parent.mkdir(parents=True); (run/'home/diagnostics').mkdir()
            response={'role':'assistant','content':[{'kind':'tool_call','name':'edit_file','input':{'path':'file.js'}}]}
            target.write_text(json.dumps({'node':{'messages':[response,response]}}))
            (run/'home/diagnostics/record.jsonl').write_text('{"kind":"request","outcome":"completed","providerMs":10}\n')
            result=edit_batches(run)
            self.assertEqual(result['responsesWithEdits'],2)
            self.assertFalse(result['timingAlignmentVerified'])
            self.assertIsNone(result['adjacentEditOnlyResponses'][0]['providerMs'])

    def test_task_selection_is_bounded_and_new_fixtures_are_materialized(self):
        with self.assertRaises(ValueError): task_directory('../../private')
        for name in ('cache','planner'):
            with tempfile.TemporaryDirectory() as directory:
                workspace=Path(directory)/'workspace'
                fixture_copy(workspace,name)
                self.assertTrue((workspace/'test/smoke.test.js').is_file())
                self.assertFalse(list(workspace.rglob('*.template')))

    def test_variant_changes_only_the_planning_instruction_and_does_not_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); source=root/'jecode'
            (source/'src').mkdir(parents=True); (source/'node_modules').mkdir()
            (source/'src/prompt.ts').write_text('['+json.dumps(ORIGINAL)+'].join("\\n")')
            (source/'src/unchanged.ts').write_text('unchanged')
            files={f'src/{name}':hashlib.sha256((source/'src'/name).read_bytes()).hexdigest()
                   for name in ('prompt.ts','unchanged.ts')}
            (root/'snapshot.json').write_text(json.dumps({'baseCommit':'synthetic','sha256':'baseline','files':files}))
            freeze_grouped(root)
            variant,snapshot=source_variant(root,'grouped')
            self.assertEqual((source/'src/prompt.ts').read_text(),'['+json.dumps(ORIGINAL)+'].join("\\n")')
            text=(variant/'src/prompt.ts').read_text()
            self.assertEqual('\n'.join(json.loads(text[:text.index('].join')+1])),GROUPED)
            self.assertEqual(snapshot['files']['src/unchanged.ts'],files['src/unchanged.ts'])
            with self.assertRaises(FileExistsError): freeze_grouped(root)
            # A promoted production prompt cannot silently receive the rule twice.
            (source/'src/prompt.ts').write_text(text)
            with self.assertRaisesRegex(ValueError, 'already contains'): freeze_grouped(root)

    def test_plan_rejects_duplicates_traversal_and_modified_codex(self):
        row={'name':'trial','client':'jecode','task':'cache','variant':'grouped'}
        self.assertEqual(validate_plan([row]),[row])
        for plan in ([row,row],[dict(row,name='../escape')],[dict(row,client='codex')],[]):
            with self.assertRaises(ValueError): validate_plan(plan)

    def test_fixture_materializes_the_frozen_smoke_test(self):
        with tempfile.TemporaryDirectory() as directory:
            workspace=Path(directory)/'workspace'
            fixture_copy(workspace)
            self.assertTrue((workspace/'test/smoke.test.js').is_file())
            self.assertFalse(list(workspace.rglob('*.template')))
            self.assertTrue((workspace/'.git').is_dir())

    def test_close_with_output_after_log_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            log=io.BytesIO()
            terminal=Terminal(['/bin/sh','-c','printf ready; sleep 30'],directory,
                              {'PATH':'/usr/bin:/bin'},log)
            terminal.wait_for('ready',3)
            log.close()
            terminal.close()
            with self.assertRaises(ChildProcessError): os.waitpid(terminal.pid,os.WNOHANG)

    def test_parser_failure_during_close_still_reaps_the_child(self):
        with tempfile.TemporaryDirectory() as directory:
            terminal = Terminal(['/bin/sh','-c','printf ready; sleep 30'], directory,
                                {'PATH':'/usr/bin:/bin'}, io.BytesIO())
            terminal.wait_for('ready', 3)
            with patch.object(terminal, 'pump', side_effect=ValueError('malformed terminal sequence')):
                with self.assertRaisesRegex(ValueError, 'malformed'):
                    terminal.close()
            with self.assertRaises(ChildProcessError): os.waitpid(terminal.pid, os.WNOHANG)
            terminal.close()
            self.assertGreater(terminal.observations['chunks'], 0)

    def test_partial_jsonl_tail_is_not_a_complete_event(self):
        with tempfile.TemporaryDirectory() as directory:
            file=Path(directory)/'events.jsonl'
            file.write_text('{"complete":true}\n{"partial":')
            self.assertEqual(lines(file),[{'complete':True}])

    def test_event_time_uses_provider_timestamp_not_batch_delivery(self):
        event={'receivedNs':99,'atNs':'0','fields':{'event.timestamp':'1970-01-01T00:00:01.125Z'}}
        self.assertEqual(event_ns(event),1_125_000_000)
        self.assertIsNone(event_ns({'receivedNs':99,'fields':{}}))

    def test_response_intervals_require_unambiguous_pairs(self):
        sent={'fields':{'event.name':'codex.websocket_request','event.timestamp':'1970-01-01T00:00:01Z'}}
        completed={'fields':{'event.name':'codex.sse_event','event.kind':'response.completed',
                             'event.timestamp':'1970-01-01T00:00:03Z'}}
        self.assertEqual(response_windows([completed,sent])['milliseconds']['sum'],2000)
        self.assertIsNone(response_windows([sent,sent,completed])['milliseconds'])
        self.assertIsNone(response_windows([completed])['milliseconds'])


class CleanupTests(unittest.TestCase):
    def test_preparation_failure_without_runs_can_remove_seed_copies(self):
        with tempfile.TemporaryDirectory(dir='/var/tmp') as directory:
            root=Path(directory)
            for name in ('environment.json','snapshot.json'): (root/name).write_text('{}')
            (root/'codex-home').mkdir(); auth=root/'codex-home/auth.json'; auth.write_text('{}')
            with redirect_stdout(io.StringIO()): cleanup(root)
            self.assertFalse(auth.exists())

    def laboratory(self, root, settled=True):
        for name in ('environment.json','snapshot.json'): (root/name).write_text('{}')
        home=root/'runs'/'synthetic'/'home'
        home.mkdir(parents=True)
        if settled: (home.parent/'outcome.json').write_text('{"status":"completed"}')
        (home/'accounts.json').write_text('{"synthetic":true}')
        (home/'settings.json').write_text('{}')
        return home

    def test_only_account_copies_are_removed(self):
        with tempfile.TemporaryDirectory(dir='/var/tmp') as directory:
            root=Path(directory)
            home=self.laboratory(root)
            with redirect_stdout(io.StringIO()): cleanup(root)
            self.assertFalse((home/'accounts.json').exists())
            self.assertTrue((home/'settings.json').exists())
            self.assertTrue((home.parent/'outcome.json').exists())

    def test_unsettled_run_preserves_account(self):
        with tempfile.TemporaryDirectory(dir='/var/tmp') as directory:
            root=Path(directory)
            home=self.laboratory(root,settled=False)
            with self.assertRaises(ValueError): cleanup(root)
            self.assertTrue((home/'accounts.json').exists())

    def test_link_escape_is_checked_before_any_deletion(self):
        with tempfile.TemporaryDirectory(dir='/var/tmp') as directory:
            root=Path(directory)
            home=self.laboratory(root)
            (root/'codex-home').mkdir()
            (root/'codex-home'/'auth.json').symlink_to('/dev/null')
            with self.assertRaises(ValueError): cleanup(root)
            self.assertTrue((home/'accounts.json').exists())


if __name__=='__main__': unittest.main()
