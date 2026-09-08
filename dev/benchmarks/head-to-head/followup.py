"""Freeze one common follow-up fixture from identity-masked planner outputs."""

import argparse
import hashlib
import json
from pathlib import Path

from prepare import private_json
from quality import workspace_files


def freeze(root, review):
    mapping = json.loads((review/'identity.json').read_text())
    candidates = []
    for alias, entry in mapping.items():
        if not alias.startswith('planner-'):
            continue
        files = workspace_files(review/'outputs'/alias)
        if files != entry['files']:
            raise ValueError('review output changed before follow-up preparation')
        digest = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
        candidates.append((digest, alias, files))
    if len(candidates) != 3:
        raise ValueError('expected three planner outputs')
    # This rule is declared before unmasking and does not select by quality or speed.
    digest, alias, files = min(candidates)
    task = root/'harness/tasks/planner-progress'
    destination = task/'fixture'
    if not (task/'PROMPT.md').is_file() or not (task/'acceptance.mjs').is_file():
        raise ValueError('freeze the new harness before materializing the fixture')
    destination.mkdir(mode=0o700)
    for name, expected in files.items():
        content = (review/'outputs'/alias/name).read_bytes()
        if hashlib.sha256(content).hexdigest() != expected:
            raise ValueError('follow-up input changed while copying')
        target = destination/(name + '.template' if '.test.' in Path(name).name else name)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    private_json(root/'followup-fixture.json', {
        'selection': 'minimum SHA-256 of the three complete file manifests',
        'maskedAlias': alias, 'workspaceHash': digest, 'files': files,
        'originalReview': str(review),
        'promptHash': hashlib.sha256((task/'PROMPT.md').read_bytes()).hexdigest(),
        'acceptanceHash': hashlib.sha256((task/'acceptance.mjs').read_bytes()).hexdigest(),
        'originalAcceptanceHash': hashlib.sha256((task.parent/'planner/acceptance.mjs').read_bytes()).hexdigest(),
    })
    print(json.dumps({'fixture': str(destination), 'maskedAlias': alias, 'sha256': digest}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('review', type=Path)
    args = parser.parse_args()
    freeze(args.root.resolve(), args.review.resolve())
