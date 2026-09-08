"""Validate the experiment fixtures and source before measuring live runs."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from prepare import private_json
from run import codex_config, fixture_copy
from scenarios import TASKS, VARIANTS, source_variant, task_directory


def main(root, tasks=('cache', 'planner'), variants=('grouped',)):
    environment = json.loads((root/'environment.json').read_text())
    node = environment['node']
    env = dict(os.environ, PATH=f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin')
    reports = []
    attempts = Path(tempfile.mkdtemp(prefix='fixture-checks-',dir=root))
    for task in tasks:
        output = attempts/task
        output.mkdir(parents=True)
        workspace = output/'workspace'
        fixture_copy(workspace,task)
        row = {'task':task}
        for label, command in (
            ('smoke',[node,'--test']),
            ('acceptance',[node,str(task_directory(task)/'acceptance.mjs'),str(workspace)]),
        ):
            with (output/f'{label}.json').open('w') as log, (output/f'{label}.stderr').open('w') as errors:
                result = subprocess.run(command,cwd=workspace,env=env,stdout=log,stderr=errors,timeout=30)
            row[label+'Exit'] = result.returncode
        acceptance = json.loads((output/'acceptance.json').read_text())
        row.update({key:acceptance[key] for key in ('total','passed')})
        reports.append(row)
    gates = []
    home = attempts/'sandbox-home'; home.mkdir(mode=0o700)
    codex_config(home, attempts, attempts/'unused-notification.json', 'http://127.0.0.1:1',
                 environment.get('codexCommandNetwork', False))
    if not environment.get('codexCommandNetwork', False):
        with (home/'config.toml').open('a') as config:
            config.write('\n[permissions.benchmark]\nextends = ":workspace"\n')
    sandbox = [str(root/'codex/node_modules/.bin/codex'), 'sandbox', '-P', 'benchmark']
    commands = [
        ('harness', [sys.executable, '-m', 'unittest', 'discover', '-p', 'test_*.py'], Path(__file__).parent, env),
        ('sandbox-spawn', [*sandbox, '-C', str(attempts), '--', node, str(Path(__file__).with_name('spawn-probe.mjs'))],
         attempts, dict(env, CODEX_HOME=str(home))),
    ]
    for variant in dict.fromkeys(['baseline', *variants]):
        source, _ = source_variant(root, variant)
        for label, suffix in [('typecheck', ['run', 'typecheck']), ('test', ['test'])]:
            commands.append((f'{variant}-{label}', [str(Path(node).parent/'npm'), *suffix], source, env))
    for task in tasks:
        validator = task_directory(task)/'validate-evaluator.mjs'
        if validator.exists():
            commands.append((f'{task}-evaluator', [node, str(validator), str(attempts/task/'workspace')],
                             attempts/task/'workspace', env))
        commands.append((f'{task}-sandbox-tests', [*sandbox, '-C', str(attempts/task/'workspace'), '--', node, '--test'],
                         attempts/task/'workspace', dict(env, CODEX_HOME=str(home))))
    for label, command, cwd, command_env in commands:
        with (root/f'{label}.log').open('w') as log:
            result = subprocess.run(command,cwd=cwd,env=command_env,stdout=log,stderr=subprocess.STDOUT,timeout=240)
        gates.append({'gate':label,'exit':result.returncode})
        print(json.dumps(gates[-1]),flush=True)
    private_json(root/'precheck.json',{'directory':str(attempts),'fixtures':reports,'gates':gates})
    print(json.dumps(reports),flush=True)
    return int(not all(row['smokeExit']==0 and row['acceptanceExit']==1 and 0<row['passed']<row['total']
                       for row in reports) or any(row['exit']!=0 for row in gates))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    parser.add_argument('--tasks', nargs='+', choices=TASKS, default=['cache', 'planner'])
    parser.add_argument('--variants', nargs='+', choices=VARIANTS, default=['grouped'])
    args = parser.parse_args()
    raise SystemExit(main(args.root.resolve(), args.tasks, args.variants))
