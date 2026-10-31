'use strict';

const targets = Object.freeze({
  'win32-x64': Object.freeze({
    platform: 'win32',
    arch: 'x64',
    binary: 'jecode.exe',
    identity: 'windows-x86_64',
  }),
  'linux-x64': Object.freeze({
    platform: 'linux',
    arch: 'x64',
    binary: 'jecode',
    identity: 'linux-x86_64',
    glibcMinimum: '2.39',
  }),
  'darwin-arm64': Object.freeze({
    platform: 'darwin',
    arch: 'arm64',
    binary: 'jecode',
    identity: 'macos-aarch64',
  }),
});

function targetFor(platform, arch) {
  const key = `${platform}-${arch}`;
  return Object.hasOwn(targets, key) ? { key, ...targets[key] } : null;
}

module.exports = { targets, targetFor };
