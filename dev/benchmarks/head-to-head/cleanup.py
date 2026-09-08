"""Remove only the laboratory's temporary account copies after all runs settle."""

import argparse
import json
from pathlib import Path


def cleanup(root):
    root = root.resolve()
    if root.parent != Path('/var/tmp') or root.stat().st_mode & 0o077:
        raise ValueError('expected a private laboratory directly under /var/tmp')
    for name in ('environment.json', 'snapshot.json'):
        json.loads((root/name).read_text())
    runs = list((root/'runs').iterdir()) if (root/'runs').exists() else []
    if any(not (run/'outcome.json').is_file() for run in runs):
        raise ValueError('a run has not settled; keep its account available')
    homes = [root/'codex-home', root/'jecode-home', *(run/'home' for run in runs)]
    targets = [home/name for home in homes for name in ('auth.json', 'accounts.json')]
    # Check every resolved target before deleting any file, including parent links.
    for target in targets:
        if target.is_symlink() or not target.resolve().is_relative_to(root):
            raise ValueError('account path escapes the laboratory')
        if target.exists() and not target.is_file():
            raise ValueError('expected a regular account file')
    removed = 0
    for target in targets:
        if target.exists():
            target.unlink()
            removed += 1
    print(json.dumps({'removedTemporaryAccountFiles': removed,
                      'productionAccountsModified': False}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    cleanup(args.root)
