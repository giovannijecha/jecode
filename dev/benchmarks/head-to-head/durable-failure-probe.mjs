// Review-derived probe, separate from the frozen durable task acceptance score.
// JavaScript callbacks may reject with any value; absence is not a failure flag.
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const { executePlan } = await import(pathToFileURL(resolve(process.argv[2], 'src/index.js')));
const tasks = ['a', 'b', 'z'].map(id => ({ id, deps: [], duration: 1 }));
const tick = () => new Promise(done => setImmediate(done));
const results = [];
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};
async function until(condition) {
  for (let i = 0; i < 100 && !condition(); i++) await tick();
  assert.ok(condition(), 'synchronization point was not reached');
}
async function check(name, run) {
  let timer;
  try {
    await Promise.race([run(), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('probe did not settle within 3 seconds')), 3000);
    })]);
    results.push({ name, passed: true });
  } catch (error) {
    results.push({ name, passed: false, error: String(error) });
  } finally { clearTimeout(timer); }
}
function sameFailure(actual, expected) {
  return actual === expected || (actual !== null && typeof actual === 'object' &&
    Object.hasOwn(actual, 'cause') && actual.cause === expected);
}

for (const [label, reason] of [['undefined', undefined], ['null', null], ['false', false],
  ['zero', 0], ['string', 'storage failure']]) {
  for (const asynchronous of [false, true]) {
    for (const phase of ['load', 'initial-save', 'commit']) {
      await check(`${phase} preserves ${asynchronous ? 'rejected' : 'thrown'} ${label}`, async () => {
        const gate = deferred(), calls = [], signals = [];
        let saves = 0, failureIssued = false, settled = false;
        const fail = () => {
          failureIssued = true;
          if (asynchronous) return Promise.reject(reason);
          throw reason;
        };
        const observed = Promise.resolve().then(() => executePlan({ tasks }, {
          concurrency: 2,
          run(task, { signal }) {
            calls.push(task.id); signals.push(signal);
            return task.id === 'b' ? gate.promise : undefined;
          },
          checkpoint: {
            load() { return phase === 'load' ? fail() : null; },
            save() {
              saves++;
              if (phase === 'initial-save' || (phase === 'commit' && saves === 2)) return fail();
            },
          },
        })).then(value => ({ resolved: true, value }), error => ({ resolved: false, error }));
        observed.then(() => { settled = true; });
        try {
          await until(() => failureIssued); await tick();
          if (phase === 'commit') {
            assert.deepEqual(calls, ['a', 'b']);
            assert.ok(signals.every(signal => signal.aborted));
            assert.equal(settled, false, 'active worker must quiesce before rejection');
          } else assert.deepEqual(calls, []);
        } finally { gate.resolve(); }
        const outcome = await observed;
        assert.equal(outcome.resolved, false, 'adapter failure must reject execution');
        assert.ok(sameFailure(outcome.error, reason), 'original failure must be preserved');
        await tick();
        assert.equal(saves, phase === 'load' ? 0 : phase === 'initial-save' ? 1 : 2);
        assert.deepEqual(calls, phase === 'commit' ? ['a', 'b'] : []);
      });
    }
  }
  await check(`worker rejection with ${label} remains a failure`, async () => {
    const calls = [];
    const document = { tasks: [tasks[0], { ...tasks[1], deps: ['a'] }, tasks[2]] };
    const value = await executePlan(document, { run(task) {
      calls.push(task.id);
      if (task.id === 'a') return Promise.reject(reason);
    } });
    assert.deepEqual(value, { status: 'failed', completed: ['z'], failed: ['a'], blocked: ['b'], pending: [] });
    assert.deepEqual(calls.sort(), ['a', 'z']);
  });
}
await check('stable accessors accepted by the planner remain supported', async () => {
  const source = { get id() { return 'a'; }, get deps() { return []; }, get duration() { return 1; } };
  const calls = [];
  const value = await executePlan({ tasks: [source] }, { run(task) { calls.push(task.id); } });
  assert.equal(value.status, 'completed');
  assert.deepEqual(calls, ['a']);
});
for (const field of ['id', 'duration']) {
  await check(`capture never executes or persists an invalid changing ${field}`, async () => {
    let reads = 0, loads = 0;
    const good = field === 'id' ? 'a' : 1;
    const bad = field === 'id' ? 'invalid id' : -1;
    // Validation reads id once and duration twice in the preserved planner.
    // A valid first capture is also acceptable; no exact getter count is required.
    const source = { id: 'a', deps: [], duration: 1 };
    Object.defineProperty(source, field, { enumerable: true,
      get: () => ++reads <= (field === 'id' ? 1 : 2) ? good : bad });
    const observed = [];
    let rejected = false;
    try {
      await executePlan({ tasks: [source] }, {
        run(task) { observed.push({ id: task.id, duration: task.duration }); },
        checkpoint: {
          load() { loads++; return null; },
          save(snapshot) { observed.push(...snapshot.tasks.map(task => ({ id: task.id, duration: task.duration }))); },
        },
      });
    } catch { rejected = true; }
    assert.ok(observed.every(task => typeof task.id === 'string' && /^[A-Za-z0-9_-]+$/.test(task.id) &&
      Number.isSafeInteger(task.duration) && task.duration >= 0), 'invalid captured values reached an effect');
    if (rejected) assert.equal(loads, 0, 'invalid capture must fail before checkpoint IO');
  });
}
console.log(JSON.stringify({ scope: 'review-derived; not the original acceptance score', results }, null, 2));
process.exitCode = results.every(result => result.passed) ? 0 : 1;
