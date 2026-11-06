import {execFileSync} from 'node:child_process';
import {embeddedFiles, exceeds, requireFiles} from './common.mjs';

const Elf = Buffer.from([127, 69, 76, 70]);

export function checkVersions(text, ceiling, file) {
  // A symbolic need such as GLIBC_ABI_DT_RELR has no number to compare, and old loaders
  // refuse it: anything but a numbered public version fails the audit.
  const required = [...text.matchAll(/Name: GLIBC_(\S+)/g)].map(match => match[1]);
  if (!required.length || required.some(version => !/^\d+(?:\.\d+)+$/.test(version))) {
    throw new Error(`Could not establish a public glibc ABI for ${file}`);
  }
  for (const version of required) {
    if (exceeds(version, ceiling)) {
      throw new Error(`${file} requires glibc ${version}, above the supported minimum ${ceiling}. Build against tools/linux-sysroot.sh before releasing.`);
    }
  }
}

export function auditLinux(binary, mainBytes, ceiling) {
  const files = embeddedFiles(binary, mainBytes, bytes => bytes.subarray(0, 4).equals(Elf));
  requireFiles(files, [
    'jecode-shell', 'jecode-clipboard_linux', 'jecode-http', 'jecode-openai_crypto',
    'jecode-process_unix', 'jecode-pty_unix', 'jecode-terminal_unix',
  ], 'Linux');
  files.set('jecode', binary);
  for (const [name, file] of files) {
    const report = execFileSync('readelf', ['--version-info', file], {encoding: 'utf8', env: {...process.env, LC_ALL: 'C'}});
    checkVersions(report, ceiling, name);
  }
  return [...files.keys()];
}
