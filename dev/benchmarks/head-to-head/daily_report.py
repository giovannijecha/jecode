"""Summarize original daily-driver outcomes without hiding unsuccessful attempts."""

import argparse
from collections import Counter
import json
from pathlib import Path
import statistics

from prepare import private_json


def distribution(values):
    if not values:
        return None
    return {'n': len(values), 'sum': sum(values), 'mean': statistics.mean(values),
            'median': statistics.median(values), 'min': min(values), 'max': max(values)}


def summarize(records):
    groups = {}
    for row in records:
        key = (row['task'], row['client'], row['variant'])
        groups.setdefault(key, []).append(row)
    output = []
    for (task, client, variant), rows in sorted(groups.items()):
        verified = [row for row in rows if row['status'] == 'completed' and row['acceptanceExit'] == 0
                    and isinstance(row['passed'], int) and not isinstance(row['passed'], bool)
                    and isinstance(row['total'], int) and not isinstance(row['total'], bool)
                    and row['total'] > 0 and row['passed'] == row['total']]
        elapsed = [row['elapsedMs'] for row in rows if isinstance(row['elapsedMs'], (int, float))]
        output.append({'task': task, 'client': client, 'variant': variant, 'attempts': len(rows),
                       'outcomes': dict(Counter(row['status'] for row in rows)),
                       'acceptanceVerifiedCompletions': len(verified),
                       'attemptElapsedMs': distribution(elapsed),
                       # This describes only the successful subset, not a ranking.
                       'verifiedSubsetElapsedMs': distribution([row['elapsedMs'] for row in verified]),
                       'allAttemptsVerified': len(verified) == len(rows),
                       'runs': [row['run'] for row in rows]})
    comparisons = []
    for task in sorted({row['task'] for row in output}):
        selected = [row for row in output if row['task'] == task]
        codex = next((row for row in selected if row['client'] == 'codex'), None)
        baseline = next((row for row in selected if row['client'] == 'jecode'
                         and row['variant'] == 'baseline'), None)
        for candidate in [row for row in selected if row['client'] == 'jecode']:
            comparable = (codex is not None and codex['allAttemptsVerified']
                          and candidate['allAttemptsVerified'] and codex['attempts'] == candidate['attempts'])
            baseline_comparable = (baseline is not None and baseline['allAttemptsVerified']
                                   and candidate['allAttemptsVerified']
                                   and baseline['attempts'] == candidate['attempts'])
            comparisons.append({'task': task, 'variant': candidate['variant'],
                                'allAttemptsVerifiedForBoth': comparable,
                                'meanElapsedRatioToCodex': candidate['attemptElapsedMs']['mean'] /
                                codex['attemptElapsedMs']['mean'] if comparable else None,
                                'allAttemptsVerifiedAgainstJecodeBaseline': baseline_comparable,
                                'meanElapsedRatioToJecodeBaseline': candidate['attemptElapsedMs']['mean'] /
                                baseline['attemptElapsedMs']['mean'] if baseline_comparable else None})
    return {'groups': output, 'comparisons': comparisons,
            'interpretation': 'Original acceptance only; source review and post-hoc probes are separate. '
            'Attempt elapsed includes failed and incomplete work. Successful-subset time is not a ranking. '
            'Three repetitions per task do not establish population failure rates or tail latency.'}


def report(root):
    plan = json.loads((root/'batch-daily.json').read_text())['plan']
    if len(plan) != 18 or len({row['name'] for row in plan}) != 18:
        raise ValueError('expected the eighteen declared trials')
    records = []
    for entry in plan:
        run = root/'runs'/entry['name']
        if run.is_symlink() or run.resolve().parent != root/'runs':
            raise ValueError('run must remain directly inside the declared laboratory')
        outcome = json.loads((run/'outcome.json').read_text())
        manifest = json.loads((run/'manifest.json').read_text())
        if any(manifest[key] != entry[key] for key in ('client', 'task', 'variant')):
            raise ValueError('measurement differs from the declared plan')
        if outcome['status'] not in ('completed', 'failed', 'interrupted', 'timeout', 'process-exited'):
            raise ValueError('unsettled or invalid measurement must be resolved first')
        try:
            acceptance = json.loads((run/'acceptance.json').read_text())
        except (OSError, ValueError):
            acceptance = {}
        records.append({'run': entry['name'], **{key: entry[key] for key in ('client', 'task', 'variant')},
                        'status': outcome['status'], 'elapsedMs': outcome['elapsedMs'],
                        'acceptanceExit': outcome.get('acceptanceExit'),
                        'passed': acceptance.get('passed'), 'total': acceptance.get('total')})
    value = {'records': records, **summarize(records)}
    private_json(root/'daily-summary.json', value)
    print(json.dumps(value, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    report(parser.parse_args().root.resolve())
