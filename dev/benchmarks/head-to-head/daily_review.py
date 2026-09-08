"""Make masked copies of every declared settled output, including failed trials."""
import argparse
import hashlib
import json
from pathlib import Path
import random
from prepare import private_json
from quality import workspace_files


def prepare(root, destination, round_number=None):
    if destination.exists() or destination.is_relative_to(root):
        raise ValueError('use a new review directory outside the original laboratory')
    plan = json.loads((root/'batch-daily.json').read_text())['plan']
    if len(plan) != 18 or len({run['name'] for run in plan}) != 18:
        raise ValueError('expected all eighteen declared trials')
    if round_number is not None:
        if round_number not in (1, 2, 3):
            raise ValueError('round must be one, two or three')
        # Fixed contiguous groups follow the original counterbalanced order.
        # Review can overlap later runs; no participant receives review feedback.
        plan = plan[(round_number - 1) * 6:round_number * 6]
    records = []
    for entry in plan:
        run = root/'runs'/entry['name']
        if run.is_symlink() or run.resolve().parent != root/'runs':
            raise ValueError('run must remain directly inside the declared laboratory')
        outcome = json.loads((run/'outcome.json').read_text())
        if outcome['status'] not in ('completed', 'failed', 'interrupted', 'timeout', 'process-exited'):
            raise ValueError('unsettled or invalid measurement must be resolved first')
        manifest = json.loads((run/'manifest.json').read_text())
        if any(manifest[key] != entry[key] for key in ('client','task','variant')):
            raise ValueError('declared run differs from its measured manifest')
        records.append((run, entry['task'], outcome['status'], workspace_files(run/'workspace')))
    random.SystemRandom().shuffle(records)
    destination.mkdir(mode=0o700)
    mapping, summary, counts = {}, [], {}
    for run, task, status, files in records:
        counts[task] = counts.get(task, 0) + 1
        alias = f'{task}-{counts[task]:02}'
        for name, expected in files.items():
            content = (run/'workspace'/name).read_bytes()
            if hashlib.sha256(content).hexdigest() != expected:
                raise ValueError('participant files changed while copying')
            target = destination/'outputs'/alias/name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        mapping[alias] = {'run': run.name, 'files': files, 'settlement': status}
        source_lines = {name: len((run/'workspace'/name).read_text().splitlines()) for name in files
                        if name.startswith('src/') and name.endswith(('.js','.mjs','.ts'))}
        summary.append({'alias': alias, 'sourceLines': source_lines,
                        'sourceTotalLines': sum(source_lines.values()), 'files': sorted(files)})
    private_json(destination/'identity.json', mapping)
    private_json(destination/'review-inventory.json', summary)
    private_json(destination/'review-scope.json', {'round': round_number,
                 'declaredTotal': 18, 'selectedRuns': [row['name'] for row in plan]})
    print(json.dumps({'destination': str(destination), 'outputs': len(records), 'round': round_number}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--round', type=int, choices=(1, 2, 3))
    args = parser.parse_args()
    prepare(args.root.resolve(), args.destination.resolve(), args.round)
