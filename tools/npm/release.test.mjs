import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import {root, targets, version, revision, sourceClean, digest, json, command, exceeds} from './common.mjs';
import {stage, collect, pack} from './release.mjs';
import {unpack} from './archive.mjs';
import {checkVersions} from './linux-abi.mjs';
import {auditMacos, machOMinimum} from './macos-abi.mjs';
import {auditWindows, checkImage, peImage, pInvokes, reviewedPInvokes} from './windows-abi.mjs';

function machO(minimum, {command = 0x32, platform = 1} = {}) {
  const bytes = Buffer.alloc(56);
  bytes.writeUInt32LE(0xfeedfacf, 0);
  bytes.writeUInt32LE(1, 16);
  bytes.writeUInt32LE(command, 32);
  bytes.writeUInt32LE(24, 36);
  const [major, minor = 0] = minimum.split('.').map(Number), packed = (major << 16) | (minor << 8);
  bytes.writeUInt32LE(platform, 40);
  if (command === 0x32) bytes.writeUInt32LE(packed, 44);
  else bytes.writeUInt32LE(packed, 40);
  return bytes;
}

function peFile({subsystem = [6, 0], libraries = ['KERNEL32.dll']} = {}) {
  const raw = 0x200, address = 0x1000, bytes = Buffer.alloc(0x400);
  bytes.write('MZ', 0, 'latin1');
  bytes.writeUInt32LE(0x40, 0x3c);
  bytes.write('PE\0\0', 0x40, 'latin1');
  bytes.writeUInt16LE(1, 0x46);
  bytes.writeUInt16LE(240, 0x54);
  const optional = 0x58;
  bytes.writeUInt16LE(0x20b, optional);
  bytes.writeUInt16LE(subsystem[0], optional + 48);
  bytes.writeUInt16LE(subsystem[1], optional + 50);
  bytes.writeUInt32LE(address, optional + 112 + 8);
  const section = optional + 240;
  bytes.writeUInt32LE(0x200, section + 8);
  bytes.writeUInt32LE(address, section + 12);
  bytes.writeUInt32LE(0x200, section + 16);
  bytes.writeUInt32LE(raw, section + 20);
  let name = 20 * (libraries.length + 1);
  libraries.forEach((library, index) => {
    bytes.writeUInt32LE(address + name, raw + index * 20 + 12);
    bytes.write(library, raw + name, 'latin1');
    name += library.length + 1;
  });
  return bytes;
}

function releaseTree(directory, binary, helpers, stale = {}) {
  const main = path.join(directory, binary), output = path.join(directory, 'build/jecode-0123/out');
  fs.mkdirSync(output, {recursive: true});
  const embedded = [];
  for (const [name, bytes] of Object.entries(helpers)) {
    fs.writeFileSync(path.join(output, name), bytes);
    embedded.push(bytes);
  }
  for (const [name, bytes] of Object.entries(stale)) fs.writeFileSync(path.join(output, name), bytes);
  return {main, write: first => {
    const bytes = Buffer.concat([first, ...embedded]);
    fs.writeFileSync(main, bytes);
    return bytes;
  }};
}

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
    ...Object.fromEntries(['glibcMinimum', 'macosMinimum', 'windowsBuildMinimum']
      .filter(floor => targets[id][floor]).map(floor => [floor, targets[id][floor]])),
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
  assert.throws(() => checkVersions('Name: GLIBC_2.17 Flags: none\nName: GLIBC_ABI_DT_RELR Flags: none', '2.39', 'fixture'), /public glibc ABI/);
});

test('OS floors compare numerically, not as text', () => {
  assert.equal(exceeds('2.9', '2.28'), false);
  assert.equal(exceeds('2.28', '2.9'), true);
  assert.equal(exceeds('9.0', '11.0'), false);
  assert.equal(exceeds('11.0.0', '11.0'), false);
  assert.equal(exceeds('11.0.1', '11.0'), true);
});

