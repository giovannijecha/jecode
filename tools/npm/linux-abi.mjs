import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';

export function exceeds(version, ceiling) {
  const left = version.split('.').map(Number), right = ceiling.split('.').map(Number);
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    if ((left[i] ?? 0) !== (right[i] ?? 0)) return (left[i] ?? 0) > (right[i] ?? 0);
  }
  return false;
}

export function checkVersions(text, ceiling, file) {
  const required = [...text.matchAll(/Name: GLIBC_(\d+(?:\.\d+)+)/g)].map(match => match[1]);
  if (!required.length || text.includes('GLIBC_PRIVATE')) {
    throw new Error(`Could not establish a public glibc ABI for ${file}`);
  }
  for (const version of required) {
    if (exceeds(version, ceiling)) {
      throw new Error(`${file} requires glibc ${version}, above the supported minimum ${ceiling}. Build against the supported baseline before releasing.`);
    }
  }
}

export function auditLinux(binary, mainBytes, ceiling) {
  const build = path.join(path.dirname(binary), 'build');
  if (!fs.existsSync(build)) throw new Error('Stage Linux binaries from their Cargo release output directory.');
  const expected = new Set([
    'jecode-shell', 'jecode-clipboard_linux', 'jecode-http', 'jecode-openai_crypto',
    'jecode-process_unix', 'jecode-pty_unix', 'jecode-terminal_unix',
  ]);
  const files = new Map([['jecode', binary]]);
  for (const entry of fs.readdirSync(build, {withFileTypes: true})) {
    if (!entry.isDirectory() || !entry.name.startsWith('jecode-')) continue;
    const output = path.join(build, entry.name, 'out');
    if (!fs.existsSync(output)) continue;
    for (const helper of fs.readdirSync(output, {withFileTypes: true})) {
      if (!helper.isFile() || !helper.name.startsWith('jecode-')) continue;
      const name = helper.name, file = path.join(output, name);
      const bytes = fs.readFileSync(file);
      // Discard stale Cargo outputs: only inspect the exact embedded payload.
      if (bytes.subarray(0, 4).equals(Buffer.from([127, 69, 76, 70])) && mainBytes.indexOf(bytes) >= 0) {
        files.set(name, file);
      }
    }
  }
  for (const name of expected) {
    if (!files.has(name)) throw new Error(`Cannot audit embedded Linux helper ${name}; rebuild the release first.`);
  }
  for (const [name, file] of files) {
    const report = execFileSync('readelf', ['--version-info', file], {encoding: 'utf8', env: {...process.env, LC_ALL: 'C'}});
    checkVersions(report, ceiling, name);
  }
  return [...files.keys()];
}
