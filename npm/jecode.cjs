#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const { existsSync } = require('node:fs');
const { constants } = require('node:os');
const path = require('node:path');
const { targetFor } = require('./targets.cjs');

function signalExitCode(signal) {
  const number = constants.signals[signal];
  return Number.isInteger(number) ? 128 + number : 1;
}

function glibcMeetsMinimum(version, minimum) {
  if (!/^\d+\.\d+$/.test(version)) return false;
  const [major, minor] = version.split('.').map(Number);
  const [requiredMajor, requiredMinor] = minimum.split('.').map(Number);
  return major > requiredMajor || (major === requiredMajor && minor >= requiredMinor);
}

function launch({
  runtime = process,
  packageRoot = path.resolve(__dirname, '..'),
  spawnChild = spawn,
  binaryExists = existsSync,
} = {}) {
  const target = targetFor(runtime.platform, runtime.arch);
  if (!target) {
    runtime.stderr.write(
      `jecode: unsupported platform ${runtime.platform}-${runtime.arch}. ` +
      'Supported: win32-x64, linux-x64, darwin-arm64.\n',
    );
    runtime.exitCode = 1;
    return null;
  }

  if (runtime.platform === 'linux') {
    let glibc;
    try {
      glibc = runtime.report?.getReport()?.header?.glibcVersionRuntime;
    } catch {
      // An unreadable diagnostic report cannot establish libc compatibility.
    }
    if (typeof glibc !== 'string' || !glibcMeetsMinimum(glibc, target.glibcMinimum)) {
      const detail = glibc ? `glibc ${glibc} is too old or unrecognized` : 'unsupported Linux libc';
      runtime.stderr.write(
        `jecode: ${detail}; requires glibc ${target.glibcMinimum} or newer.\n`,
      );
      runtime.exitCode = 1;
      return null;
    }
  }

  const executable = path.join(packageRoot, 'native', target.key, target.binary);
  if (!binaryExists(executable)) {
    runtime.stderr.write(
      `jecode: missing native executable for ${target.key}. ` +
      'Reinstall the npm package and check its contents.\n',
    );
    runtime.exitCode = 1;
    return null;
  }

  let child;
  try {
    child = spawnChild(executable, runtime.argv.slice(2), {
      stdio: 'inherit',
      shell: false,
    });
  } catch (error) {
    runtime.stderr.write(`jecode: could not start ${target.key}: ${error.message}\n`);
    runtime.exitCode = 1;
    return null;
  }

  let spawnError = false;
  const onInterrupt = () => {
    // The terminal sends Ctrl+C to the foreground group. The child already
    // receives it; forwarding here could interrupt it twice or kill it on Windows.
  };
  const onTerminate = () => child.kill('SIGTERM');
  const onHangup = () => child.kill('SIGHUP');
  runtime.on('SIGINT', onInterrupt);
  runtime.on('SIGTERM', onTerminate);
  if (runtime.platform !== 'win32') runtime.on('SIGHUP', onHangup);

  child.on('error', (error) => {
    spawnError = true;
    runtime.stderr.write(`jecode: could not start ${target.key}: ${error.message}\n`);
    runtime.exitCode = 1;
  });
  child.on('close', (code, signal) => {
    runtime.removeListener('SIGINT', onInterrupt);
    runtime.removeListener('SIGTERM', onTerminate);
    if (runtime.platform !== 'win32') runtime.removeListener('SIGHUP', onHangup);
    if (!spawnError) runtime.exitCode = signal ? signalExitCode(signal) : (code ?? 1);
  });
  return child;
}

if (require.main === module) launch();

module.exports = { launch, signalExitCode };
