import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { patchDocument } from './patch.js';
export async function migrateFiles(root, changes, { dryRun = false } = {}) {
  const changed = [];
  for (const change of changes) {
    const file = resolve(root, change.path);
    const document = JSON.parse(await readFile(file, 'utf8'));
    const next = patchDocument(document, change.operations);
    if (JSON.stringify(document) === JSON.stringify(next)) continue;
    if (!dryRun) await writeFile(file, JSON.stringify(next, null, 2) + '\n');
    changed.push(change.path);
  }
  return { changed };
}
