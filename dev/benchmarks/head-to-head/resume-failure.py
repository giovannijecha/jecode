"""Continue a preserved failed trial in a copied workspace, outside timed rankings."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from prepare import private_json
from recover import CONTINUE, nodes, observe, submit, exit_tui, workspace_hashes, request_count
from run import jecode_setup, sha
from terminal import Terminal


def main(root, failed_name, name, source, timeout):
    if root.parent != Path('/var/tmp') or root.stat().st_mode & 0o077:
        raise ValueError('expected a private laboratory directly under /var/tmp')
    if not source.is_relative_to(root):
        raise ValueError('recovery source must be a laboratory snapshot')
    original = root/'runs'/failed_name
    if json.loads((original/'outcome.json').read_text())['status'] != 'failed':
        raise ValueError('expected a settled failed original trial')
    if not json.loads((root/'auth-status.json').read_text()).get('sameAccount'):
        raise ValueError('matching account not verified')
    before = workspace_hashes(original)
    run = root/'runs'/name; run.mkdir(mode=0o700)
    workspace, home = run/'workspace', run/'home'
    shutil.copytree(original/'workspace', workspace, symlinks=True)
    home.mkdir(mode=0o700)
    staging = run/'import'; staging.mkdir(mode=0o700)
    shutil.copytree(original/'home/sessions', staging/'sessions', symlinks=True)
    for filename in ('accounts.json', 'settings.json'):
        shutil.copyfile(root/'jecode-home'/filename, home/filename)
        (home/filename).chmod(0o600)
    environment = json.loads((root/'environment.json').read_text())
    node = environment['node']
    env = {k:v for k,v in os.environ.items() if k in ('HOME','USER','LOGNAME','LANG','LC_ALL','TZ')}
    env.update(PATH=f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin', TERM='xterm-256color',
               NO_COLOR='1', JECODE_HOME=str(home), CODEX_HOME=str(home))
    session_files = list((staging/'sessions').glob('*/*/meta.json'))
    if len(session_files) != 1: raise ValueError('expected one original session')
    session_id = json.loads(session_files[0].read_text())['id']
    clone = Path(__file__).with_name('clone-session.mjs')
    with (run/'clone.json').open('w') as log:
        subprocess.run([node, str(clone), str(source), str(original/'workspace'),
                        str(staging/'sessions'), session_id, str(workspace), str(home/'sessions')],
                       env=env, stdout=log, check=True, timeout=30)
    private_json(run/'manifest.json', {'client':'jecode','task':'durable','recovery':True,
        'preflight':False,'originalRun':failed_name,'sourceSnapshot':str(source),
        'excludedFromTimedComparison':True,'continuation':CONTINUE,'environment':environment,
        'cloneMethod':'load staged copy and publish unchanged conversation into copied workspace',
        'harness':{f.name:sha(f) for f in (Path(__file__),clone)}})
    original_nodes = {file:sha(Path(file)) for file in nodes(home)}
    files = workspace_hashes(workspace)
    terminal = None
    evidence = {}
    outcome = {'status':'harness-error'}
    log = (run/'resume-terminal.bin').open('wb')
    try:
        terminal = Terminal([node,str(source/'dev/context/record.ts'),'-c'], workspace, env, log)
        jecode_setup(terminal); terminal.pause(2)
        evidence.update(workspaceUnchangedBeforeNewInput=workspace_hashes(workspace)==files,
            historicalNodesUnchangedBeforeNewInput=all(sha(Path(f))==h for f,h in original_nodes.items()),
            requestsRecordedBeforeNewInput=request_count(home))
        if not (evidence['workspaceUnchangedBeforeNewInput'] and
                evidence['historicalNodesUnchangedBeforeNewInput'] and request_count(home)==0):
            raise RuntimeError('resume changed saved work before new input')
        (run/'resumed-screen.txt').write_text(terminal.text())
        continued = submit(terminal, CONTINUE)
        final = observe(terminal, home, lambda saved: next((n for f,n in saved.items()
            if f not in original_nodes and n['settlement']!='checkpointed'), None), timeout)
        outcome = {'status':final['settlement'], 'continuationMs':(time.monotonic_ns()-continued)/1e6}
        evidence['historicalNodesPreserved'] = all(sha(Path(f))==h for f,h in original_nodes.items())
        (run/'final-screen.txt').write_text(terminal.text())
        exit_tui(terminal); terminal = None
    except Exception as error:
        outcome = {'status':'harness-error','error':str(error)}
    finally:
        try:
            if terminal: terminal.close()
        except Exception as error:
            outcome = {'status':'harness-error','cleanupError':str(error),'previousOutcome':outcome}
        finally:
            log.close()
        evidence['originalTrialUnchanged'] = workspace_hashes(original)==before
        private_json(run/'recovery.json', evidence)
        private_json(run/'outcome.json', outcome)
    print(json.dumps({'run':name,'outcome':outcome,'recovery':evidence}),flush=True)
    return int(outcome['status']!='completed' or not evidence.get('historicalNodesPreserved') or
               not evidence['originalTrialUnchanged'])


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root',type=Path)
    parser.add_argument('--failed',required=True)
    parser.add_argument('--name',required=True)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--timeout',type=int,default=1200)
    args=parser.parse_args()
    if not all(re.fullmatch(r'[a-z0-9-]+', value) for value in (args.failed,args.name)) or args.timeout <= 0:
        parser.error('invalid run name or timeout')
    raise SystemExit(main(args.root.resolve(), args.failed, args.name, args.source.resolve(), args.timeout))
