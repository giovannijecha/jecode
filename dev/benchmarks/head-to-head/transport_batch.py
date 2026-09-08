"""Serial transport-only probes, separate from TUI task-quality measurements."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
from prepare import private_json
from scenarios import source_variant


def main(root):
    environment = json.loads((root/'environment.json').read_text())
    node = environment['node']
    output = root/'transport-probes'
    output.mkdir(mode=0o700)
    order = ['websocket', 'http', 'http', 'websocket', 'websocket', 'http']
    private_json(output/'plan.json', {'order': order, 'requestsPerTurn': 2,
                                     'model': environment['model'], 'effort': environment['effort']})
    env = {key: value for key, value in os.environ.items() if key in ('HOME','LANG','TZ')}
    env.update(PATH=f'{Path(node).parent}:/usr/local/bin:/usr/bin:/bin', JECODE_HOME=str(root/'jecode-home'))
    failed = False
    for index, transport in enumerate(order):
        source, snapshot = source_variant(root, 'http' if transport == 'http' else 'baseline')
        name = f'{index+1}-{transport}'
        private_json(output/f'{name}-manifest.json', {'snapshotHash': snapshot['sha256'], 'transport': transport})
        with (output/f'{name}.log').open('w') as log:
            result = subprocess.run([node, str(source/'dev/benchmarks/head-to-head/transport-live.ts'),
                                     transport, str(output/f'{name}.json')], cwd=source, env=env,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=750)
        failed |= result.returncode != 0
        print(json.dumps({'transportProbe': name, 'exit': result.returncode}), flush=True)
    return int(failed)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    raise SystemExit(main(parser.parse_args().root.resolve()))
