import assert from 'node:assert/strict';
import { reference } from './reference.mjs';
import { runChecks } from './checks.mjs';

const api = await reference(process.argv[2]);
const results = [];
await runChecks(api, async (name, run) => {
  try { await run(); results.push({ name, passed: true }); }
  catch (error) { results.push({ name, passed: false, error: String(error) }); }
});
console.log(JSON.stringify({ reference: true, passed: results.filter(r => r.passed).length, total: results.length, results }, null, 2));
assert.ok(results.every(r => r.passed), 'reference must pass every contract check');

const mutations = [
  ['serialized workers', 'bounded concurrency over a wide graph', {
    ...api, executePlan: (document, options) => api.executePlan(document, { ...options, concurrency: 1 }),
  }],
  ['disabled persistence', 'resume skips successes and retries failures', {
    ...api, executePlan: (document, options) => api.executePlan(document, { ...options, checkpoint: undefined }),
  }],
  ['failure reported as success', 'worker failure blocks transitive dependents only', {
    ...api, async executePlan(document, options) { return { ...await api.executePlan(document, options), status: 'completed' }; },
  }],
  ['no execution', 'sequential lexical readiness and prototype IDs', {
    ...api, async executePlan(document) { return { status: 'completed', completed: document.tasks.map(t => t.id).sort(), failed: [], blocked: [], pending: [] }; },
  }],
];
for (const [name, selected, mutant] of mutations) {
  let detected = false;
  await runChecks(mutant, async (label, run) => {
    if (label !== selected) return;
    try { await run(); } catch { detected = true; }
  });
  console.log(JSON.stringify({ mutation: name, detected }));
  assert.ok(detected, `evaluator must reject ${name}`);
}
