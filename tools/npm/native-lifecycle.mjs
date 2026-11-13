import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {gzipSync} from 'node:zlib';
import {command, packageName, root, targets, version} from './common.mjs';

const source = fileURLToPath(new URL('./native_fixture.rs', import.meta.url));
const host = `${process.platform}-${process.arch}`;
const target = targets[host];
const nextMajor = Number(version().split('.')[0]) + 1;
const future = `${nextMajor}.0.0`;
const wrong = `${nextMajor}.0.1`;

function sha(algorithm, bytes, encoding) {
  return createHash(algorithm).update(bytes).digest(encoding);
}

function entry(name, bytes) {
  const header = Buffer.alloc(512);
  const octal = (value, offset, width) => {
    const digits = value.toString(8);
    assert.ok(digits.length < width, `Tar field ${value} does not fit`);
    header.write(`${digits.padStart(width - 1, '0')}\0`, offset, width, 'ascii');
  };
  assert.ok(Buffer.byteLength(name) <= 100, `Tar path is too long: ${name}`);
  header.write(name, 0, 'ascii');
  octal(0o644, 100, 8);
  octal(0, 108, 8);
  octal(0, 116, 8);
  octal(bytes.length, 124, 12);
  octal(0, 136, 12);
  header.fill(32, 148, 156);
  header.write('0', 156, 'ascii');
  header.write('ustar\0', 257, 'ascii');
  header.write('00', 263, 'ascii');
  octal(header.reduce((sum, byte) => sum + byte, 0), 148, 8);
  const padding = Buffer.alloc((512 - bytes.length % 512) % 512);
  return Buffer.concat([header, bytes, padding]);
}

function packageBytes(binary, receiptVersion = future) {
  const manifest = Buffer.from(JSON.stringify({name: packageName, version: future}));
  const receipt = Buffer.from(JSON.stringify({
    version: future,
    artifacts: [{target: host, version: receiptVersion, sha256: sha('sha256', binary, 'hex')}],
  }));
  const tar = Buffer.concat([
    entry('package/package.json', manifest),
    entry('package/native/manifest.json', receipt),
    entry(`package/native/${host}/${target.binary}`, binary),
    Buffer.alloc(1024),
  ]);
  return gzipSync(tar);
}

function compileFixture(scratch, reportedVersion) {
  const output = path.join(scratch, `fixture-${reportedVersion}${process.platform === 'win32' ? '.exe' : ''}`);
  const flags = process.platform === 'win32' ? ['-Ctarget-feature=+crt-static'] : [];
  const linker = process.env.CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER;
  if (process.platform === 'win32' && linker) flags.push(`-Clinker=${linker}`);
  const result = spawnSync('rustc', ['--edition=2024', ...flags, source, '-o', output], {
    cwd: root,
    env: {...process.env, JECODE_NATIVE_FIXTURE_VERSION: reportedVersion},
    encoding: 'utf8',
    windowsHide: true,
    timeout: 120_000,
  });
  assert.equal(result.status, 0, `rustc fixture failed: ${result.error ?? result.stderr}`);
  return fs.readFileSync(output);
}

function standalone(scratch, label, builtBinary) {
  const installRoot = path.join(scratch, label);
  const bin = path.join(installRoot, 'bin');
  fs.mkdirSync(bin, {recursive: true});
  const executable = path.join(bin, target.binary);
  fs.copyFileSync(builtBinary, executable);
  if (process.platform !== 'win32') fs.chmodSync(executable, 0o755);
  fs.writeFileSync(path.join(installRoot, '.jecode-install'), 'Jecode local Rust installation\n');
  return {installRoot, bin, executable};
}

