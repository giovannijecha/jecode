'use strict';

const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { EventEmitter, once } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { launch, signalExitCode } = require('./jecode.cjs');
const { targets, targetFor } = require('./targets.cjs');

function runtimeFor(platform = process.platform, arch = process.arch) {
  const runtime = new EventEmitter();
  const errors = [];
  runtime.platform = platform;
  runtime.arch = arch;
  runtime.report = { getReport: () => ({ header: { glibcVersionRuntime: '2.39' } }) };
  runtime.argv = ['node', 'jecode.cjs', '--plain', 'two words'];
  runtime.stderr = { write: (message) => errors.push(message) };
  runtime.exitCode = undefined;
  return { runtime, errors };
}

function fakeChild() {
  const child = new EventEmitter();
  const signals = [];
  child.kill = (signal) => {
    signals.push(signal);
    return true;
  };
  return { child, signals };
}

test('target map matches exactly the three release artifact identities', () => {
  assert.deepEqual(Object.keys(targets), ['win32-x64', 'linux-x64', 'darwin-arm64']);
  assert.equal(targets['linux-x64'].glibcMinimum, '2.39');
  for (const [key, target] of Object.entries(targets)) {
    assert.deepEqual(targetFor(target.platform, target.arch), { key, ...target });
  }
  assert.equal(targetFor('darwin', 'x64'), null);
  assert.equal(targetFor('linux', 'arm64'), null);
  assert.equal(targetFor('freebsd', 'x64'), null);
});

test('unsupported pair fails clearly without spawning', () => {
  const { runtime, errors } = runtimeFor('darwin', 'x64');
  const result = launch({
    runtime,
    spawnChild: () => assert.fail('must not spawn'),
  });
  assert.equal(result, null);
  assert.equal(runtime.exitCode, 1);
  assert.match(errors.join(''), /unsupported platform darwin-x64/);
});

test('missing packaged binary fails clearly without spawning', () => {
  const { runtime, errors } = runtimeFor('win32', 'x64');
  const result = launch({
    runtime,
    packageRoot: path.join(os.tmpdir(), 'jecode-absent-package'),
    binaryExists: () => false,
    spawnChild: () => assert.fail('must not spawn'),
  });
  assert.equal(result, null);
  assert.equal(runtime.exitCode, 1);
  assert.match(errors.join(''), /missing native executable for win32-x64/);
});

test('Linux without glibc fails before checking the binary or spawning', () => {
  const { runtime, errors } = runtimeFor('linux', 'x64');
  runtime.report.getReport = () => ({ header: {} });
  const result = launch({
    runtime,
    binaryExists: () => assert.fail('must not inspect the binary'),
    spawnChild: () => assert.fail('must not spawn'),
  });
  assert.equal(result, null);
  assert.equal(runtime.exitCode, 1);
  assert.match(errors.join(''), /unsupported Linux libc; requires glibc 2\.39 or newer/);
});

test('Linux rejects glibc 2.38 and accepts 2.39 and 3.0', () => {
  for (const [version, accepted] of [['2.38', false], ['2.39', true], ['3.0', true]]) {
    const { runtime, errors } = runtimeFor('linux', 'x64');
    runtime.report.getReport = () => ({ header: { glibcVersionRuntime: version } });
    const { child } = fakeChild();
    let spawned = false;
    const result = launch({
      runtime,
      binaryExists: () => true,
      spawnChild: () => {
        spawned = true;
        return child;
      },
    });
    assert.equal(spawned, accepted, version);
    if (accepted) {
      assert.equal(result, child);
      child.emit('close', 0, null);
      assert.equal(runtime.exitCode, 0);
      assert.deepEqual(errors, []);
    } else {
      assert.equal(result, null);
      assert.equal(runtime.exitCode, 1);
      assert.match(errors.join(''), /glibc 2\.38.*requires glibc 2\.39 or newer/);
    }
  }
});

test('libc report is read only on supported Linux', () => {
  for (const [platform, arch] of [['win32', 'x64'], ['darwin', 'arm64']]) {
    const { runtime } = runtimeFor(platform, arch);
    runtime.report.getReport = () => assert.fail('must not read a report');
    const { child } = fakeChild();
    launch({ runtime, binaryExists: () => true, spawnChild: () => child });
    child.emit('close', 0, null);
    assert.equal(runtime.exitCode, 0);
  }
});

test('launcher passes executable, arguments and inherited terminal contract', () => {
  const { runtime } = runtimeFor('linux', 'x64');
  const { child } = fakeChild();
  const packageRoot = path.join(os.tmpdir(), 'jecode-test-package');
  let spawned;
  launch({
    runtime,
    packageRoot,
    binaryExists: () => true,
    spawnChild: (...args) => {
      spawned = args;
      return child;
    },
  });
  assert.deepEqual(spawned, [
    path.join(packageRoot, 'native', 'linux-x64', 'jecode'),
    ['--plain', 'two words'],
    { stdio: 'inherit', shell: false },
  ]);
  child.emit('close', 27, null);
  assert.equal(runtime.exitCode, 27);
  assert.equal(runtime.listenerCount('SIGINT'), 0);
  assert.equal(runtime.listenerCount('SIGTERM'), 0);
  assert.equal(runtime.listenerCount('SIGHUP'), 0);
});

test('terminal Ctrl+C is not forwarded twice; termination and child signal are handled', () => {
  const { runtime } = runtimeFor('win32', 'x64');
  const { child, signals } = fakeChild();
  launch({ runtime, binaryExists: () => true, spawnChild: () => child });
  runtime.emit('SIGINT');
  assert.deepEqual(signals, []);
  runtime.emit('SIGTERM');
  assert.deepEqual(signals, ['SIGTERM']);
  child.emit('close', null, 'SIGTERM');
  assert.equal(runtime.exitCode, signalExitCode('SIGTERM'));
  assert.equal(runtime.listenerCount('SIGINT'), 0);
});

test('spawn failures return failure even if close follows', () => {
  const { runtime, errors } = runtimeFor('linux', 'x64');
  const { child } = fakeChild();
  launch({ runtime, binaryExists: () => true, spawnChild: () => child });
  child.emit('error', new Error('access denied'));
  child.emit('close', -1, null);
  assert.equal(runtime.exitCode, 1);
  assert.match(errors.join(''), /access denied/);
});

test('a real child inherits cwd and environment and returns its exit code', async (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'jecode-launcher-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const output = path.join(directory, 'result.json');
  const fixture = [
    "const fs = require('node:fs');",
    "const [output, ...args] = process.argv.slice(1);",
    "fs.writeFileSync(output, JSON.stringify({ args, cwd: process.cwd(), marker: process.env.JECODE_LAUNCHER_TEST }));",
    'process.exit(23);',
  ].join(' ');
  const marker = `fixture-${process.pid}`;
  const previous = process.env.JECODE_LAUNCHER_TEST;
  process.env.JECODE_LAUNCHER_TEST = marker;
  t.after(() => {
    if (previous === undefined) delete process.env.JECODE_LAUNCHER_TEST;
    else process.env.JECODE_LAUNCHER_TEST = previous;
  });

  const { runtime } = runtimeFor('linux', 'x64');
  const child = launch({
    runtime,
    binaryExists: () => true,
    spawnChild: (_executable, args, options) =>
      spawn(process.execPath, ['-e', fixture, output, ...args], options),
  });
  await once(child, 'close');
  assert.deepEqual(JSON.parse(fs.readFileSync(output, 'utf8')), {
    args: ['--plain', 'two words'],
    cwd: process.cwd(),
    marker,
  });
  assert.equal(runtime.exitCode, 23);
});
