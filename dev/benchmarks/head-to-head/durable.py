"""Materialize the fixed durable-execution starter from a prior frozen laboratory."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from prepare import private_json


def materialize(root, previous):
    source = previous/'harness/tasks/planner-progress/fixture'
    destination = root/'harness/tasks/durable'
    if any(file.is_symlink() for file in source.rglob('*')):
        raise ValueError('starter must not contain symbolic links')
    files = {str(file.relative_to(source)): hashlib.sha256(file.read_bytes()).hexdigest()
             for file in source.rglob('*') if file.is_file()}
    expected = '11661ff6ba4a0141f887f79bbd830d391f45512bfa5fb65f34695158064ba3a2'
    # The prior manifest records installed test names, before the harness adds
    # .template to keep fixtures out of Jecode's own test discovery.
    installed = {name.removesuffix('.template'): value for name, value in files.items()}
    if len(installed) != len(files):
        raise ValueError('ambiguous materialized fixture names')
    actual = hashlib.sha256(json.dumps(installed, sort_keys=True).encode()).hexdigest()
    if actual != expected:
        raise ValueError('starter differs from the recorded integration fixture')
    if (destination/'fixture').exists():
        present = {str(file.relative_to(destination/'fixture')): hashlib.sha256(file.read_bytes()).hexdigest()
                   for file in (destination/'fixture').rglob('*') if file.is_file()}
        if present != files:
            raise ValueError('existing starter differs; refusing to replace it')
    else:
        shutil.copytree(source, destination/'fixture')
    shutil.copyfile(Path(__file__).parent/'tasks/planner/acceptance.mjs', destination/'planner-acceptance.mjs')
    private_json(destination/'STARTER.json', {'source': str(source), 'sha256': actual, 'files': files})
    print(json.dumps({'starterFiles': len(files), 'starterHash': actual}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path); parser.add_argument('previous', type=Path)
    args = parser.parse_args()
    materialize(args.root.resolve(), args.previous.resolve())
