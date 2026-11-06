import fs from 'node:fs';
import {embeddedFiles, exceeds, requireFiles} from './common.mjs';

const Magic64 = 0xfeedfacf, BuildVersion = 0x32, VersionMinMacos = 0x24, PlatformMacos = 1;

function isMachO(bytes) {
  return bytes.length >= 32 && bytes.readUInt32LE(0) === Magic64;
}

function packed(value) {
  return `${value >>> 16}.${(value >>> 8) & 0xff}.${value & 0xff}`;
}

// The deployment target the linker recorded: LC_BUILD_VERSION, or the older
// LC_VERSION_MIN_MACOSX. Files without either one fail rather than pass.
export function machOMinimum(bytes, file) {
  if (!isMachO(bytes)) throw new Error(`${file} is not a 64-bit Mach-O file.`);
  const commands = bytes.readUInt32LE(16);
  let offset = 32;
  for (let index = 0; index < commands; index++) {
    const kind = bytes.readUInt32LE(offset), size = bytes.readUInt32LE(offset + 4);
    if (kind === BuildVersion) {
      const platform = bytes.readUInt32LE(offset + 8);
      if (platform !== PlatformMacos) throw new Error(`${file} targets Apple platform ${platform}, not macOS.`);
      return packed(bytes.readUInt32LE(offset + 12));
    }
    if (kind === VersionMinMacos) return packed(bytes.readUInt32LE(offset + 8));
    if (size < 8) break;
    offset += size;
  }
  throw new Error(`${file} records no macOS deployment target.`);
}

export function auditMacos(binary, mainBytes, ceiling) {
  const files = embeddedFiles(binary, mainBytes, isMachO);
  requireFiles(files, [
    'jecode-shell', 'jecode-http', 'jecode-openai_crypto',
    'jecode-process_unix', 'jecode-pty_unix', 'jecode-terminal_unix',
  ], 'macOS');
  files.set('jecode', binary);
  for (const [name, file] of files) {
    const minimum = machOMinimum(fs.readFileSync(file), name);
    if (exceeds(minimum, ceiling)) {
      throw new Error(`${name} requires macOS ${minimum}, above the supported minimum ${ceiling}. Set MACOSX_DEPLOYMENT_TARGET=${ceiling} before releasing.`);
    }
  }
  return [...files.keys()];
}
