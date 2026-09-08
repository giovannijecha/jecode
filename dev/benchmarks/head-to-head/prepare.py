"""Prepare an isolated Linux pilot using pinned tools and a source snapshot."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def digest(data):
    return hashlib.sha256(data).hexdigest()


def private_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")
    path.chmod(0o600)


def auth_inventory(homes):
    result = []
    for base in homes:
        for product, name in (('.codex', 'auth.json'), ('.jecode', 'accounts.json')):
            file = base / product / name
            row = {'path': str(file), 'exists': file.is_file()}
            if file.is_file():
                try:
                    data = json.loads(file.read_text())
                    if product == '.codex':
                        tokens = data.get('tokens') or {}
                        row['accountAuth'] = bool(tokens.get('access_token'))
                    else:
                        account = data.get('accounts', {}).get('openai-codex') or {}
                        row['accountAuth'] = bool(account.get('accessToken'))
                        row['expiresAt'] = account.get('expiresAt')
                except (OSError, ValueError, AttributeError):
                    row['readable'] = False
            result.append(row)
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inspect-auth', action='store_true')
    parser.add_argument('--auth-home', type=Path, action='append')
    parser.add_argument('--root', type=Path)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--node', type=Path)
    parser.add_argument('--codex-version', default='0.153.4')
    parser.add_argument('--codex-auth', type=Path)
    parser.add_argument('--jecode-account', type=Path)
    parser.add_argument('--codex-command-network', action='store_true',
                        help='allow command networking in the isolated Codex workspace profile')
    args = parser.parse_args()
    if args.inspect_auth:
        auth_inventory(args.auth_home or [Path.home()])
        return
    if not all((args.root, args.source, args.node)):
        parser.error('--root, --source and --node are required')
    root, source, node = args.root.resolve(), args.source.resolve(), args.node.resolve()
    if str(root).startswith('/mnt/') or not root.is_dir() or root.stat().st_mode & 0o077:
        parser.error('root must be an existing private directory on the Linux filesystem')
    destination = root / 'jecode'
    env = dict(os.environ, PATH=f'{node.parent}:/usr/local/bin:/usr/bin:/bin')
    installed_codex = json.loads((root/'codex/node_modules/@openai/codex/package.json').read_text())['version']
    if installed_codex != args.codex_version:
        parser.error('installed Codex version does not match the requested pin')
    codex_version = subprocess.check_output([str(root/'codex/node_modules/.bin/codex'),'--version'],env=env,text=True).strip()
    destination.mkdir(mode=0o700)
    names = subprocess.check_output(
        ['git', '-c', f'safe.directory={source}', 'ls-files', '-z',
         '--cached', '--others', '--exclude-standard'], cwd=source).split(b'\0')
    manifest = {}
    for raw in sorted(set(names)):
        if not raw:
            continue
        relative = Path(os.fsdecode(raw))
        if relative.name == 'AGENTS.md':
            continue
        original = source / relative
        if not original.exists():
            continue
        if (relative.is_absolute() or '..' in relative.parts or original.is_symlink()
                or not original.resolve().is_relative_to(source)):
            raise ValueError('snapshot only accepts regular repository files')
        content = original.read_bytes()
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
        manifest[relative.as_posix()] = digest(content)
    base = subprocess.check_output(['git', '-c', f'safe.directory={source}',
                                    'rev-parse', 'HEAD'], cwd=source, text=True).strip()
    private_json(root / 'snapshot.json', {'baseCommit': base, 'files': manifest,
                 'sha256': digest(json.dumps(manifest, sort_keys=True).encode())})
    for product in ('codex-home', 'jecode-home'):
        (root / product).mkdir(mode=0o700)
    for supplied, target in ((args.codex_auth, root / 'codex-home/auth.json'),
                             (args.jecode_account, root / 'jecode-home/accounts.json')):
        if supplied is not None:
            # Local account reuse is explicit; contents never enter reports or the source snapshot.
            shutil.copyfile(supplied, target)
            target.chmod(0o600)
    if args.codex_auth and args.jecode_account:
        codex_account = json.loads(args.codex_auth.read_text()).get('tokens', {}).get('account_id')
        jecode_account = json.loads(args.jecode_account.read_text()).get('accounts', {}).get('openai-codex', {}).get('accountId')
        private_json(root / 'auth-status.json', {'sameAccount': bool(codex_account and codex_account == jecode_account)})
    private_json(root / 'jecode-home/settings.json', {
        'provider': 'openai-codex', 'models': {'openai-codex': 'gpt-6-astra'},
        'effort': 'high', 'reducedMotion': True,
    })
    private_json(root / 'environment.json', {
        'node': str(node), 'nodeVersion': subprocess.check_output([str(node), '--version'], text=True).strip(),
        'sourceSnapshot': str(destination), 'platform': os.uname().release,
        'codexVersion': installed_codex, 'codexReportedVersion': codex_version,
        'codexLockHash': digest((root/'codex/package-lock.json').read_bytes()),
        'model': 'gpt-6-astra', 'effort': 'high',
        'codexCommandNetwork': args.codex_command_network,
    })
    with (root / 'npm-ci.log').open('w') as log:
        subprocess.run([str(node.parent / 'npm'), 'ci', '--ignore-scripts', '--no-audit', '--no-fund'],
                       cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    print(json.dumps({'root': str(root), 'baseCommit': base,
                      'snapshotFiles': len(manifest), 'snapshotHash': digest(json.dumps(manifest, sort_keys=True).encode())}))


if __name__ == '__main__':
    main()
