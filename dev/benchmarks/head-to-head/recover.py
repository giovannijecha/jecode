"""Interrupt after a persisted write, restart the real TUI, and finish the task."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from prepare import private_json
from run import fixture_copy, jecode_setup, sha
from scenarios import TASKS, VARIANTS, source_variant, task_directory
from terminal import Terminal

CONTINUE = ('Continue the original task from the saved state. Inspect the existing '
            'files first, preserve completed work, and finish the implementation, '
            'regression tests, documentation and verification. Report the result.')


def nodes(home):
    output = {}
    for file in home.glob('sessions/*/*/nodes/*.json'):
        try: output[str(file)] = json.loads(file.read_text())['node']
        except (OSError, ValueError): pass
    return output


def successful_writes(node):
    return sum(block.get('kind') == 'tool' and block.get('name') in ('edit_file','write_file')
               and block.get('tone') == 'ok' for block in node.get('blocks',[]))


def workspace_hashes(workspace):
    return {str(file.relative_to(workspace)):sha(file) for file in workspace.rglob('*')
            if file.is_file() and '.git' not in file.parts}


def request_count(home):
    count = 0
    for file in (home/'diagnostics').glob('*.jsonl'):
        for line in file.read_text().split('\n')[:-1]:
            if line.strip() and json.loads(line).get('kind') == 'request': count += 1
    return count


def observe(terminal, home, predicate, timeout):
    deadline = time.monotonic() + timeout
    last_status = time.monotonic()
    while time.monotonic() < deadline:
        terminal.pump()
        saved = nodes(home)
        result = predicate(saved)
        if result: return result
        if terminal.dead: raise RuntimeError('TUI exited before the expected recovery state')
        if time.monotonic() - last_status > 30:
            print(json.dumps({'phase':'recovery','remainingSeconds':round(deadline-time.monotonic())}),flush=True)
            last_status = time.monotonic()
    raise TimeoutError('recovery phase timed out')


def submit(terminal, text):
    terminal.send('\x1b[200~'+text+'\x1b[201~'); terminal.pause(.5)
    stamp = time.monotonic_ns(); terminal.send('\r')
    return stamp


def exit_tui(terminal):
    terminal.send('/exit'); terminal.pause(.15); terminal.send('\r'); terminal.pause(1)
    terminal.close()


def main(args):
    root = args.root.resolve()
    if not json.loads((root/'auth-status.json').read_text()).get('sameAccount'):
        raise ValueError('matching account not verified')
    source, snapshot = source_variant(root,args.variant)
    task = task_directory(args.task)
    environment = json.loads((root/'environment.json').read_text())
    node = environment['node']
    run = root/'runs'/args.name
    run.mkdir(parents=True,mode=0o700)
    workspace,home = run/'workspace',run/'home'
    fixture_copy(workspace,args.task); home.mkdir(mode=0o700)
    for name in ('accounts.json','settings.json'):
        shutil.copyfile(root/'jecode-home'/name,home/name); (home/name).chmod(0o600)
    env = {key:value for key,value in os.environ.items() if key in ('HOME','USER','LOGNAME','LANG','LC_ALL','TZ')}
    env.update(PATH=f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin',TERM='xterm-256color',
               NO_COLOR='1',JECODE_HOME=str(home),CODEX_HOME=str(home))
    private_json(run/'manifest.json',{
        'client':'jecode','preflight':False,'recovery':True,'task':args.task,'variant':args.variant,
        'sourceSnapshot':str(source),'snapshotHash':snapshot['sha256'],'environment':environment,
        'bootId':Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
        'promptHash':sha(task/'PROMPT.md'),'acceptanceHash':sha(task/'acceptance.mjs'),
        'fixture':workspace_hashes(workspace),'continuation':CONTINUE,'dimensions':[140,40],
        'trigger':'first persisted successful edit/write during an unsettled turn',
        'harness':{f.name:sha(f) for f in Path(__file__).parent.glob('*.py')},
    })
    command = [node,str(source/'dev/context/record.ts')]
    terminal = None; logs = []; evidence = {}; outcome = {'status':'harness-error'}
    started = None
    try:
        logs.append((run/'terminal.bin').open('wb'))
        terminal = Terminal(command,workspace,env,logs[-1]); jecode_setup(terminal)
        started = submit(terminal,(task/'PROMPT.md').read_text())
        private_json(run/'start.json',{'monotonicNs':started,'atNs':time.time_ns()})
        def written(saved):
            for value in saved.values():
                if value['settlement'] != 'checkpointed':
                    raise RuntimeError('turn settled before the interruption trigger')
                if successful_writes(value): return True
            return False
        observe(terminal,home,written,args.timeout)
        interrupted = time.monotonic_ns(); terminal.send('\x1b')
        observe(terminal,home,lambda saved: any(n['settlement']=='interrupted' for n in saved.values()),20)
        evidence['interruptionMs'] = (time.monotonic_ns()-interrupted)/1e6
        (run/'interrupted-screen.txt').write_text(terminal.text())
        exit_tui(terminal)
        evidence['interruptedTerminalLifetime'] = terminal.observations
        terminal = None
        original_nodes = {file:sha(Path(file)) for file in nodes(home)}
        files = workspace_hashes(workspace); count = request_count(home)
        private_json(run/'before-resume.json',{'nodes':original_nodes,'workspace':files,'requests':count})
        logs.append((run/'resume-terminal.bin').open('wb'))
        terminal = Terminal(command+['-c'],workspace,env,logs[-1]); jecode_setup(terminal)
        terminal.pause(2)
        evidence['workspaceUnchangedBeforeNewInput'] = workspace_hashes(workspace) == files
        evidence['historicalNodesUnchangedBeforeNewInput'] = all(sha(Path(file))==value for file,value in original_nodes.items())
        evidence['requestsRecordedBeforeNewInput'] = request_count(home)-count
        evidence['idleFooterBeforeNewInput'] = not re.search(
            r'(Thinking|Preparing|Running|Compacting)\s*[·]',terminal.text().splitlines()[-1])
        if not all((evidence['workspaceUnchangedBeforeNewInput'],evidence['historicalNodesUnchangedBeforeNewInput'],
                    evidence['requestsRecordedBeforeNewInput']==0,evidence['idleFooterBeforeNewInput'])):
            raise RuntimeError('resume changed historical work before new user input')
        (run/'resumed-screen.txt').write_text(terminal.text())
        continued = submit(terminal,CONTINUE)
        def settled(saved):
            return next((value for file,value in saved.items()
                         if file not in original_nodes and value['settlement']!='checkpointed'),None)
        final = observe(terminal,home,settled,args.timeout)
        finished = time.monotonic_ns()
        outcome = {'status':final['settlement'],'elapsedMs':(finished-started)/1e6}
        evidence.update(beforeInterruptionMs=(interrupted-started)/1e6,continuationMs=(finished-continued)/1e6,
                        restartAndSetupMs=(continued-interrupted)/1e6,
                        historicalNodesPreserved=all(sha(Path(file))==value for file,value in original_nodes.items()),
                        settlements=[n['settlement'] for n in nodes(home).values()])
        (run/'final-screen.txt').write_text(terminal.text())
        exit_tui(terminal)
        evidence['resumedTerminalLifetime'] = terminal.observations
        terminal = None
    except Exception as error:
        outcome = {'status':'harness-error','error':str(error)}
    finally:
        if terminal:
            (run/'failure-screen.txt').write_text(terminal.text())
            try: terminal.close()
            except (OSError, ValueError) as error:
                outcome.update(status='harness-error', cleanupError=str(error))
            evidence['failedTerminalLifetime'] = terminal.observations
        for log in logs: log.close()
        private_json(run/'recovery.json',evidence)
        private_json(run/'outcome.json',outcome)
    with (run/'acceptance.json').open('w') as log, (run/'acceptance.stderr').open('w') as errors:
        try:
            result = subprocess.run([node,str(task/'acceptance.mjs'),str(workspace)],cwd=workspace,
                                    env=env,stdout=log,stderr=errors,timeout=30)
            outcome['acceptanceExit'] = result.returncode
        except subprocess.TimeoutExpired: outcome['acceptanceExit'] = 124
    private_json(run/'outcome.json',outcome)
    print(json.dumps({'run':args.name,'outcome':outcome,'recovery':evidence}),flush=True)
    return int(outcome['status']!='completed' or outcome['acceptanceExit']!=0 or not evidence.get('historicalNodesPreserved'))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--name',required=True)
    parser.add_argument('--task',choices=TASKS,default='cache')
    parser.add_argument('--variant',choices=VARIANTS,default='baseline')
    parser.add_argument('--timeout',type=int,default=1200)
    args=parser.parse_args()
    if not re.fullmatch(r'[a-z0-9-]+',args.name): parser.error('invalid run name')
    raise SystemExit(main(args))
