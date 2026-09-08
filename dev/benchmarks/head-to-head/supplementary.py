"""Run review-derived probes separately from original scores and elapsed times."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def main(root, probes):
    node = json.loads((root/'environment.json').read_text())['node']
    runs = [run for run in sorted((root/'runs').iterdir())
            if not json.loads((run/'manifest.json').read_text())['preflight']]
    if any(not (run/'outcome.json').exists() for run in runs):
        raise ValueError('wait for every timed run to settle')
    output = []
    for run in runs:
        workspace = run/'workspace'
        row = {'run': run.name, 'probes': []}
        for name, script, arguments in probes:
            report = run/f'{name}.json'
            if report.exists():
                raise FileExistsError(report)
            env = {'PATH': f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin',
                   'HOME': str(run/'home'), 'LANG': 'C.UTF-8', 'NO_COLOR': '1'}
            with report.open('w') as stdout, (run/f'{name}.stderr').open('w') as stderr:
                result = subprocess.run([node, str(script), *arguments, str(workspace)],
                                        cwd=workspace, env=env, stdout=stdout, stderr=stderr, timeout=30)
            data = json.loads(report.read_text())
            row['probes'].append({'name': name, 'exit': result.returncode,
                'sourceHash': hashlib.sha256(script.read_bytes()).hexdigest(),
                'passed': sum(case['passed'] for case in data['results']),
                'total': len(data['results'])})
        output.append(row)
    (root/'supplementary.json').write_text(json.dumps(output, indent=2)+'\n')
    print(json.dumps(output, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    main(args.root.resolve(), [
        ('supplementary-quality', here/'quality-probes.mjs', ['planner']),
        ('representation', here/'representation-probe.mjs', []),
    ])
