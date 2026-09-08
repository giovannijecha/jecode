"""Recheck finished outputs and provenance without changing original measurements."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from prepare import private_json
from scenarios import source_variant, task_directory
from provenance import evaluator_files

HERE = Path(__file__).resolve().parent


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def verify(root, own_tests):
    environment = json.loads((root/'environment.json').read_text())
    node = environment['node']
    changed = []
    reports = []
    separate_recovery = []
    for run in sorted((root/'runs').iterdir()):
        if not (run/'outcome.json').exists():
            continue
        manifest = json.loads((run/'manifest.json').read_text())
        if manifest['preflight']:
            continue
        if manifest.get('recovery') is True and manifest.get('excludedFromTimedComparison') is True:
            separate_recovery.append(run.name)
            continue
        outcome = json.loads((run/'outcome.json').read_text())
        source, snapshot = source_variant(root, manifest.get('variant','baseline'))
        changed_source = [name for name, expected in snapshot['files'].items()
                          if not (source/name).is_file() or digest(source/name) != expected]
        changed.extend(f'{source}/{name}' for name in changed_source)
        task = task_directory(manifest.get('task','ledger'))
        fixture = {str(file.relative_to(task/'fixture')).removesuffix('.template'): digest(file)
                   for file in (task/'fixture').rglob('*') if file.is_file()}
        prompt_hash = digest(task/'PROMPT.md')
        evaluator_hash = digest(task/'acceptance.mjs')
        workspace = run/'workspace'
        report = {
            'run': run.name,
            'runStatus': outcome['status'],
            'sourceUnchanged': not changed_source,
            'sameSourceSnapshot': manifest['snapshotHash'] == snapshot['sha256'],
            'samePrompt': manifest['promptHash'] == prompt_hash,
            'sameFixture': manifest['fixture'] == fixture,
            'acceptanceHash': evaluator_hash,
            'originalAcceptanceHash': manifest['acceptanceHash'],
            'sameAcceptance': manifest['acceptanceHash'] == evaluator_hash,
            'evaluatorScope': 'module-tree' if 'evaluatorFiles' in manifest else 'legacy-entrypoint',
            'sameEvaluatorFiles': manifest.get('evaluatorFiles', evaluator_files(task)) == evaluator_files(task),
            'workspaceLinks': [str(file.relative_to(workspace))
                               for file in workspace.rglob('*') if file.is_symlink()],
        }
        environment_copy = {'PATH': f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin',
                            'HOME': str(run/'home'), 'NO_COLOR': '1', 'LANG': 'C.UTF-8'}
        with (run/'acceptance-recheck.json').open('w') as output, (run/'acceptance-recheck.stderr').open('w') as errors:
            try:
                result = subprocess.run([node, str(task/'acceptance.mjs'), str(workspace)],
                                        cwd=workspace, env=environment_copy,
                                        stdout=output, stderr=errors, timeout=30)
                report['acceptanceExit'] = result.returncode
            except subprocess.TimeoutExpired:
                report['acceptanceExit'] = 124
        if own_tests:
            with (run/'independent-tests.log').open('w') as log:
                try:
                    result = subprocess.run([node, '--test'], cwd=workspace, env=environment_copy,
                                            stdout=log, stderr=subprocess.STDOUT, timeout=30)
                    report['ownTestsExit'] = result.returncode
                except subprocess.TimeoutExpired:
                    report['ownTestsExit'] = 124
        history = subprocess.run(['git', 'rev-list', '--count', 'HEAD'], cwd=workspace,
                                 text=True, capture_output=True, check=True)
        report['gitCommitCount'] = int(history.stdout)
        status = subprocess.run(['git', 'status', '--porcelain'], cwd=workspace,
                                text=True, capture_output=True, check=True)
        report['changedPaths'] = status.stdout.splitlines()
        package = json.loads((workspace/'package.json').read_text())
        report['dependencyCount'] = sum(len(package.get(key, {})) for key in
                                        ('dependencies', 'devDependencies', 'optionalDependencies'))
        private_json(run/'verification.json', report)
        reports.append(report)
    declared = {entry['name'] for file in root.glob('batch-*.json')
                for entry in json.loads(file.read_text())['plan']}
    missing = sorted(declared - {row['run'] for row in reports})
    result = {'changedSnapshotFiles': changed, 'missingDeclaredRuns': missing,
              'separateRecoveryRuns': separate_recovery, 'runs': reports}
    private_json(root/'verification.json', result)
    print(json.dumps(result, indent=2))
    passed = all(row['runStatus'] == 'completed' and row['sourceUnchanged'] and row['sameSourceSnapshot'] and row['samePrompt']
                 and row['sameFixture'] and row['sameAcceptance'] and row['sameEvaluatorFiles'] and row['acceptanceExit'] == 0
                 and row.get('ownTestsExit', 0) == 0 and row['gitCommitCount'] == 1
                 and row['dependencyCount'] == 0 and not row['workspaceLinks'] for row in reports)
    return 0 if reports and passed and not missing else 1


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--own-tests', action='store_true')
    args = parser.parse_args()
    raise SystemExit(verify(args.root.resolve(), args.own_tests))