test('macOS deployment targets are read from either load command and fail closed', () => {
  assert.equal(machOMinimum(machO('11.0'), 'fixture'), '11.0.0');
  assert.equal(machOMinimum(machO('10.12', {command: 0x24}), 'fixture'), '10.12.0');
  assert.throws(() => machOMinimum(machO('11.0', {command: 0x19}), 'fixture'), /records no macOS deployment target/);
  assert.throws(() => machOMinimum(machO('11.0', {platform: 2}), 'fixture'), /Apple platform 2/);
  assert.throws(() => machOMinimum(Buffer.from('#!/bin/sh'), 'fixture'), /not a 64-bit Mach-O/);
});

test('the macOS audit fails when the executable or an embedded helper exceeds the floor', t => {
  const directory = fixture(t);
  const names = ['jecode-shell', 'jecode-http', 'jecode-openai_crypto', 'jecode-process_unix', 'jecode-pty_unix', 'jecode-terminal_unix'];
  const helpers = Object.fromEntries(names.map(name => [name, machO('11.0')]));
  // A stale helper from an earlier build must not fail a clean release.
  const tree = releaseTree(directory, 'jecode', helpers, {'jecode-stale': machO('26.0')});
  assert.deepEqual(auditMacos(tree.main, tree.write(machO('11.0')), '11.0').sort(), [...names, 'jecode'].sort());
  assert.throws(() => auditMacos(tree.main, tree.write(machO('15.0')), '11.0'), /jecode requires macOS 15\.0\.0, above the supported minimum 11\.0/);

  const raised = path.join(directory, 'raised');
  fs.mkdirSync(raised);
  const high = releaseTree(raised, 'jecode', {...helpers, 'jecode-http': machO('26.0', {platform: 1})});
  assert.throws(() => auditMacos(high.main, high.write(machO('11.0')), '11.0'), /jecode-http requires macOS 26\.0\.0/);

  const missing = path.join(directory, 'missing');
  fs.mkdirSync(missing);
  const {'jecode-pty_unix': _, ...partial} = helpers;
  const absent = releaseTree(missing, 'jecode', partial);
  assert.throws(() => auditMacos(absent.main, absent.write(machO('11.0')), '11.0'), /Cannot audit embedded macOS helper jecode-pty_unix/);
});

test('PE files expose their subsystem and imports, and unreviewed ones fail', () => {
  const image = peImage(peFile({libraries: ['KERNEL32.dll', 'api-ms-win-crt-heap-l1-1-0.dll']}), 'fixture');
  assert.deepEqual(image, {subsystem: '6.0', libraries: ['KERNEL32.dll', 'api-ms-win-crt-heap-l1-1-0.dll']});
  checkImage(image, 17763, 'fixture');
  assert.throws(() => checkImage(peImage(peFile({subsystem: [10, 1]}), 'fixture'), 17763, 'fixture'), /subsystem 10\.1/);
  assert.throws(() => checkImage(peImage(peFile({libraries: ['api-ms-win-core-path-l1-1-0.dll']}), 'fixture'), 17763, 'fixture'),
    /imports api-ms-win-core-path-l1-1-0\.dll, not reviewed against Windows build 17763/);
  assert.throws(() => peImage(Buffer.from('MZ not a real file'), 'fixture'), /not a PE file/);
});

test('the Windows audit covers the executable and every embedded helper', t => {
  const directory = fixture(t);
  const names = ['jecode-shell.exe', 'jecode-supervisor.exe', 'jecode-pty.exe', 'jecode-terminal.exe',
    'jecode-openai_crypto.exe', 'jecode-http.exe'];
  const helpers = Object.fromEntries(names.map(name => [name, peFile({libraries: ['mscoree.dll']})]));
  const tree = releaseTree(directory, 'jecode.exe', helpers);
  assert.deepEqual(auditWindows(tree.main, tree.write(peFile()), 17763).sort(), [...names, 'jecode.exe'].sort());

  const raised = path.join(directory, 'raised');
  fs.mkdirSync(raised);
  const high = releaseTree(raised, 'jecode.exe', {...helpers, 'jecode-http.exe': peFile({libraries: ['newsystem.dll']})});
  assert.throws(() => auditWindows(high.main, high.write(peFile()), 17763), /jecode-http\.exe imports newsystem\.dll/);
});

