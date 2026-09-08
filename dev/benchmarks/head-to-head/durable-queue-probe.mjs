// Post-review scaling observation, not part of the original task score.
// Keep one worker pending while independent short workers refill the other slot.
import { createHook } from 'node:async_hooks';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const { executePlan } = await import(pathToFileURL(resolve(process.argv[2], 'src/index.js')));
const tick = () => new Promise(done => setImmediate(done));
const results = [];
for (const count of [1000, 5000]) {
  const tasks = Array.from({ length: count + 1 }, (_, i) => ({
    id: String(i).padStart(6, '0'), deps: [], duration: 1,
  }));
  let release, finished = 0, created = 0;
  const gate = new Promise(done => { release = done; });
  const pending = new Set();
  const hook = createHook({
    init(id, type) { if (type === 'PROMISE') { pending.add(id); created++; } },
    promiseResolve(id) { pending.delete(id); },
  });
  hook.enable();
  const start = performance.now();
  const execution = Promise.resolve().then(() => executePlan({ tasks }, {
    concurrency: 2,
    run(task) {
      if (task.id === '000000') return gate;
      finished++;
    },
  }));
  // Observe rejection immediately while waiting for the refill boundary.
  const observed = execution.then(value => ({ value }), error => ({ error: String(error) }));
  let beforeRelease, refilled;
  try {
    for (let i = 0; i < 200 && finished < count; i++) await tick();
    await tick();
    refilled = finished === count;
    beforeRelease = pending.size;
  } finally { release(); }
  const outcome = await observed;
  await tick();
  const afterRelease = pending.size;
  hook.disable();
  results.push({ shortTasks: count, refilledBeforeSlowWorker: refilled,
    createdPromises: created, pendingBeforeRelease: beforeRelease,
    pendingAfterRelease: afterRelease, instrumentedMs: performance.now() - start,
    completed: outcome.value?.completed?.length, status: outcome.value?.status,
    ...(outcome.error === undefined ? {} : { error: outcome.error }),
  });
}
console.log(JSON.stringify({
  scope: 'review-derived scaling observation; not an acceptance or latency score',
  measurement: 'PROMISE init events without a matching promiseResolve event; not fulfillment or retained heap bytes',
  results,
}, null, 2));
process.exitCode = results.every(row => row.refilledBeforeSlowWorker &&
  row.status === 'completed' && row.completed === row.shortTasks + 1) ? 0 : 1;
