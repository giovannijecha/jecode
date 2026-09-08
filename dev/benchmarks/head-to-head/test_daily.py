import unittest

from daily_report import summarize
from analyze import reported_errors


def record(client, elapsed, status='completed', passed=53):
    return {'task': 'config-edit', 'client': client, 'variant': 'baseline',
            'run': f'{client}-{elapsed}', 'status': status, 'elapsedMs': elapsed,
            'passed': passed, 'total': 53, 'acceptanceExit': 0}


class DailyReportTests(unittest.TestCase):
    def test_usage_free_completion_errors_remain_visible_without_error_content(self):
        event = {'fields': {'event.name': 'codex.sse_event', 'event.kind': 'response.completed'},
                 'attributeKeys': ['event.name', 'event.kind', 'error.message']}
        result = reported_errors([event, {'fields': {'event.name': 'codex.sse_event'}}])
        self.assertEqual(result, {'count': 1, 'events': {'codex.sse_event': 1}})
        self.assertEqual(reported_errors([]), {'count': 0, 'events': {}})

    def test_fast_partial_and_failed_attempts_cannot_win_a_completion_comparison(self):
        rows = [record('jecode', 10, passed=52), record('jecode', 20, status='failed'),
                record('jecode', 300), record('codex', 200), record('codex', 210), record('codex', 220)]
        result = summarize(rows)
        group = next(row for row in result['groups'] if row['client'] == 'jecode')
        self.assertEqual(group['attempts'], 3)
        self.assertEqual(group['attemptElapsedMs']['sum'], 330)
        self.assertEqual(group['acceptanceVerifiedCompletions'], 1)
        self.assertEqual(group['verifiedSubsetElapsedMs']['mean'], 300)
        self.assertIsNone(result['comparisons'][0]['meanElapsedRatioToCodex'])

    def test_all_attempts_must_have_valid_nonempty_acceptance(self):
        for passed, total in ((None, None), (0, 0), (True, True)):
            row = {**record('jecode', 100), 'passed': passed, 'total': total}
            result = summarize([row, record('codex', 200)])
            self.assertFalse(result['groups'][1]['allAttemptsVerified'])
            self.assertIsNone(result['comparisons'][0]['meanElapsedRatioToCodex'])

    def test_complete_equal_groups_report_a_descriptive_ratio(self):
        result = summarize([record('jecode', 100), record('jecode', 200), record('jecode', 150),
                            record('codex', 300), record('codex', 400), record('codex', 200)])
        self.assertEqual(result['comparisons'][0]['meanElapsedRatioToCodex'], 0.5)

    def test_reported_passes_do_not_hide_a_failed_acceptance_process(self):
        for code in (None, 1, 124):
            result = summarize([{**record('jecode', 100), 'acceptanceExit': code}, record('codex', 200)])
            self.assertFalse(result['comparisons'][0]['allAttemptsVerifiedForBoth'])
            self.assertIsNone(result['comparisons'][0]['meanElapsedRatioToCodex'])

    def test_candidate_benefit_is_measured_against_the_current_jecode_too(self):
        rows = [record('jecode', 100), record('codex', 200),
                {**record('jecode', 120), 'variant': 'work-state'}]
        result = summarize(rows)
        candidate = next(row for row in result['comparisons'] if row['variant'] == 'work-state')
        self.assertEqual(candidate['meanElapsedRatioToJecodeBaseline'], 1.2)
        self.assertEqual(candidate['meanElapsedRatioToCodex'], 0.6)


if __name__ == '__main__':
    unittest.main()
