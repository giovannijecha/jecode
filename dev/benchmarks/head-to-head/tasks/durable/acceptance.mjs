import { pathToFileURL } from 'node:url';
import { resolve, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { runChecks } from './checks.mjs';

const root = resolve(process.argv[2]);
const api = await import(pathToFileURL(join(root, 'src/index.js')));
const results = [];
await runChecks(api, async (name, run) => {
  let timeout;
  try {
    await Promise.race([Promise.resolve().then(run), new Promise((_, reject) => {
      timeout = setTimeout(() => reject(new Error('check deadline')), 3000);
    })]);
    results.push({ name, passed: true });
  } catch (error) { results.push({ name, passed: false, error: String(error).slice(0, 500) }); }
  finally { clearTimeout(timeout); }
});
const old = spawnSync(process.execPath, [new URL('./planner-acceptance.mjs', import.meta.url).pathname, root],
  { encoding: 'utf8', timeout: 10000 });
try {
  const report = JSON.parse(old.stdout);
  results.push(...report.results.map(row => ({ ...row, name: `original planner: ${row.name}` })));
} catch { results.push({ name: 'original planner evaluator', passed: false, error: 'evaluator did not return valid results' }); }
console.log(JSON.stringify({ passed: results.filter(r => r.passed).length, total: results.length, results }, null, 2));
process.exitCode = results.every(row => row.passed) ? 0 : 1;
