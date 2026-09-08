"""Resolve frozen tasks and source variants without changing the production checkout."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil

from prepare import private_json

HERE = Path(__file__).resolve().parent
TASKS = ('ledger', 'cache', 'planner', 'planner-progress', 'durable', 'config-edit', 'file-server')
VARIANTS = ('baseline', 'grouped', 'contract', 'work-state', 'http')
ORIGINAL = '- Batch independent tool calls in one turn; the results come back together.'
GROUPED = ORIGINAL + '''
- Once reads establish the changes needed, group related, non-overlapping edits
  in one response. Inspect results before making dependent changes. During review,
  collect the fixes you can already identify and apply them together, then run
  the relevant verification. Preserve all necessary tests and failure checks.'''
CONTRACT = '''- Before changing existing behavior, identify its callers, tests and documented
  contract. Preserve supported inputs and error behavior unless the user asks
  to change them. For stateful work, check ownership, cancellation and recovery
  across module boundaries, not just the happy path.
- Derive verification from the requirements and preserved behavior. A failing
  test is evidence to investigate; do not weaken it to match your implementation.'''


def task_directory(name):
    if name not in TASKS:
        raise ValueError('unknown task')
    return HERE if name == 'ledger' else HERE/'tasks'/name


def source_variant(root, name):
    if name not in VARIANTS:
        raise ValueError('unknown source variant')
    base = root if name == 'baseline' else root/'variants'/name
    snapshot = json.loads((base/'snapshot.json').read_text())
    return base/'jecode', snapshot


def freeze_grouped(root):
    freeze_instruction(root, 'grouped', ORIGINAL, GROUPED)


def freeze_contract(root):
    freeze_instruction(root, 'contract', ORIGINAL, CONTRACT+'\n'+ORIGINAL)


def freeze_instruction(root, variant, original_line, instruction):
    source, original = source_variant(root, 'baseline')
    prompt = (source/'src/prompt.ts').read_text()
    new_line = next(line for line in instruction.splitlines() if line != original_line)
    if json.dumps(new_line) in prompt:
        raise ValueError(f'baseline already contains the {variant} instruction')
    destination = root/'variants'/variant/'jecode'
    destination.mkdir(parents=True, mode=0o700)
    files = dict(original['files'])
    for name, expected in files.items():
        content = (source/name).read_bytes()
        if hashlib.sha256(content).hexdigest() != expected:
            raise ValueError(f'baseline snapshot changed: {name}')
        if name == 'src/prompt.ts':
            text = content.decode()
            literal = json.dumps(original_line)
            if text.count(literal) != 1:
                raise ValueError('planning instruction does not match the frozen baseline')
            replacement = ',\n    '.join(json.dumps(line) for line in instruction.splitlines())
            content = text.replace(literal, replacement).encode()
        target = destination/name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
        files[name] = hashlib.sha256(content).hexdigest()
    # Preserve npm's relative .bin symlinks; dereferencing them breaks ESM imports.
    shutil.copytree(source/'node_modules', destination/'node_modules', symlinks=True)
    private_json(destination.parent/'snapshot.json', {
        'baseCommit': original['baseCommit'], 'files': files,
        'sha256': hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
        'baselineHash': original['sha256'], 'changedFiles': ['src/prompt.ts'],
        'instruction': instruction,
    })


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--variant', choices=('grouped','contract'), default='grouped')
    args = parser.parse_args()
    (freeze_grouped if args.variant == 'grouped' else freeze_contract)(args.root.resolve())
