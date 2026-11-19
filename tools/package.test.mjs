import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {fileURLToPath} from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const scratch = path.join(root, '.tmp');
const windows = process.platform === 'win32';
const publicDocs = {
  'PUBLIC.md': '# Public guide\n\n[Details](usage/DETAILS.md)\n',
  'usage/DETAILS.md': '# Details\n\n[Public guide](../PUBLIC.md)\n',
};

function fixture(t) {
  fs.mkdirSync(scratch, {recursive: true});
  assert.equal(path.dirname(fs.realpathSync(scratch)), fs.realpathSync(root));
  const directory = fs.mkdtempSync(path.join(scratch, 'package-test-'));
  t.after(() => {
    assert.equal(path.dirname(fs.realpathSync(directory)), fs.realpathSync(scratch));
    fs.rmSync(directory, {recursive: true, force: true});
  });
  const bin = path.join(directory, 'bin');
  fs.mkdirSync(bin);
  if (windows) {
    fs.writeFileSync(path.join(bin, 'cargo.cmd'), '@echo off\r\nexit /b 0\r\n');
    fs.writeFileSync(path.join(bin, 'rustc.cmd'), '@echo off\r\necho host: x86_64-pc-windows-gnullvm\r\n');
  } else {
    for (const [name, output] of [['cargo', ''], ['rustc', 'host: x86_64-unknown-linux-gnu']]) {
      fs.writeFileSync(path.join(bin, name), `#!/bin/sh\nprintf '%s\\n' '${output}'\n`, {mode: 0o755});
    }
  }
  const script = windows ? 'PACKAGE.ps1' : 'PACKAGE.sh';
  fs.copyFileSync(path.join(root, script), path.join(directory, script));
  for (const name of [windows ? 'INSTALL.ps1' : 'INSTALL.sh', 'LICENSE', 'README.md',
    'CHANGELOG.md', 'SECURITY.md', 'CONTRIBUTING.md', 'CODE_OF_CONDUCT.md']) {
    fs.writeFileSync(path.join(directory, name), `fixture ${name}\n`);
  }
  const docs = path.join(directory, 'docs');
  fs.mkdirSync(path.join(docs, 'usage'), {recursive: true});
  for (const [name, content] of Object.entries(publicDocs)) {
    fs.writeFileSync(path.join(docs, name), content);
  }
  for (const name of ['BENCHMARK_LOCAL_NOTE.md', 'CODEX_HARNESS_STUDY.md', 'HARNESS_NOTES.md', 'Harness_Draft.md',
    'AGENTS.md', 'CLAUDE.md']) {
    fs.writeFileSync(path.join(docs, name), `fixture ${name}\n`);
  }
  const manifest = path.join(directory, 'tools/package-docs.txt');
  fs.mkdirSync(path.dirname(manifest));
  fs.writeFileSync(manifest, `${Object.keys(publicDocs).join('\n')}\n`);
  const outside = path.join(directory, 'outside');
  fs.mkdirSync(outside);
  fs.writeFileSync(path.join(outside, 'LINKED.md'), 'fixture linked\n');
  fs.symlinkSync(outside, path.join(directory, 'docs/linked'), windows ? 'junction' : 'dir');
  const release = path.join(directory, 'target/release');
  fs.mkdirSync(release, {recursive: true});
  fs.writeFileSync(path.join(release, windows ? 'jecode.exe' : 'jecode'), 'fixture binary');
  const platform = windows ? 'windows' : process.platform === 'darwin' ? 'macos' : 'linux';
  return {directory, outside, bin, script, manifest, packageRoot: path.join(directory, `target/package/jecode-${platform}-x86_64`)};
}

function run({directory, outside, bin, script}) {
  const command = windows ? 'powershell.exe' : 'sh';
  const args = windows ? ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(directory, script)] :
    [path.join(directory, script)];
  // Windows names are case-insensitive, but Node de-duplicates object keys.
  const env = Object.fromEntries(Object.entries(process.env).map(
    ([name, value]) => [windows ? name.toUpperCase() : name, value]));
  env.CARGO_TARGET_DIR = '';
  env.PATH = `${bin}${path.delimiter}${env.PATH ?? ''}`;
  if (windows) {
    env.PSEXECUTIONPOLICYPREFERENCE = 'Restricted';
    // This child uses Windows PowerShell even when its parent uses pwsh.
    env.PSMODULEPATH = path.join(env.SYSTEMROOT, 'System32/WindowsPowerShell/v1.0/Modules');
  }
  return spawnSync(command, args, {
    cwd: outside, encoding: 'utf8', timeout: 30_000,
    env,
  });
}

test('standalone packaging copies the listed guides and refuses an existing destination', t => {
  const item = fixture(t);
  const first = run(item);
  assert.equal(first.status, 0, first.stderr || first.error?.message);
  assert.ok(fs.existsSync(path.join(item.packageRoot, 'CODE_OF_CONDUCT.md')));
  const docs = path.join(item.packageRoot, 'docs');
  assert.deepEqual(fs.readdirSync(docs).sort(), ['PUBLIC.md', 'usage']);
  assert.deepEqual(fs.readdirSync(path.join(docs, 'usage')), ['DETAILS.md']);
  for (const [name, expected] of Object.entries(publicDocs)) {
    const file = path.join(docs, name);
    const content = fs.readFileSync(file, 'utf8');
    assert.equal(content, expected);
    for (const [, href] of content.matchAll(/\]\(([^)]+)\)/g)) {
      assert.ok(fs.statSync(path.resolve(path.dirname(file), href)).isFile(), `${name}: ${href}`);
    }
  }

  const stale = path.join(docs, 'STALE.md');
  fs.writeFileSync(stale, 'preserve existing package contents');
  const second = run(item);
  assert.notEqual(second.status, 0, 'packaging reused an existing destination');
  assert.match(`${second.stdout}\n${second.stderr}`, /package destination already exists/i);
  assert.equal(fs.readFileSync(stale, 'utf8'), 'preserve existing package contents');
});

test('standalone packaging rejects unavailable or linked documentation before creating a package', t => {
  const item = fixture(t);
  const allowed = fs.readFileSync(item.manifest, 'utf8');
  for (const [entry, error] of [
    ['../outside/LINKED.md', /invalid package documentation path/i],
    ['MISSING.md', /missing package documentation file/i],
    ['linked/LINKED.md', /linked package documentation path/i],
  ]) {
    fs.writeFileSync(item.manifest, `${allowed}${entry}\n`);
    const result = run(item);
    assert.notEqual(result.status, 0, entry);
    assert.match(result.stderr, error);
    assert.ok(!fs.existsSync(item.packageRoot));
  }
});
