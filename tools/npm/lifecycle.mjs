import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {root, packageName, targets, host, digest, regularFile, npm, command, json} from './common.mjs';
import {unpack} from './archive.mjs';

const target = targets[host];

function isolatedEnvironment(scratch) {
  const home = path.join(scratch, 'home'), prefix = path.join(scratch, 'prefix with spaces');
  const data = path.join(home, 'jecode-data'), defaultData = path.join(home, '.jecode');
  for (const dir of [prefix, data, defaultData]) fs.mkdirSync(dir, {recursive: true});
  const userConfig = path.join(scratch, 'user.npmrc'), globalConfig = path.join(scratch, 'global.npmrc');
  fs.writeFileSync(userConfig, '');
  fs.writeFileSync(globalConfig, '');
  fs.writeFileSync(path.join(data, 'keep.txt'), 'preserved conversation');
  fs.writeFileSync(path.join(defaultData, 'keep.txt'), 'preserved default data');
  fs.writeFileSync(path.join(prefix, 'unrelated.txt'), 'not owned by jecode');
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
    !/^npm_config_/i.test(key) && !/^(?:NODE_AUTH_TOKEN|NPM_TOKEN|NPM_AUTH_TOKEN)$/i.test(key)));
  Object.assign(env, {
    HOME: home, USERPROFILE: home, LOCALAPPDATA: path.join(home, 'local'),
    APPDATA: path.join(home, 'roaming'), XDG_CACHE_HOME: path.join(home, 'cache'),
    XDG_CONFIG_HOME: path.join(home, 'config'), XDG_DATA_HOME: path.join(home, 'data'),
    JECODE_HOME: data, npm_config_prefix: prefix, npm_config_cache: path.join(scratch, 'cache'),
    npm_config_userconfig: userConfig, npm_config_globalconfig: globalConfig,
    npm_config_ignore_scripts: 'true', npm_config_audit: 'false', npm_config_fund: 'false',
    npm_config_update_notifier: 'false', npm_config_logs_max: '0',
    npm_config_fetch_retries: '0', npm_config_fetch_timeout: '15000',
  });
  return {env, prefix, data, defaultData};
}

function installed(prefix) {
  const directory = path.join(prefix, ...(process.platform === 'win32' ? [] : ['lib']), 'node_modules', packageName);
  return {
    directory,
    launcher: path.join(directory, 'bin/jecode.cjs'),
    binary: path.join(directory, 'native', host, target.binary),
    shim: path.join(prefix, ...(process.platform === 'win32' ? [] : ['bin']), process.platform === 'win32' ? 'jecode.cmd' : 'jecode'),
  };
}

async function runInstalled(prefix, args, options) {
  const files = installed(prefix);
  assert.ok(fs.existsSync(files.shim), 'npm must create its command shim');
  // Invoke npm's real command shim, including its Windows quoting of spaces.
  if (process.platform === 'win32') {
    const literal = value => `'${value.replaceAll("'", "''")}'`;
    const script = `& ${literal(files.shim)} ${args.map(literal).join(' ')}; exit $LASTEXITCODE`;
    return command('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], options);
  }
  return command(files.shim, args, options);
}

function preserved(state) {
  assert.equal(fs.readFileSync(path.join(state.data, 'keep.txt'), 'utf8'), 'preserved conversation');
  assert.equal(fs.readFileSync(path.join(state.defaultData, 'keep.txt'), 'utf8'), 'preserved default data');
  assert.equal(fs.readFileSync(path.join(state.prefix, 'unrelated.txt'), 'utf8'), 'not owned by jecode');
}

async function uninstall(state, options) {
  await npm(['uninstall', '--global', packageName], options);
  const files = installed(state.prefix);
  assert.ok(!fs.existsSync(files.directory), 'npm must remove its package');
  assert.ok(!fs.existsSync(files.shim), 'npm must remove its command shim');
  preserved(state);
}

async function fixture(version, scratch, template, options) {
  const directory = path.join(scratch, `fixture-${version}`);
  fs.mkdirSync(directory);
  const native = path.join(directory, 'native', host);
  fs.mkdirSync(native, {recursive: true});
  const linker = process.platform === 'win32' && process.env.CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER;
  const compiler = await command('rustc', [
    '--edition=2024', '-Dunsafe_code', '-Copt-level=1',
    ...(process.platform === 'win32' ? ['-Ctarget-feature=+crt-static'] : []),
    ...(linker ? ['-C', `linker=${linker}`] : []),
    path.join(root, 'tools/npm/fixture.rs'), '-o', path.join(native, target.binary),
  ], {env: {...process.env, JECODE_NPM_FIXTURE_VERSION: version}});
  assert.equal(compiler.status, 0, compiler.stderr);
  const manifest = {
    name: packageName, version, private: true, license: 'MIT',
    bin: {jecode: 'bin/jecode.cjs'}, files: ['bin/', 'native/'],
  };
  json(path.join(directory, 'package.json'), manifest);
  fs.mkdirSync(path.join(directory, 'bin'));
  for (const name of ['jecode.cjs', 'targets.cjs']) {
    fs.writeFileSync(path.join(directory, 'bin', name), template.get(`package/bin/${name}`), {mode: 0o755});
  }
  const packed = await npm(['pack', '--ignore-scripts', '--offline', '--json', '--pack-destination', directory, directory], options);
  const info = JSON.parse(packed.stdout)[0];
  const bytes = regularFile(path.join(directory, info.filename));
  return {manifest, bytes, binaryDigest: digest(regularFile(path.join(native, target.binary)))};
}

async function registry(fixtures) {
  let latest = '0.0.1', origin;
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, origin);
    response.setHeader('Cache-Control', 'no-store');
    if (decodeURIComponent(url.pathname) === `/${packageName}`) {
      const versions = {};
      for (const [version, item] of fixtures) {
        if (version > latest) continue;
        versions[version] = {...item.manifest, dist: {
          tarball: `${origin}/jecode-${version}.tgz`,
          shasum: createHash('sha1').update(item.bytes).digest('hex'),
          integrity: `sha512-${createHash('sha512').update(item.bytes).digest('base64')}`,
        }};
      }
      response.setHeader('Content-Type', 'application/json');
      response.end(JSON.stringify({name: packageName, 'dist-tags': {latest}, versions}));
      return;
    }
    const version = /^\/jecode-(0\.0\.[12])\.tgz$/.exec(url.pathname)?.[1];
    const item = fixtures.get(version);
    if (item) {
      response.setHeader('Content-Type', 'application/octet-stream');
      response.end(item.bytes);
    } else {
      response.writeHead(404, {'Content-Type': 'application/json'});
      response.end(JSON.stringify({error: 'Fixture route not found'}));
    }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  origin = `http://127.0.0.1:${server.address().port}`;
  return {
    origin,
    update() { latest = '0.0.2'; },
    close: () => new Promise((resolve, reject) => {
      server.close(error => error ? reject(error) : resolve());
      server.closeAllConnections();
    }),
  };
}

