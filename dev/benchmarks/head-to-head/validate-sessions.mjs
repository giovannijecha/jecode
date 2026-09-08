// Validate Jecode's saved pilot nodes with the measured runtime's production codec.
import { readFileSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const { decodeNode } = await import(pathToFileURL(join(root, 'jecode/src/sessions/codec.ts')).href);
let validated = 0;
for (const run of readdirSync(join(root, 'runs'))) {
  const directory = join(root, 'runs', run);
  const manifest = JSON.parse(readFileSync(join(directory, 'manifest.json'), 'utf8'));
  if (manifest.client !== 'jecode' || manifest.preflight) continue;
  for (const relative of readdirSync(join(directory, 'home/sessions'), { recursive: true })) {
    if (/[\/]nodes[\/][0-9]+\.json$/.test(relative)) {
      decodeNode(JSON.parse(readFileSync(join(directory, 'home/sessions', relative), 'utf8')));
      validated++;
    }
  }
}
console.log(JSON.stringify({ validated, codec: 'measured source snapshot' }));
