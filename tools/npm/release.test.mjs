import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {root, targets, version, revision, sourceClean, digest, json} from './common.mjs';
import {stage, collect, pack} from './release.mjs';
import {unpack} from './archive.mjs';
import {checkVersions} from './linux-abi.mjs';

function fixture(t) {
  const scratchRoot = path.join(root, '.tmp');
  fs.mkdirSync(scratchRoot, {recursive: true});
  const directory = fs.mkdtempSync(path.join(scratchRoot, 'npm-release-test-'));
  t.after(() => {
    assert.equal(path.dirname(fs.realpathSync(directory)), fs.realpathSync(scratchRoot));
    fs.rmSync(directory, {recursive: true, force: true});
  });
  return directory;
}

function artifact(directory, id) {
  const folder = path.join(directory, id), bytes = Buffer.from(`owned test payload ${id}`);
  fs.mkdirSync(folder);
  fs.writeFileSync(path.join(folder, targets[id].binary), bytes);
  const manifest = {
    format: 1, version: version(), revision: revision(), sourceClean: true, target: id,
    binary: targets[id].binary, bytes: bytes.length, sha256: digest(bytes),
    ...(targets[id].glibcMinimum ? {glibcMinimum: targets[id].glibcMinimum} : {}),
  };
  json(path.join(folder, 'manifest.json'), manifest);
  return {folder, manifest, bytes};
}

test('release assembly requires every supported native artifact', t => {
  const directory = fixture(t);
  artifact(directory, 'win32-x64');
  assert.throws(() => collect(directory), /Missing native artifact: linux-x64/);
  assert.deepEqual(collect(directory, {local: true}).payloads.map(item => item.id), ['win32-x64']);
});

test('Linux ABI checks include version needs beyond the documented runtime baseline', () => {
  checkVersions('Name: GLIBC_2.2.5 Flags: none\nName: GLIBC_2.39 Flags: none', '2.39', 'fixture');
  assert.throws(() => checkVersions('Name: GLIBC_2.40 Flags: none', '2.39', 'fixture'), /requires glibc 2.40/);
  assert.throws(() => checkVersions('Name: GLIBC_PRIVATE', '2.39', 'fixture'), /public glibc ABI/);
});

test('release assembly rejects changed bytes and a different source revision or version', t => {
  const directory = fixture(t), item = artifact(directory, 'linux-x64');
  const file = path.join(item.folder, 'manifest.json');
  fs.appendFileSync(path.join(item.folder, targets['linux-x64'].binary), 'changed');
  assert.throws(() => collect(directory, {local: true}), /checksum mismatch/);
  fs.writeFileSync(path.join(item.folder, targets['linux-x64'].binary), item.bytes);
  for (const change of [{revision: '0'.repeat(40)}, {version: '99.0.0'}, {target: 'win32-x64'}, {binary: '../outside'}]) {
    fs.writeFileSync(file, JSON.stringify({...item.manifest, ...change}));
    assert.throws(() => collect(directory, {local: true}), /mismatch/);
  }
});

test('artifacts from modified source are limited to private local packages', t => {
  const directory = fixture(t);
  for (const id of Object.keys(targets)) artifact(directory, id);
  const file = path.join(directory, 'linux-x64/manifest.json');
  const manifest = JSON.parse(fs.readFileSync(file));
  fs.writeFileSync(file, JSON.stringify({...manifest, sourceClean: false}));
  assert.throws(() => collect(directory), /clean checkout/);
  assert.equal(collect(directory, {local: true}).payloads.length, 3);
});

test('staging rejects an executable that is not the native Jecode release', async t => {
  const directory = fixture(t);
  await assert.rejects(stage(process.execPath, path.join(directory, 'output')), /Expected the native/);
  assert.ok(!fs.existsSync(path.join(directory, 'output')));
});

test('a local package is private and contains only the selected payload and public npm files', async t => {
  const directory = fixture(t), artifacts = path.join(directory, 'artifacts');
  fs.mkdirSync(artifacts);
  const item = artifact(artifacts, 'win32-x64');
  fs.writeFileSync(path.join(item.folder, 'AGENTS.md'), 'private instructions must stay out');
  fs.writeFileSync(path.join(item.folder, 'secret.key'), 'a fixture, not a credential');
  const output = path.join(directory, 'output');
  const tarball = await pack(artifacts, output, {local: true});
  const files = unpack(fs.readFileSync(tarball));
  const manifest = JSON.parse(files.get('package/package.json'));
  assert.equal(manifest.name, 'jecode');
  assert.equal(manifest.version, version());
  assert.equal(manifest.private, true);
  assert.equal(manifest.scripts, undefined);
  assert.equal(manifest.dependencies, undefined);
  assert.equal(manifest.optionalDependencies, undefined);
  assert.equal(manifest.libc, undefined, 'a package-wide libc constraint rejects Windows and macOS');
  assert.ok(!files.get('package/bin/jecode.cjs').includes(Buffer.from('\r\n')));
  assert.deepEqual([...files.keys()].sort(), [
    'package/LICENSE', 'package/README.md', 'package/bin/jecode.cjs',
    'package/bin/targets.cjs', 'package/native/manifest.json',
    'package/native/win32-x64/jecode.exe', 'package/package.json',
  ].sort());
  assert.equal(digest(files.get('package/native/win32-x64/jecode.exe')), item.manifest.sha256);
  await assert.rejects(pack(artifacts, output, {local: true}), /EEXIST/);
});

test('a complete package uses one Cargo version and records all three verified artifacts', async t => {
  const directory = fixture(t), artifacts = path.join(directory, 'artifacts');
  fs.mkdirSync(artifacts);
  for (const id of Object.keys(targets)) artifact(artifacts, id);
  if (!sourceClean()) {
    await assert.rejects(pack(artifacts, path.join(directory, 'output')), /clean checkout/);
    return;
  }
  const tarball = await pack(artifacts, path.join(directory, 'output'));
  const files = unpack(fs.readFileSync(tarball));
  const manifest = JSON.parse(files.get('package/package.json'));
  assert.equal(manifest.private, undefined);
  assert.equal(manifest.version, version());
  const receipt = JSON.parse(files.get('package/native/manifest.json'));
  assert.equal(receipt.revision, revision());
  assert.deepEqual(receipt.artifacts.map(item => item.target), Object.keys(targets));
});