async function registry(packages) {
  let origin;
  let selected = 'valid';
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url, origin).pathname;
    if (decodeURIComponent(pathname) === `/${packageName}`) {
      const bytes = packages[selected];
      const integrity = selected === 'integrity-bad'
        ? 'sha512-' + Buffer.alloc(64, 0).toString('base64')
        : `sha512-${sha('sha512', bytes, 'base64')}`;
      response.writeHead(200, {'Content-Type': 'application/json', 'Cache-Control': 'no-store'});
      response.end(JSON.stringify({
        name: packageName,
        'dist-tags': {latest: future},
        versions: {[future]: {dist: {tarball: `${origin}/fixture.tgz`, integrity}}},
      }));
    } else if (pathname === '/fixture.tgz') {
      response.writeHead(200, {'Content-Type': 'application/octet-stream'});
      response.end(packages[selected]);
    } else {
      response.writeHead(404);
      response.end();
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  origin = `http://127.0.0.1:${server.address().port}`;
  return {
    origin,
    select(value) { selected = value; },
    close: () => new Promise((resolve, reject) => {
      server.close(error => error ? reject(error) : resolve());
      server.closeAllConnections();
    }),
  };
}

async function invoke(executable, args, env, scratch) {
  return command(executable, args, {cwd: scratch, env, timeout: 120_000});
}

async function awaitRemoved(file) {
  for (let attempt = 0; attempt < 100 && fs.existsSync(file); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(!fs.existsSync(file), `Deferred cleanup left ${file}`);
}

function userPath(env) {
  const script = '$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment"); ' +
    'if ($null -eq $key) { exit 0 }; ' +
    '$value = $key.GetValue("Path", $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); ' +
    'if ($null -eq $value) { exit 0 }; ' +
    '[Console]::Out.Write($key.GetValueKind("Path").ToString() + "`n" + ' +
    '[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($value)))';
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    env, encoding: 'utf8', windowsHide: true, timeout: 30_000,
  });
  assert.equal(result.status, 0, `Could not inspect user PATH: ${result.error ?? result.stderr}`);
  const [kind, encoded] = result.stdout.split('\n');
  return encoded === undefined ? {kind: null, value: null} : {
    kind, value: Buffer.from(encoded.trim(), 'base64').toString('utf8'),
  };
}

function pathContains(directory, raw, env) {
  const variables = new Map(Object.entries(env).map(([name, value]) => [name.toLowerCase(), value]));
  const normalize = value => value.trim().replace(/^"+|"+$/g, '').replace(/[\\/]+$/g, '').toLowerCase();
  const target = normalize(directory);
  return raw?.split(';').some(entry => {
    const expanded = entry.replace(/%([^%]+)%/g, (match, name) => variables.get(name.toLowerCase()) ?? match);
    return entry.trim() && normalize(expanded) === target;
  }) ?? false;
}

