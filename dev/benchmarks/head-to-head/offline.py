"""Check the public laboratory without accounts, Codex, or private starters."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    root = Path(__file__).resolve().parent
    if os.name != 'posix':
        raise SystemExit('The PTY laboratory requires Linux/WSL or macOS; CI uses Linux.')
    if not Path('/var/tmp').is_dir():
        raise SystemExit('The cleanup tests require /var/tmp; do not weaken their path boundary.')
    with tempfile.TemporaryDirectory(prefix='jecode-lab-offline-') as temporary:
        env = {key: value for key, value in os.environ.items()
               if key in ('PATH', 'SYSTEMROOT', 'TMPDIR', 'LANG', 'LC_ALL')}
        env.update(HOME=temporary, JECODE_HOME=str(Path(temporary)/'jecode'),
                   NO_COLOR='1', PYTHONDONTWRITEBYTECODE='1')
        commands = [
            [sys.executable, '-B', '-m', 'unittest', 'discover', '-p', 'test_*.py'],
            ['node', '../validate-probes.ts'],
            *[['node', f'tasks/{task}/validate-evaluator.mjs'] for task in ('config-edit', 'file-server')],
        ]
        for command in commands:
            print('Running:', ' '.join(command), flush=True)
            subprocess.run(command, cwd=root, env=env, check=True, timeout=180)


if __name__ == '__main__':
    main()
