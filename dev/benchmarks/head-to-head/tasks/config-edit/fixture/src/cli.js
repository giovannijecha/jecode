import { readFile } from 'node:fs/promises';
import { migrateFiles } from './index.js';
const [root, manifest, flag] = process.argv.slice(2);
try {
  if (!root || !manifest || (flag && flag !== '--dry-run')) throw new Error('Usage: ROOT MANIFEST [--dry-run]');
  console.log(JSON.stringify(await migrateFiles(root, JSON.parse(await readFile(manifest, 'utf8')), { dryRun: flag === '--dry-run' })));
} catch (error) { console.error(error.message); process.exitCode = 1; }
