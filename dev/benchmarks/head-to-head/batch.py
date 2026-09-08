"""Run a declared comparison sequence serially, keeping the WSL guest alive."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

from prepare import private_json
from scenarios import TASKS, VARIANTS
from precheck import main as precheck


def validate_plan(plan):
    if not isinstance(plan, list) or not plan:
        raise ValueError('plan must contain at least one run')
    names = set()
    for run in plan:
        if not isinstance(run, dict) or set(run) != {'name','client','task','variant'}:
            raise ValueError('each run requires exactly name, client, task and variant')
        name = run['name']
        if not isinstance(name, str) or not re.fullmatch(r'[a-z0-9-]+',name) or name in names:
            raise ValueError('run names must be valid and unique')
        names.add(name)
        if run['client'] not in ('jecode','codex') or run['task'] not in TASKS or run['variant'] not in VARIANTS:
            raise ValueError('unknown client, task or variant')
        if run['client'] == 'codex' and run['variant'] != 'baseline':
            raise ValueError('Codex retains its native instructions')
    return plan


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--prefix', required=True)
    parser.add_argument('--first', type=int, default=1)
    parser.add_argument('--timeout', type=int, default=1200)
    parser.add_argument('--precheck', action='store_true', help='require offline preparation to pass before timing')
    selection = parser.add_mutually_exclusive_group(required=True)
    selection.add_argument('--clients', nargs='+', choices=('jecode', 'codex'))
    selection.add_argument('--plan', type=Path)
    args = parser.parse_args()
    if not re.fullmatch(r'[a-z0-9-]+', args.prefix):
        parser.error('invalid batch prefix')
    if args.first < 1:
        parser.error('--first must be positive')
    if args.timeout < 1:
        parser.error('--timeout must be positive')
    counters = {'jecode': args.first, 'codex': args.first}
    plan = []
    for client in args.clients or []:
        plan.append({'client': client, 'name': f'{args.prefix}-{client}-{counters[client]}',
                     'task':'ledger', 'variant':'baseline'})
        counters[client] += 1
    plan = validate_plan(json.loads(args.plan.read_text()) if args.plan else plan)
    root = args.root.resolve()
    manifest = root/f'batch-{args.prefix}.json'
    if manifest.exists() or any((root/'runs'/run['name']).exists() for run in plan):
        parser.error('batch names must be new')
    if args.precheck:
        tasks = list(dict.fromkeys(run['task'] for run in plan))
        variants = list(dict.fromkeys(run['variant'] for run in plan if run['client'] == 'jecode'))
        if precheck(root, tasks, variants) != 0:
            return 1
    private_json(manifest, {'plan': plan, 'timeoutSeconds':args.timeout,
                           'bootId': Path('/proc/sys/kernel/random/boot_id').read_text().strip()})
    failed = False
    for run in plan:
        result = subprocess.run([sys.executable, str(Path(__file__).with_name('run.py')),
                                 '--root', str(root), '--client', run['client'], '--name', run['name'],
                                 '--task',run['task'],'--variant',run['variant'],'--timeout',str(args.timeout)])
        failed |= result.returncode != 0
        outcome = root/'runs'/run['name']/'outcome.json'
        if not outcome.exists() or json.loads(outcome.read_text())['status'] in ('setup-failed', 'harness-error'):
            # A broken measurement environment is not a model quality observation.
            break
    return int(failed)


if __name__ == '__main__':
    raise SystemExit(main())
