import {execFileSync} from 'node:child_process';
import {embeddedFiles, exceeds, requireFiles} from './common.mjs';

const Elf = Buffer.from([127, 69, 76, 70]);

export function checkVersions(text, ceiling, file) {
  const required = [...text.matchAll(/Name: GLIBC_(\d+(?:\.\d+)+)/g)].map(match => match[1]);
  if (!required.length || text.includes('GLIBC_PRIVATE')) {
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
