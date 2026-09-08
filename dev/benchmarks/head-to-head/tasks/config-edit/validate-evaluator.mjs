import { validateEvaluator } from '../../validate-evaluator.mjs';
await validateEvaluator(import.meta.url, [
  ['pointer escape', "if (/~(?![01])/u.test(part)) throw new Error('escape');", ''],
  ['rollback', 'for (const entry of committed.reverse())', 'for (const entry of [])'],
  ['cancellation', 'signal?.throwIfAborted();', ''],
  ['detachment', 'parent[key] = value;', 'parent[key] = operation.value;'],
], { cli: `import { readFile } from 'node:fs/promises';
import { migrateFiles } from './index.js';
try {
 const args = process.argv.slice(2), [root, manifest, flag] = args;
 if (args.length < 2 || args.length > 3 || (flag && flag !== '--dry-run')) throw new Error('usage');
 console.log(JSON.stringify(await migrateFiles(root, JSON.parse(await readFile(manifest, 'utf8')), { dryRun: flag === '--dry-run' })));
} catch (error) { console.error(String(error)); process.exitCode = 1; }
` });