async function checkFixtureVersion(prefix, version, options) {
  const result = await runInstalled(prefix, ['--version'], options);
  const detail = `Fixture ${version} exit=${result.status ?? result.signal}; stderr=${result.stderr.trim() || '(empty)'}`;
  assert.equal(result.status, 0, detail);
  assert.equal(result.stdout.trim(), `jecode npm fixture ${version}`, detail);
}

export async function lifecycle(tarball) {
  if (!target) throw new Error(`Unsupported test host: ${host}`);
  const bytes = regularFile(path.resolve(tarball)), contents = unpack(bytes);
  const metadata = JSON.parse(contents.get('package/package.json'));
  assert.equal(metadata.name, packageName);
  const native = contents.get(`package/native/${host}/${target.binary}`);
  assert.ok(native, `The package does not contain ${host}`);
  const receipt = JSON.parse(contents.get('package/native/manifest.json'));
  const entry = receipt.artifacts.find(item => item.target === host);
  assert.equal(receipt.version, metadata.version);
  assert.equal(entry?.sha256, digest(native));
  assert.equal(entry?.version, metadata.version);
  fs.mkdirSync(path.join(root, '.tmp'), {recursive: true});
  const scratch = fs.mkdtempSync(path.join(root, '.tmp/npm-lifecycle-'));
  const state = isolatedEnvironment(scratch), options = {env: state.env, cwd: scratch};
  let service;
  try {
    await npm(['install', '--global', '--offline', '--ignore-scripts', path.resolve(tarball)], options);
    const real = await runInstalled(state.prefix, ['--version'], options);
    assert.equal(real.status, 0, real.stderr);
    assert.ok(real.stdout.trim().startsWith(`jecode ${metadata.version} (${target.identity}-release,`), real.stdout);
    assert.equal(digest(regularFile(installed(state.prefix).binary)), digest(native));
    const invalid = await runInstalled(state.prefix, ['--npm-fixture-invalid-option'], options);
    assert.equal(invalid.status, 2, invalid.stderr);
    preserved(state);
    await uninstall(state, options);

    const fixtures = new Map();
    for (const value of ['0.0.1', '0.0.2']) fixtures.set(value, await fixture(value, scratch, contents, options));
    service = await registry(fixtures);
    options.env.npm_config_registry = service.origin;
    await npm(['install', '--global', packageName], options);
    await checkFixtureVersion(state.prefix, '0.0.1', options);
    const forwarded = ['text with spaces', 'λ', 'a&b', '$(untouched)', '--flag=value'];
    const args = await command(process.execPath, [installed(state.prefix).launcher, '--args', ...forwarded], options);
    assert.equal(args.status, 0, args.stderr);
    assert.equal(args.stdout, `${forwarded.join('\0')}\0`);
    const input = 'first line\nsecond λ line\n';
    const echoed = await command(process.execPath, [installed(state.prefix).launcher, '--echo'], {...options, input});
    assert.equal(echoed.status, 0, echoed.stderr);
    assert.equal(echoed.stdout, input);
    const exit = await runInstalled(state.prefix, ['--exit', '7'], options);
    assert.equal(exit.status, 7, exit.stderr);
    service.update();
    await npm(['update', '--global', packageName], options);
    await checkFixtureVersion(state.prefix, '0.0.2', options);
    assert.equal(digest(regularFile(installed(state.prefix).binary)), fixtures.get('0.0.2').binaryDigest);
    preserved(state);
    await uninstall(state, options);
    process.stdout.write(`npm lifecycle passed on ${host}: real package install/run/uninstall; registry install/update/uninstall; argv, stdin, exit status and user data preserved.\n`);
  } finally {
    if (service) await service.close();
    // Only the mkdtemp-owned directory is removed; npm never sees the real home.
    if (path.dirname(fs.realpathSync(scratch)) !== fs.realpathSync(path.join(root, '.tmp'))) {
      throw new Error('Refusing to remove a fixture directory outside the workspace scratch root.');
    }
    fs.rmSync(scratch, {recursive: true, force: true});
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 3) {
    console.error('Usage: node tools/npm/lifecycle.mjs <package.tgz>');
    process.exitCode = 1;
  } else lifecycle(process.argv[2]).catch(error => { console.error(error.stack); process.exitCode = 1; });
}
