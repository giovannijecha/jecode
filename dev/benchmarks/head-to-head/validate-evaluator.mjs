// Calibrate an evaluator independently of all measured participant artifacts.
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm, cp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
export async function validateEvaluator(url, mutations, { cli } = {}) {
  const task = dirname(fileURLToPath(url));
  const root = await mkdtemp(join(tmpdir(), 'evaluator-calibration-'));
  try {
    await cp(join(task, 'fixture'), root, { recursive: true });
    await mkdir(join(root, 'src'), { recursive: true });
    const reference = await readFile(join(task, 'reference.mjs'), 'utf8');
    if (cli) await writeFile(join(root, 'src/cli.js'), cli);
    const run = () => {
      const result = spawnSync(process.execPath, [join(task, 'acceptance.mjs'), root], { encoding: 'utf8', timeout: 25000 });
      assert.ok([0, 1].includes(result.status), result.stderr || String(result.error));
      assert.ok(result.stdout.trim(), result.stderr || 'Evaluator produced no JSON');
      return JSON.parse(result.stdout);
    };
    await writeFile(join(root, 'src/index.js'), reference);
    const good = run(); assert.equal(good.passed, good.total, JSON.stringify(good.results.filter(row => !row.passed)));
    const reports = [{ name: 'reference', total: good.total, passed: good.passed }];
    for (const [name, from, to] of mutations) {
      assert.ok(reference.includes(from), `mutation ${name} did not match`);
      await writeFile(join(root, 'src/index.js'), reference.replaceAll(from, to));
      const bad = run(); assert.ok(bad.passed < bad.total, `${name} was not detected`);
      reports.push({ name, passed: bad.passed, total: bad.total });
    }
    console.log(JSON.stringify(reports));
  } finally { await rm(root, { recursive: true, force: true }); }
}
