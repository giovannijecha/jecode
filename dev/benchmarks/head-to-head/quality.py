"""Make identity-masked review copies without modifying saved agent outputs."""

import argparse
import hashlib
import json
from pathlib import Path
import random
import shutil
import subprocess

from prepare import private_json


def workspace_files(workspace):
    if workspace.is_symlink() or not workspace.is_dir():
        raise ValueError('review input must be a regular workspace directory')
    result = {}
    for file in sorted(workspace.rglob('*')):
        relative = file.relative_to(workspace)
        if '.git' in relative.parts:
            continue
        if file.is_symlink():
            raise ValueError('review input contains a symbolic link')
        if file.is_file():
            if not file.resolve().is_relative_to(workspace.resolve()):
                raise ValueError('review input escapes the workspace')
            result[relative.as_posix()] = hashlib.sha256(file.read_bytes()).hexdigest()
    return result


def prepare(root, destination):
    if destination.exists():
        raise FileExistsError('review destination already exists')
    if destination.is_relative_to(root):
        raise ValueError('review copies must be outside the original laboratory')
    inputs = []
    configurations = {'cache': set(), 'planner': set()}
    for run in sorted((root/'runs').iterdir()):
        if run.is_symlink():
            raise ValueError('run directories must not be symbolic links')
        if not (run/'manifest.json').is_file():
            continue
        manifest = json.loads((run/'manifest.json').read_text())
        if manifest.get('preflight') or manifest.get('recovery'):
            continue
        if manifest.get('task') not in ('cache', 'planner'):
            continue
        outcome = json.loads((run/'outcome.json').read_text())
        if outcome['status'] != 'completed':
            raise ValueError('incomplete trial must be reviewed separately, not silently omitted')
        configuration = (manifest.get('client'), manifest.get('variant', 'baseline'))
        if configuration in configurations[manifest['task']]:
            raise ValueError('duplicate task configuration')
        configurations[manifest['task']].add(configuration)
        inputs.append((run, manifest['task'], workspace_files(run/'workspace')))
    expected = {('jecode', 'baseline'), ('jecode', 'grouped'), ('codex', 'baseline')}
    if len(inputs) != 6 or any(configs != expected for configs in configurations.values()):
        raise ValueError('expected the six uninterrupted planning outputs')
    random.SystemRandom().shuffle(inputs)
    destination.mkdir(parents=True, mode=0o700)
    counters = {'cache': 0, 'planner': 0}
    mapping = {}
    for run, task, files in inputs:
        counters[task] += 1
        alias = f'{task}-{counters[task]}'
        for name, expected in files.items():
            content = (run/'workspace'/name).read_bytes()
            if hashlib.sha256(content).hexdigest() != expected:
                raise ValueError('review input changed while copying')
            target = destination/'outputs'/alias/name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        mapping[alias] = {'run': run.name, 'files': files}
    private_json(destination/'identity.json', mapping)
    print(json.dumps({'destination': str(destination), 'outputs': sorted(mapping)}))


def cross_tests(review, node):
    mapping = json.loads((review/'identity.json').read_text())
    for alias, entry in mapping.items():
        if workspace_files(review/'outputs'/alias) != entry['files']:
            raise ValueError('masked output changed before cross testing')
    destination = review/'cross-suites'
    destination.mkdir(mode=0o700)
    rows = []
    for task in ('cache', 'planner'):
        outputs = sorted((review/'outputs').glob(f'{task}-*'))
        for implementation in outputs:
            for suite in outputs:
                target = destination/f'{implementation.name}-suite-{suite.name}'
                # Examples and documentation are fixtures owned by the test suite.
                # Only implementation code comes from the other participant.
                target.mkdir()
                for file in suite.iterdir():
                    if file.name == 'src':
                        continue
                    if file.is_dir(): shutil.copytree(file, target/file.name)
                    else: shutil.copyfile(file, target/file.name)
                shutil.copytree(implementation/'src', target/'src')
                try:
                    result = subprocess.run([str(node), '--test', '--test-reporter=tap'], cwd=target,
                                            capture_output=True, text=True, timeout=30)
                    (target/'result.log').write_text(result.stdout + result.stderr)
                    row = {'implementation': implementation.name, 'suite': suite.name,
                           'exit': result.returncode,
                           'summary': [line for line in result.stdout.splitlines()
                                       if line.startswith(('# tests', '# pass', '# fail', 'not ok'))]}
                except subprocess.TimeoutExpired:
                    row = {'implementation': implementation.name, 'suite': suite.name,
                           'exit': 124, 'error': 'suite timed out'}
                rows.append(row)
    private_json(review/'cross-suites.json', rows)
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'cross'])
    parser.add_argument('root', type=Path)
    parser.add_argument('destination', type=Path, nargs='?')
    parser.add_argument('--node', type=Path)
    args = parser.parse_args()
    if args.action == 'prepare':
        if not args.destination: parser.error('prepare requires a destination')
        prepare(args.root.resolve(), args.destination.resolve())
    else:
        if not args.node or args.destination: parser.error('cross requires --node and no destination')
        cross_tests(args.root.resolve(), args.node.resolve())
