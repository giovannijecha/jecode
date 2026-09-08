"""Run one bounded pilot through a real TUI, then evaluate the frozen acceptance checks."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

from prepare import private_json
from telemetry import Collector
from terminal import Terminal
from scenarios import TASKS, VARIANTS, source_variant, task_directory
from provenance import evaluator_files

HERE = Path(__file__).resolve().parent


def sha(file): return hashlib.sha256(file.read_bytes()).hexdigest()


def fixture_copy(destination, task='ledger'):
    shutil.copytree(task_directory(task)/'fixture',destination)
    # Materialize tests only inside the pilot, outside Jecode's own test discovery.
    for template in destination.rglob('*.template'):
        template.rename(template.with_suffix(''))
    # The synthetic Git baseline is part of this disposable fixture, not a maintainer commit.
    for command in (['git','init','-q'], ['git','add','.'],
                    ['git','-c','user.name=Benchmark','-c','user.email=benchmark@example.invalid',
                     'commit','-qm','Initial benchmark fixture']):
        subprocess.run(command,cwd=destination,check=True,capture_output=True)


def codex_config(home, workspace, completion, endpoint, command_network=False):
    config = f'''
model = "gpt-6-astra"
model_reasoning_effort = "high"
service_tier = "default"
approval_policy = "never"
{'default_permissions = "benchmark"' if command_network else 'sandbox_mode = "workspace-write"'}
web_search = "disabled"
check_for_update_on_startup = false
project_root_markers = [".git"]
notify = ["/usr/bin/python3", {json.dumps(str(HERE/'notify.py'))}, {json.dumps(str(completion))}]
[agents]
enabled = false
[features]
multi_agent = false
apps = false
browser_use = false
computer_use = false
image_generation = false
plugins = false
memories = false
fast_mode = false
[memories]
generate_memories = false
use_memories = false
[analytics]
enabled = false
[otel]
environment = "local-head-to-head"
log_user_prompt = false
exporter = {{ otlp-http = {{ endpoint = {json.dumps(endpoint)}, protocol = "json" }} }}
trace_exporter = "none"
metrics_exporter = "none"
[projects.{json.dumps(str(workspace))}]
trust_level = "trusted"
[tui]
notifications = false
'''
    (home/'config.toml').write_text(config.strip()+'\n')
    if command_network:
        with (home/'config.toml').open('a') as file:
            file.write('\n[permissions.benchmark]\nextends = ":workspace"\n'
                       '[permissions.benchmark.network]\nenabled = true\n')


def jecode_setup(terminal):
    terminal.wait_for(r'OpenAI Account.*gpt-6-astra.*high',30)
    terminal.send('/permissions')
    terminal.pause(.2)
    terminal.send('\r')
    terminal.wait_for(r'read_file.*allow')
    # The production registry orders four shared-read tools, then edit/write/command.
    for _ in range(4):
        terminal.send('\x1b[B'); terminal.pause(.05)
    for index in range(3):
        terminal.send('\x1b[C'); terminal.pause(.1)
        if index < 2:
            terminal.send('\x1b[B'); terminal.pause(.05)
    for tool in ('edit_file','write_file','run_command'):
        terminal.wait_for(rf'{tool}.*allow')
    terminal.send('\x1b'); terminal.pause(.4)


def jecode_completed(home):
    for file in home.glob('sessions/*/*/nodes/*.json'):
        try:
            saved=json.loads(file.read_text())
            node=saved.get('node',{})
            if node.get('settlement') in ('completed','failed','interrupted'):
                return {'settlement':node['settlement'],'file':str(file),'updatedAt':saved.get('updatedAt')}
        except (OSError,ValueError):
            pass
    return None


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',required=True,type=Path)
    parser.add_argument('--client',required=True,choices=['jecode','codex'])
    parser.add_argument('--name',required=True)
    parser.add_argument('--preflight',action='store_true')
    parser.add_argument('--timeout',type=int,default=1200)
    parser.add_argument('--task',choices=TASKS,default='ledger')
    parser.add_argument('--variant',choices=VARIANTS,default='baseline')
    args=parser.parse_args()
    if args.timeout < 1: parser.error('--timeout must be positive')
    if not re.fullmatch(r'[a-z0-9-]+',args.name): parser.error('invalid run name')
    root=args.root.resolve()
    if args.client=='codex' and args.variant!='baseline': parser.error('Codex uses its native instructions')
    task=task_directory(args.task)
    source,snapshot=source_variant(root,args.variant)
    environment=json.loads((root/'environment.json').read_text())
    if not json.loads((root/'auth-status.json').read_text()).get('sameAccount'):
        raise RuntimeError('matching account identities have not been verified')
    node=environment['node']
    run=root/'runs'/args.name
    run.mkdir(parents=True,mode=0o700)
    workspace=run/'workspace'
    fixture_copy(workspace,args.task)
    home=run/'home'; home.mkdir(mode=0o700)
    seed=root/f'{args.client}-home'
    for name in ('auth.json','accounts.json','settings.json'):
        if (seed/name).is_file():
            shutil.copyfile(seed/name,home/name); (home/name).chmod(0o600)
    env={key:value for key,value in os.environ.items()
         if key in ('HOME','USER','LOGNAME','LANG','LC_ALL','TZ')}
    env.update(PATH=f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin',
               TERM='xterm-256color',COLORTERM='truecolor',NO_COLOR='1',
               CODEX_HOME=str(home),JECODE_HOME=str(home))
    completion=run/'completion.json'
    collector=Collector(run/'codex-otel.jsonl') if args.client=='codex' else None
    if collector:
        command_network = environment.get('codexCommandNetwork', False)
        codex_config(home,workspace,completion,collector.endpoint,command_network)
        command=[str(root/'codex/node_modules/.bin/codex'),'--strict-config']
    else:
        command=[node,str(source/'dev/context/record.ts')]
    prompt=(task/'PROMPT.md').read_text()
    fixture_hash={str(f.relative_to(workspace)):sha(f) for f in workspace.rglob('*') if f.is_file() and '.git' not in f.parts}
    manifest={'client':args.client,'preflight':args.preflight,'environment':environment,
              'bootId':Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
              'task':args.task,'variant':args.variant,'sourceSnapshot':str(source),
              'snapshotHash':snapshot['sha256'],
              'promptHash':sha(task/'PROMPT.md'),'acceptanceHash':sha(task/'acceptance.mjs'),
              'evaluatorFiles':evaluator_files(task),
              'fixture':fixture_hash,'timeoutSeconds':args.timeout,'dimensions':[140,40],
              'harness':{f.name:sha(f) for f in HERE.glob('*.py')}}
    private_json(run/'manifest.json',manifest)
    terminal=None
    outcome={'status':'setup-failed'}
    started=None
    try:
        with (run/'terminal.bin').open('wb') as log:
            terminal=Terminal(command,workspace,env,log)
            if args.client=='jecode':
                jecode_setup(terminal)
            else:
                terminal.pause(4)
                # New-install onboarding can require acknowledgement before the composer appears.
                if 'Press enter' in terminal.text() or 'Press Enter' in terminal.text():
                    terminal.send('\r'); terminal.pause(2)
                if 'gpt-6-astra' not in terminal.text():
                    terminal.wait_for('gpt-6-astra',30)
            (run/'ready-screen.txt').write_text(terminal.text())
            if args.preflight:
                outcome={'status':'ready'}
            else:
                terminal.send('\x1b[200~'+prompt+'\x1b[201~')
                terminal.pause(.5)
                started=time.monotonic_ns()
                private_json(run/'start.json',{'monotonicNs':started,'atNs':time.time_ns()})
                terminal.send('\r')
                deadline=time.monotonic()+args.timeout
                last_status=0
                while time.monotonic()<deadline:
                    terminal.pump()
                    state=jecode_completed(home) if args.client=='jecode' else (
                        json.loads(completion.read_text()) if completion.exists() else None)
                    if state:
                        outcome={'status':'completed' if state.get('settlement','completed')=='completed' else state['settlement'],
                                 'elapsedMs':(time.monotonic_ns()-started)/1e6,'completion':state}
                        break
                    if terminal.dead:
                        outcome={'status':'process-exited','elapsedMs':(time.monotonic_ns()-started)/1e6}
                        break
                    if time.monotonic()-last_status>30:
                        last_status=time.monotonic()
                        elapsed=(time.monotonic_ns()-started)/1e9
                        print(json.dumps({'run':args.name,'elapsedSeconds':round(elapsed),'status':'running'}),flush=True)
                        (run/'current-screen.txt').write_text(terminal.text())
                else:
                    outcome={'status':'timeout','elapsedMs':(time.monotonic_ns()-started)/1e6}
            (run/'final-screen.txt').write_text(terminal.text())
            if not terminal.dead:
                if outcome['status'] not in ('ready','completed'):
                    terminal.send('\x03'); terminal.pause(.5)
                terminal.send('/exit'); terminal.pause(.15); terminal.send('\r'); terminal.pause(1)
    except Exception as error:
        outcome={'status':'harness-error','error':str(error)}
        if terminal: (run/'failure-screen.txt').write_text(terminal.text())
    finally:
        if terminal:
            try: terminal.close()
            except OSError as error: outcome['cleanupError']=str(error)
        if collector:
            collector.close()
            outcome['telemetry']={'records':collector.records,'errors':collector.errors}
        private_json(run/'outcome.json',outcome)
    if not args.preflight:
        verify_started=time.monotonic_ns()
        with (run/'acceptance.json').open('w') as output, (run/'acceptance.stderr').open('w') as errors:
            try:
                result=subprocess.run([node,str(task/'acceptance.mjs'),str(workspace)],cwd=workspace,
                                      env=env,stdout=output,stderr=errors,timeout=30)
                outcome['acceptanceExit']=result.returncode
            except subprocess.TimeoutExpired:
                outcome['acceptanceExit']=124
                outcome['acceptanceError']='external acceptance timed out'
        outcome['acceptanceMs']=(time.monotonic_ns()-verify_started)/1e6
        diff=subprocess.run(['git','diff','--stat'],cwd=workspace,capture_output=True,text=True,check=True)
        (run/'diff-stat.txt').write_text(diff.stdout)
        private_json(run/'outcome.json',outcome)
    print(json.dumps({'run':str(run),'outcome':outcome}),flush=True)
    return 0 if outcome['status'] in ('ready','completed') and outcome.get('acceptanceExit',0)==0 else 1


if __name__=='__main__': sys.exit(main())
