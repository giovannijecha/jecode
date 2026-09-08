"""Apply post-hoc probes to every masked output after all timed trials settle."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import tempfile

from prepare import private_json
from quality import workspace_files


def run(root, reviews):
    plan = json.loads((root/'batch-daily.json').read_text())['plan']
    declared = [entry['name'] for entry in plan]
    if len(declared) != 18 or len(set(declared)) != 18:
        raise ValueError('expected all eighteen declared trials')
    for name in declared:
        outcome = json.loads((root/'runs'/name/'outcome.json').read_text())
        if outcome['status'] not in ('completed', 'failed', 'interrupted', 'timeout', 'process-exited'):
            raise ValueError('timed trials must settle before supplementary execution')
    selected = []
    entries = []
    for review in reviews:
        scope = json.loads((review/'review-scope.json').read_text())
        inventory = json.loads((review/'review-inventory.json').read_text())
        selected.extend(scope['selectedRuns'])
        if len(inventory) != len(scope['selectedRuns']):
            raise ValueError('review inventory differs from its declared scope')
        for entry in inventory:
            alias = entry['alias']
            if not re.fullmatch(r'(config-edit|file-server)-\d{2}', alias):
                raise ValueError('unexpected masked alias')
            workspace = review/'outputs'/alias
            if workspace.is_symlink() or workspace.resolve().parent != review/'outputs':
                raise ValueError('masked output escapes its review directory')
            entries.append((scope['round'], alias, workspace))
    if Counter(selected) != Counter(declared) or len(entries) != 18:
        raise ValueError('include each declared output exactly once')
    labels = [f'round-{number or "all"}-{alias}' for number, alias, _ in entries]
    if len(set(labels)) != 18:
        raise ValueError('masked output labels must be unique')
    node = json.loads((root/'environment.json').read_text())['node']
    output = root/'daily-probes'
    output.mkdir(mode=0o700)
    reports = []
    for label, (number, alias, workspace) in zip(labels, entries):
        before = workspace_files(workspace)
        task = alias.rsplit('-', 1)[0]
        with tempfile.TemporaryDirectory(prefix='fixture-', dir=output) as fixture:
            env = {'PATH': f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin',
                   'HOME': fixture, 'TMPDIR': fixture, 'LANG': 'C.UTF-8', 'NO_COLOR': '1'}
            with (output/f'{label}.json').open('w') as stdout, (output/f'{label}.stderr').open('w') as stderr:
                try:
                    result = subprocess.run([node, str(Path(__file__).with_name('daily-probes.mjs')),
                                             task, str(workspace)], cwd=fixture, env=env,
                                            stdout=stdout, stderr=stderr, timeout=30)
                    code = result.returncode
                except subprocess.TimeoutExpired:
                    code = 124
        try:
            value = json.loads((output/f'{label}.json').read_text())
        except (OSError, ValueError):
            value = None
        row = {'round': number, 'alias': alias, 'task': task, 'exit': code,
               'workspaceUnchanged': before == workspace_files(workspace), 'probe': value}
        reports.append(row)
        print(json.dumps({key: row[key] for key in ('round', 'alias', 'exit', 'workspaceUnchanged')}), flush=True)
    private_json(output/'results.json', reports)
    # This is runner integrity, not a claim that every robustness probe passed.
    return int(not all(row['exit'] == 0 and row['workspaceUnchanged'] and row['probe'] for row in reports))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('reviews', nargs='+', type=Path)
    args = parser.parse_args()
    raise SystemExit(run(args.root.resolve(), [path.resolve() for path in args.reviews]))