async function main(builtBinary) {
  assert.ok(target, `Unsupported native target: ${host}`);
  const absolute = path.resolve(builtBinary);
  assert.ok(fs.statSync(absolute).isFile(), `Expected a built executable: ${absolute}`);
  const original = fs.readFileSync(absolute);
  const scratchParent = fs.realpathSync(os.tmpdir());
  const scratch = fs.mkdtempSync(path.join(scratchParent, 'jecode-native-lifecycle-'));
  let service;
  try {
    const home = path.join(scratch, 'home');
    fs.mkdirSync(home);
    const homeProof = path.join(home, 'keep.txt');
    fs.writeFileSync(homeProof, 'home stays');
    const installed = standalone(scratch, 'update-root', absolute);
    const good = compileFixture(scratch, future);
    const bad = compileFixture(scratch, wrong);
    service = await registry({
      'integrity-bad': packageBytes(good),
      'wrong-target-version': packageBytes(good, wrong),
      'wrong-executable-version': packageBytes(bad),
      valid: packageBytes(good),
    });
    const env = {
      ...process.env,
      JECODE_HOME: home,
      JECODE_UPDATE_REGISTRY: service.origin,
      HOME: home,
      USERPROFILE: home,
      APPDATA: path.join(scratch, 'appdata'),
      LOCALAPPDATA: path.join(scratch, 'localappdata'),
      HTTP_PROXY: '', HTTPS_PROXY: '', ALL_PROXY: '',
      NO_PROXY: '127.0.0.1,localhost',
    };
    const installedVersion = await invoke(installed.executable, ['--version'], env, scratch);
    assert.equal(installedVersion.status, 0, installedVersion.stderr);
    assert.ok(installedVersion.stdout.startsWith(`jecode ${version()} (`), installedVersion.stdout);
    const originalDigest = sha('sha256', original, 'hex');
    const check = await invoke(installed.executable, ['update', '--check', '--json'], env, scratch);
    assert.equal(check.status, 0, check.stderr);
    const checked = JSON.parse(check.stdout);
    assert.equal(checked.update_available, true);
    assert.equal(checked.updated, false);
    assert.equal(checked.installation.managed_by, 'standalone');
    assert.equal(sha('sha256', fs.readFileSync(installed.executable), 'hex'), originalDigest);

    for (const [scenario, expected] of [
      ['integrity-bad', /sha512 integrity/],
      ['wrong-target-version', /does not match its manifest/],
      ['wrong-executable-version', /reported.*instead of Jecode/],
    ]) {
      service.select(scenario);
      const result = await invoke(installed.executable, ['update', '--json'], env, scratch);
      assert.notEqual(result.status, 0, `${scenario} unexpectedly succeeded: ${result.stdout}`);
      assert.match(result.stderr, expected, `${scenario}: ${result.stderr}`);
      assert.equal(sha('sha256', fs.readFileSync(installed.executable), 'hex'), originalDigest);
    }
    service.select('valid');
    const updated = await invoke(installed.executable, ['update', '--json'], env, scratch);
    assert.equal(updated.status, 0, updated.stderr);
    const outcome = JSON.parse(updated.stdout).result;
    assert.equal(outcome.updated, true);
    assert.equal(outcome.version, future);
    assert.equal(sha('sha256', fs.readFileSync(installed.executable), 'hex'), sha('sha256', good, 'hex'));
    for (const file of outcome.deferred) await awaitRemoved(file);
    const leftovers = fs.readdirSync(installed.bin).filter(name => name.startsWith('.jecode-previous-'));
    assert.deepEqual(leftovers, []);
    assert.equal(fs.readFileSync(homeProof, 'utf8'), 'home stays');

    const uninstallRoot = standalone(scratch, 'uninstall-root', absolute);
    const unrelated = path.join(uninstallRoot.installRoot, 'keep.txt');
    fs.writeFileSync(unrelated, 'unrelated stays');
    // The CLI only writes HKCU Environment\\Path when it contains this exact
    // disposable bin. Verify absence first, then compare the untouched value.
    const beforePath = process.platform === 'win32' ? userPath(env) : null;
    assert.ok(
      !beforePath || !pathContains(uninstallRoot.bin, beforePath.value, env),
      'Refusing uninstall verification because user PATH names the disposable bin',
    );
    const removed = await invoke(uninstallRoot.executable, ['uninstall', '--yes', '--json'], env, scratch);
    assert.equal(removed.status, 0, removed.stderr);
    if (beforePath) assert.deepEqual(userPath(env), beforePath, 'Uninstall changed the user PATH');
    const removal = JSON.parse(removed.stdout);
    assert.equal(removal.removed, true);
    if (beforePath) assert.equal(removal.path_entries_removed, 0);
    for (const file of removal.deferred) await awaitRemoved(file);
    assert.ok(!fs.existsSync(uninstallRoot.executable));
    assert.ok(!fs.existsSync(path.join(uninstallRoot.installRoot, '.jecode-install')));
    assert.equal(fs.readFileSync(unrelated, 'utf8'), 'unrelated stays');
    assert.equal(fs.readFileSync(homeProof, 'utf8'), 'home stays');
    console.log(`native lifecycle passed on ${host}: check, failure preservation, update, cleanup and uninstall`);
  } finally {
    if (service) await service.close();
    const resolvedScratch = fs.realpathSync(scratch);
    assert.equal(path.dirname(resolvedScratch), scratchParent);
    assert.ok(path.basename(resolvedScratch).startsWith('jecode-native-lifecycle-'));
    fs.rmSync(resolvedScratch, {recursive: true, force: true});
  }
}

if (process.argv.length !== 3) {
  console.error('Usage: node tools/npm/native-lifecycle.mjs <built-jecode-binary>');
  process.exitCode = 2;
} else {
  main(process.argv[2]).catch(error => {
    console.error(error.stack ?? error);
    process.exitCode = 1;
  });
}
