"""Check native subprocess capture under named Codex permission profiles, offline."""

import argparse
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--codex', type=Path, required=True)
    parser.add_argument('--node', type=Path, action='append', required=True)
    args = parser.parse_args()
    directory = args.output.resolve()
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    probe = directory/'spawn-probe.mjs'
    probe.write_bytes(Path(__file__).with_name('spawn-probe.mjs').read_bytes())
    rows = []
    for index, node in enumerate(args.node):
        for profile, extra in [('default', ''), ('unix', 'dangerously_allow_all_unix_sockets = true\n'),
                               ('network', 'enabled = true\n')]:
            home = directory/f'home-{index}-{profile}'
            home.mkdir(mode=0o700)
            (home/'config.toml').write_text(
                '[permissions.probe]\nextends = ":workspace"\n[permissions.probe.network]\n'+extra)
            env = dict(os.environ, CODEX_HOME=str(home),
                       PATH=f'{node.parent}:/usr/local/bin:/usr/bin:/bin')
            result = subprocess.run([str(args.codex), 'sandbox', '-P', 'probe', '-C', str(directory),
                                     '--', str(node), str(probe)], env=env, capture_output=True,
                                    text=True, timeout=30)
            rows.append({'node': str(node), 'profile': profile, 'exit': result.returncode,
                         'stdout': result.stdout, 'stderr': result.stderr})
    (directory/'results.json').write_text(json.dumps(rows, indent=2)+'\n')
    print(json.dumps(rows, indent=2))


if __name__ == '__main__':
    main()
