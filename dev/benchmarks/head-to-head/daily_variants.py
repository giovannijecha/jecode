"""Freeze isolated daily-driver candidates; never mutate production sources."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
from prepare import private_json
from scenarios import source_variant


def replace_once(content, before, after):
    if content.count(before) != 1:
        raise ValueError('variant integration point is missing or ambiguous')
    return content.replace(before, after)


def freeze(root, variant):
    if variant not in ('work-state', 'http'):
        raise ValueError('unknown daily-driver variant')
    source, original = source_variant(root, 'baseline')
    files = {}
    changed = []
    content_by_name = {}
    for name, expected in original['files'].items():
        content = (source/name).read_bytes()
        if hashlib.sha256(content).hexdigest() != expected:
            raise ValueError(f'baseline changed: {name}')
        content_by_name[name] = content
    if variant == 'http':
        name = 'src/providers/responses-session.ts'
        content_by_name[name] = replace_once(content_by_name[name], b'#httpOnly = false;', b'#httpOnly = true;')
        changed.append(name)
    else:
        module = content_by_name['dev/experiments/work-state.ts'].replace(b'../../src/', b'./')
        content_by_name['src/work-state.ts'] = module
        changed.append('src/work-state.ts')
        for name, before, after in (
            ('src/tools/index.ts', b'import { listDir, readFile }',
             b'import { workStateTool } from "../work-state.ts";\nimport { listDir, readFile }'),
            ('src/tools/index.ts', b'editFile, writeFile, runCommand];',
             b'editFile, writeFile, runCommand, workStateTool([])];'),
            ('src/controller.ts', b'import { settlePool }',
             b'import { workStateTool } from "./work-state.ts";\nimport { settlePool }'),
            ('src/controller.ts', b'  const specs = toolSpecs(options.tools);',
             b'  options = { ...options, tools: options.tools.map(tool =>\n'
             b'    tool.name === "work_state" ? workStateTool(history) : tool) };\n'
             b'  const specs = toolSpecs(options.tools);'),
            ('test/permissions.test.ts', b'{ name: "run_command", mode: "ask" },',
             b'{ name: "run_command", mode: "ask" },\n      { name: "work_state", mode: "allow" },'),
            ('test/permission-command.test.ts', b'    "run_command",',
             b'    "run_command",\n    "work_state",'),
            ('test/permission-command.test.ts', b'assert.equal(screen.pickers[0]?.visible, 7);',
             b'assert.equal(screen.pickers[0]?.visible, 8);'),
            ('test/search-tools.test.ts', b'["run_command", "exclusive"],',
             b'["run_command", "exclusive"],\n    ["work_state", "exclusive"],'),
        ):
            content_by_name[name] = replace_once(content_by_name[name], before, after)
            if name not in changed:
                changed.append(name)
    destination = root/'variants'/variant/'jecode'
    destination.mkdir(parents=True, mode=0o700)
    for name, content in content_by_name.items():
        target = destination/name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
        files[name] = hashlib.sha256(content).hexdigest()
    shutil.copytree(source/'node_modules', destination/'node_modules', symlinks=True)
    private_json(destination.parent/'snapshot.json', {
        'baseCommit': original['baseCommit'], 'files': files,
        'sha256': hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
        'baselineHash': original['sha256'], 'changedFiles': changed,
        'experiment': variant,
    })


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--variant', required=True, choices=('work-state', 'http'))
    args = parser.parse_args()
    freeze(args.root.resolve(), args.variant)