test('every Windows P/Invoke an adapter declares was reviewed against the floor', () => {
  const declared = new Set();
  const visit = folder => {
    for (const entry of fs.readdirSync(folder, {withFileTypes: true})) {
      const file = path.join(folder, entry.name);
      if (entry.isDirectory()) visit(file);
      // The terminal fixtures under src/tui/terminal/windows never ship.
      else if (entry.name.endsWith('.cs') && !entry.name.startsWith('native_')) {
        for (const item of pInvokes(fs.readFileSync(file, 'utf8'))) declared.add(item);
      }
    }
  };
  visit(path.join(root, 'src'));
  const reviewed = Object.entries(reviewedPInvokes).flatMap(([library, names]) => names.map(name => `${library}!${name}`));
  assert.deepEqual([...declared].sort(), reviewed.sort());
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
  assert.equal(collect(directory, {local: true}).payloads.length, 4);
});

test('release assembly rejects an artifact staged against a different OS floor', t => {
  for (const [id, change] of [
    ['linux-x64', {glibcMinimum: '2.39'}], ['darwin-arm64', {macosMinimum: '15.0'}],
    ['darwin-x64', {macosMinimum: undefined}], ['win32-x64', {windowsBuildMinimum: 26100}],
  ]) {
    const directory = fixture(t), item = artifact(directory, id);
    fs.writeFileSync(path.join(item.folder, 'manifest.json'), JSON.stringify({...item.manifest, ...change}));
    assert.throws(() => collect(directory, {local: true}), new RegExp(`OS floor mismatch: ${id}`));
  }
});

test('staging rejects an executable that is not the native Jecode release', async t => {
  const directory = fixture(t);
  await assert.rejects(stage(process.execPath, path.join(directory, 'output')), /Expected the native/);
  assert.ok(!fs.existsSync(path.join(directory, 'output')));
});

test('commands without input tolerate a child exiting immediately', async () => {
  for (const input of [undefined, '', Buffer.alloc(0)]) {
    for (let index = 0; index < 10; index++) {
      const result = await command(process.execPath, ['--version'], {input, timeout: 5000});
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout.trim(), process.version);
    }
  }
});

test('commands deliver input and collect the child output and status', async () => {
  const input = 'first line\nsecond λ line\n';
  const result = await command(process.execPath, ['-e',
    'process.stdin.pipe(process.stdout); process.exitCode = 7;',
  ], {input, timeout: 5000});
  assert.equal(result.stdout, input);
  assert.equal(result.status, 7);
});

test('commands reject input delivery failures', async () => {
  // Unix can close fd 0 while staying alive; Windows closes its inherited
  // pipe when the process exits. Neither child consumes the buffered input.
  const script = process.platform === 'win32' ? 'process.exit(0);' :
    "require('node:fs').closeSync(0); setInterval(() => {}, 1000);";
  await assert.rejects(command(process.execPath, ['-e', script],
    {input: Buffer.alloc(8 * 1024 * 1024), timeout: 5000}),
  error => ['EPIPE', 'ECONNRESET', 'EOF'].includes(error.code));
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
  assert.equal(manifest.name, '@giovannijecha/jecode');
  assert.equal(path.basename(tarball), `giovannijecha-jecode-${version()}.tgz`);
  assert.deepEqual(manifest.bin, {jecode: 'bin/jecode.cjs'});
  assert.equal(manifest.publishConfig.access, 'public');
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

test('a complete package uses one Cargo version and records every verified artifact', async t => {
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
  assert.equal(manifest.name, '@giovannijecha/jecode');
  assert.equal(path.basename(tarball), `giovannijecha-jecode-${version()}.tgz`);
  const packageReceipt = JSON.parse(fs.readFileSync(path.join(path.dirname(tarball), 'manifest.json')));
  assert.equal(packageReceipt.filename, path.basename(tarball));
  assert.equal(packageReceipt.sha256, digest(fs.readFileSync(tarball)));
  assert.equal(manifest.version, version());
  const receipt = JSON.parse(files.get('package/native/manifest.json'));
  assert.equal(receipt.revision, revision());
  assert.deepEqual(receipt.artifacts.map(item => item.target), Object.keys(targets));
});
