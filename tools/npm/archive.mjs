import {gunzipSync} from 'node:zlib';
import {digest} from './common.mjs';

// npm pack writes regular files in a gzip-compressed tar archive. Read them
// without extraction: release validation must never follow archive paths.
export function unpack(bytes) {
  const tar = gunzipSync(bytes, {maxOutputLength: 256 * 1024 * 1024});
  const files = new Map();
  let offset = 0;
  while (offset + 512 <= tar.length && tar[offset] !== 0) {
    const header = tar.subarray(offset, offset + 512);
    const text = (start, size) => header.subarray(start, start + size).toString('utf8').split('\0')[0];
    const name = text(0, 100);
    const prefix = text(345, 155);
    const file = prefix ? `${prefix}/${name}` : name;
    const sizeText = text(124, 12).trim();
    if (!/^[0-7]+$/.test(sizeText)) throw new Error('Unsupported tar entry size.');
    const size = Number.parseInt(sizeText, 8);
    const checksum = Number.parseInt(text(148, 8).trim(), 8);
    let actual = 0;
    for (let i = 0; i < 512; i++) actual += i >= 148 && i < 156 ? 32 : header[i];
    if (checksum !== actual) throw new Error('Invalid tar header checksum.');
    if (!['0', '\0', '5'].includes(String.fromCharCode(header[156]))) {
      throw new Error(`Unsupported tar entry: ${file}`);
    }
    if (!file.startsWith('package/') || file.includes('\\') ||
        file.split('/').some(part => part === '..' || part === '.') || files.has(file)) {
      throw new Error(`Invalid or repeated package path: ${file}`);
    }
    const start = offset + 512;
    if (start + size > tar.length) throw new Error('Truncated npm archive.');
    if (header[156] !== 53) files.set(file, Buffer.from(tar.subarray(start, start + size)));
    offset = start + Math.ceil(size / 512) * 512;
  }
  if (!files.size) throw new Error('The npm archive is empty.');
  return files;
}

export function verifyArchive(bytes, expected) {
  const files = unpack(bytes);
  if (files.size !== expected.size) throw new Error('Unexpected files in the npm archive.');
  for (const [name, content] of expected) {
    const packed = files.get(`package/${name}`);
    if (!packed || digest(packed) !== digest(content)) {
      throw new Error(`Missing or changed npm payload: ${name}`);
    }
  }
  return files;
}
