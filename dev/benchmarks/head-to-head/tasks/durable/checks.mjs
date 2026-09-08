// Contract-derived checks. The durable validation report distinguishes the
// frozen live evaluator from this reviewed callback revision.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, readdir, mkdir, symlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export const task = (id, deps = [], duration = 1) => ({ id, deps, duration });
const doc = (...tasks) => ({ tasks });
const tick = () => new Promise(resolve => setImmediate(resolve));
const defer = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
async function until(condition) {
  for (let index = 0; index < 100; index++) { if (condition()) return; await tick(); }
  assert.ok(condition(), 'expected synchronization point was not reached');
}
function observe(promise) { promise.catch(() => {}); return promise; }
const snapshot = (document, completed = []) => ({ version: 1,
  tasks: document.tasks.map(t => ({ id: t.id, deps: [...t.deps].sort(), duration: t.duration }))
    .sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0), completed: [...completed].sort() });
const result = (status, completed = [], failed = [], blocked = [], pending = []) =>
  ({ status, completed: [...completed].sort(), failed: [...failed].sort(),
    blocked: [...blocked].sort(), pending: [...pending].sort() });

export async function runChecks(api, check) {
  const { executePlan: execute, createCheckpointStore: store } = api;
  await check('new public exports', () => {
    assert.equal(typeof execute, 'function'); assert.equal(typeof store, 'function');
  });
  if (typeof execute !== 'function' || typeof store !== 'function') {
    const original = check;
    check = (name) => original(name, () => assert.fail('required exports unavailable'));
  }
  await check('empty graph', async () => assert.deepEqual(await execute(doc(), {
    run() { assert.fail('empty must not run'); },
  }), result('completed')));
  await check('sequential lexical readiness and prototype IDs', async () => {
    const order = [];
    const input = doc(task('z'), task('a', ['b']), task('b'), task('__proto__'), task('constructor'));
    assert.deepEqual(await execute(input, { concurrency: 1, run(t) { order.push(t.id); } }),
      result('completed', input.tasks.map(t => t.id)));
    assert.deepEqual(order, ['__proto__', 'b', 'a', 'constructor', 'z']);
  });
  await check('default overlap, refill without waiting for a wave', async () => {
    const slow = defer(), started = []; const input = doc(task('a'), task('b'), task('c', ['b']));
    const running = observe(execute(input, { run(t) { started.push(t.id); return t.id === 'a' ? slow.promise : undefined; } }));
    try { await until(() => started.includes('c')); assert.deepEqual(started, ['a', 'b', 'c']); }
    finally { slow.resolve(); }
    assert.deepEqual(await running, result('completed', ['a', 'b', 'c']));
  });
  await check('bounded concurrency over a wide graph', async () => {
    let active = 0, peak = 0; const seen = [];
    const input = doc(...Array.from({ length: 80 }, (_, i) => task(`t${i}`)));
    await execute(input, { concurrency: 3, async run(t) {
      active++; peak = Math.max(peak, active); seen.push(t.id); await tick(); active--;
    } });
    assert.equal(peak, 3); assert.equal(active, 0); assert.equal(new Set(seen).size, 80);
  });
  await check('worker failure blocks transitive dependents only', async () => {
    const seen = []; const input = doc(task('a'), task('b', ['a']), task('c', ['b']), task('d'));
    const value = await execute(input, { run(t) { seen.push(t.id); if (t.id === 'a') throw new Error('worker'); } });
    assert.deepEqual(value, result('failed', ['d'], ['a'], ['b', 'c']));
    assert.deepEqual(seen.sort(), ['a', 'd']);
  });
  await check('rejected worker and diamond failure', async () => {
    const input = doc(task('a'), task('b'), task('c', ['a', 'b']), task('d', ['c']));
    assert.deepEqual(await execute(input, { run(t) { return t.id === 'b' ? Promise.reject(7) : undefined; } }),
      result('failed', ['a'], ['b'], ['c', 'd']));
  });
  for (const [name, input] of [
    ['cycle', doc(task('a', ['b']), task('b', ['a']))],
    ['missing dependency', doc(task('a', ['b']))], ['duplicate ID', doc(task('a'), task('a'))],
    ['unsafe total', doc(task('a', [], Number.MAX_SAFE_INTEGER), task('b'))],
    ['extra task field', doc({ ...task('a'), command: 'no' })],
  ]) await check(`validation before effects: ${name}`, async () => {
    let effects = 0;
    await assert.rejects(async () => execute(input, { run() { effects++; }, checkpoint: {
      load() { effects++; return null; }, save() { effects++; },
    } })); assert.equal(effects, 0);
  });
  for (const [name, options] of [
    ['missing run', {}], ['null options', null], ['extra option', { run() {}, extra: true }],
    ...[0, -1, 33, 1.5, '2', NaN].map(value => [`concurrency ${String(value)}`, { run() {}, concurrency: value }]),
    ['invalid signal', { run() {}, signal: {} }], ['invalid adapter', { run() {}, checkpoint: { load() {} } }],
  ]) await check(`invalid options: ${name}`, async () => assert.rejects(async () => execute(doc(task('a')), options)));
  await check('frozen null-prototype non-enumerable input', async () => {
    const hidden = Object.freeze(Object.defineProperties(Object.create(null), {
      id: { value: 'b' }, deps: { value: Object.freeze(['a']) }, duration: { value: 2 },
    }));
    const input = Object.freeze({ tasks: Object.freeze([Object.freeze(task('a')), hidden]) });
    const snapshots = [];
    assert.deepEqual(await execute(input, Object.freeze({ run(t) { assert.equal(t.duration, t.id === 'b' ? 2 : 1); },
      checkpoint: { load: () => null, save: s => snapshots.push(s) },
    })), result('completed', ['a', 'b']));
    assert.deepEqual(snapshots.at(-1), snapshot(input, ['a', 'b']));
  });
  await check('definition captured before asynchronous load', async () => {
    const gate = defer(), input = doc(task('a'), task('b', ['a'])), seen = [];
    const running = observe(execute(input, { run(t) { seen.push(t.id); }, checkpoint: {
      load: () => gate.promise, save() {},
    } }));
    input.tasks[0].id = 'changed'; input.tasks[1].deps.length = 0; input.tasks.push(task('extra'));
    gate.resolve(null); assert.deepEqual(await running, result('completed', ['a', 'b']));
    assert.deepEqual(seen, ['a', 'b']);
  });
  await check('checkpoint initial commit before dispatch and dependency commit barrier', async () => {
    const gates = [defer(), defer(), defer()], saves = [], started = [];
    const running = observe(execute(doc(task('a'), task('b', ['a'])), { run(t) { started.push(t.id); },
      checkpoint: { load: () => null, save(s) { saves.push(s); return gates[saves.length - 1].promise; } },
    }));
    try {
      await until(() => saves.length === 1); assert.deepEqual(started, []);
      gates[0].resolve(); await until(() => saves.length === 2); assert.deepEqual(started, ['a']);
      await tick(); assert.deepEqual(started, ['a']);
      gates[1].resolve(); await until(() => saves.length === 3); assert.deepEqual(started, ['a', 'b']);
    } finally { gates.forEach(g => g.resolve()); }
    await running; assert.deepEqual(saves.map(s => s.completed), [[], ['a'], ['a', 'b']]);
  });
  await check('serialized detached snapshots and canonical order', async () => {
    const input = doc(task('z'), task('c', ['z', 'a']), task('a')); const saved = [];
    let active = 0, peak = 0, loads = 0;
    await execute(input, { run(t) { if (!Object.isFrozen(t.deps)) t.deps.push('poison'); }, checkpoint: {
      load() { loads++; return null; }, async save(s) { active++; peak = Math.max(peak, active); saved.push(s); await tick(); active--; },
    } });
    assert.equal(loads, 1); assert.equal(peak, 1); assert.equal(saved.length, 4);
    assert.deepEqual(saved[0], snapshot(input));
    assert.deepEqual(saved.at(-1), snapshot(input, ['a', 'c', 'z']));
    assert.deepEqual(saved[1].completed, ['a']);
    assert.notEqual(saved[0].tasks, saved[1].tasks);
  });
  await check('resume skips successes and retries failures', async () => {
    const input = doc(task('a'), task('b', ['a']), task('c')); let state = null;
    const checkpoint = { load: () => state, save: s => { state = structuredClone(s); } };
    assert.deepEqual(await execute(input, { checkpoint, run(t) { if (t.id === 'b') throw new Error('retry'); } }),
      result('failed', ['a', 'c'], ['b']));
    const seen = []; assert.deepEqual(await execute(input, { checkpoint, run(t) { seen.push(t.id); } }),
      result('completed', ['a', 'b', 'c'])); assert.deepEqual(seen, ['b']);
  });
  await check('equivalent input reorder resumes without replay', async () => {
    const input = doc(task('c', ['b', 'a']), task('b'), task('a'));
    assert.deepEqual(await execute(doc(task('a'), task('b'), task('c', ['a', 'b'])), {
      run() { assert.fail('already done'); }, checkpoint: { load: () => snapshot(input, ['a', 'b', 'c']), save() {} },
    }), result('completed', ['a', 'b', 'c']));
  });
  const graph = doc(task('a'), task('b', ['a']));
  for (const [name, value] of [
    ['undefined', undefined], ['unknown version', { ...snapshot(graph), version: 2 }],
    ['unknown completed', snapshot(graph, ['x'])], ['duplicate completed', snapshot(graph, ['a', 'a'])],
    ['dependency omitted', snapshot(graph, ['b'])], ['different duration', snapshot(doc(task('a', [], 9), task('b', ['a'])))],
    ['different edge', snapshot(doc(task('a'), task('b')))], ['extra field', { ...snapshot(graph), extra: true }],
    ['unsorted task records', { ...snapshot(graph), tasks: [...graph.tasks].reverse() }],
  ]) await check(`reject invalid checkpoint before dispatch: ${name}`, async () => {
    let effects = 0;
    await assert.rejects(async () => execute(graph, { run() { effects++; }, checkpoint: { load: () => value, save() { effects++; } } }));
    assert.equal(effects, 0);
  });
  await check('pre-abort starts no workers', async () => {
    assert.deepEqual(await execute(graph, { signal: AbortSignal.abort('stop'), run() { assert.fail('aborted'); } }),
      result('cancelled', [], [], [], ['a', 'b']));
  });
  await check('active abort quiesces; success saved and aborted rejection remains pending', async () => {
    const control = new AbortController(), first = defer(), second = defer(), signals = [], saved = [];
    let finished = false;
    const input = doc(task('a'), task('b'), task('c', ['a']));
    const running = observe(execute(input, { signal: control.signal,
      run(t, { signal }) { signals.push(signal); return t.id === 'a' ? first.promise : second.promise; },
      checkpoint: { load: () => null, save: s => saved.push(s) },
    })); running.then(() => { finished = true; }, () => { finished = true; });
    try {
      await until(() => signals.length === 2); control.abort(); await tick();
      assert.ok(signals.every(s => s.aborted)); assert.equal(finished, false);
    } finally { first.resolve(); second.reject(new Error('cancelled worker')); }
    assert.deepEqual(await running, result('cancelled', ['a'], [], [], ['b', 'c']));
    assert.deepEqual(saved.at(-1).completed, ['a']);
  });
  await check('failure before cancellation still blocks descendants', async () => {
    const control = new AbortController(), gate = defer(); let started = false;
    const input = doc(task('a'), task('b', ['a']), task('c'));
    const running = observe(execute(input, { signal: control.signal, run(t) {
      if (t.id === 'a') throw new Error('failure'); started = true; return gate.promise;
    } }));
    try { await until(() => started); await tick(); control.abort(); }
    finally { gate.reject(new Error('abort')); }
    assert.deepEqual(await running, result('cancelled', [], ['a'], ['b'], ['c']));
  });
  await check('load and initial save failures preserve cause and have no workers', async () => {
    const failure = new Error('storage');
    for (const checkpoint of [ { load() { throw failure; }, save() {} },
      { load: () => null, save() { throw failure; } } ]) {
      await assert.rejects(async () => execute(graph, { checkpoint, run() { assert.fail('storage failed'); } }),
        error => error === failure || error.cause === failure);
    }
  });
  await check('failed save stops dispatch and waits for active workers without later writes', async () => {
    const gate = defer(), failure = new Error('save failed'), signals = [], started = [];
    let saves = 0, finished = false;
    const running = observe(execute(doc(task('a'), task('b'), task('c', ['a'])), {
      run(t, { signal }) { started.push(t.id); signals.push(signal); return t.id === 'b' ? gate.promise : undefined; },
      checkpoint: { load: () => null, save() { if (++saves === 2) throw failure; } },
    })); running.then(() => { finished = true; }, () => { finished = true; });
    try {
      await until(() => saves === 2); await tick(); assert.equal(finished, false);
      assert.ok(signals.every(s => s.aborted)); assert.deepEqual(started, ['a', 'b']);
    } finally { gate.resolve(); }
    await assert.rejects(running, error => error === failure || error.cause === failure);
    await tick(); assert.equal(saves, 2); assert.deepEqual(started, ['a', 'b']);
  });
  await check('deep graph avoids recursion overflow', async () => {
    const input = doc(...Array.from({ length: 8000 }, (_, i) => task(`n${i}`, i ? [`n${i - 1}`] : [])));
    let count = 0; const value = await execute(input, { run() { count++; } });
    assert.equal(count, 8000); assert.equal(value.completed.length, 8000); assert.equal(value.status, 'completed');
  });
  for (let seed = 1; seed <= 24; seed++) await check(`seeded DAG failure oracle ${seed}`, async () => {
    let state = seed; const random = () => ((state = (state * 1664525 + 1013904223) >>> 0) / 2 ** 32);
    const tasks = Array.from({ length: 25 }, (_, i) => task(`n${i}`,
      Array.from({ length: i }, (_, j) => `n${j}`).filter(() => random() < .08)));
    const failures = new Set(tasks.filter(() => random() < .15).map(t => t.id));
    const done = new Set(), failed = new Set(), blocked = new Set(), seen = new Set();
    for (const t of tasks) {
      if (t.deps.some(id => failed.has(id) || blocked.has(id))) blocked.add(t.id);
      else (failures.has(t.id) ? failed : done).add(t.id);
    }
    const value = await execute({ tasks }, { concurrency: 4, async run(t) {
      assert.ok(!seen.has(t.id)); seen.add(t.id); await tick(); if (failures.has(t.id)) throw new Error('seeded');
    } });
    assert.deepEqual(value, result(failed.size ? 'failed' : 'completed', [...done], [...failed], [...blocked]));
  });
  await fileChecks(store, execute, check);
}

async function fileChecks(store, execute, check) {
  const folder = await mkdtemp(join(tmpdir(), 'durable-evaluator-'));
  try {
    const input = doc(task('a'), task('b', ['a'])), file = join(folder, 'state.json');
    await check('file missing, valid save and detached loads', async () => {
      const adapter = store(file); assert.equal(await adapter.load(), null);
      await adapter.save(snapshot(input, ['a']));
      const first = await adapter.load(); first.completed.length = 0;
      assert.deepEqual(await adapter.load(), snapshot(input, ['a']));
      assert.ok((await readFile(file, 'utf8')).endsWith('\n'));
    });
    await check('file save captures input before await and queues in invocation order', async () => {
      const adapter = store(file), first = snapshot(input), second = snapshot(input, ['a']);
      const a = observe(adapter.save(first)), b = observe(adapter.save(second));
      first.tasks[0].id = 'poison'; second.completed.push('poison');
      await Promise.all([a, b]); assert.deepEqual(await adapter.load(), snapshot(input, ['a']));
    });
    await check('invalid save preserves previous bytes and does not poison valid saves', async () => {
      const adapter = store(file), before = await readFile(file);
      await assert.rejects(async () => adapter.save({ version: 99 }));
      assert.deepEqual(await readFile(file), before);
      await adapter.save(snapshot(input, ['a', 'b']));
      assert.deepEqual(await adapter.load(), snapshot(input, ['a', 'b']));
      assert.deepEqual(await readdir(folder), ['state.json']);
    });
    for (const [name, contents] of [ ['malformed JSON', '{'], ['invalid snapshot', '{"version":1}'],
      ['oversized file', ' '.repeat(1024 * 1024 + 1)] ]) await check(`file load rejects ${name}`, async () => {
      const target = join(folder, `bad-${name.replaceAll(' ', '-')}.json`); await writeFile(target, contents);
      await assert.rejects(async () => store(target).load());
    });
    await check('file adapter refuses directory and target symlink', async () => {
      const directory = join(folder, 'directory'); await mkdir(directory);
      const link = join(folder, 'link.json'); await symlink(file, link);
      const before = await readFile(file);
      for (const target of [directory, link]) {
        await assert.rejects(async () => store(target).load());
        await assert.rejects(async () => store(target).save(snapshot(input)));
      }
      assert.deepEqual(await readFile(file), before);
    });
    await check('missing parent is not created', async () => {
      const target = join(folder, 'missing', 'state.json');
      await assert.rejects(async () => store(target).save(snapshot(input)));
      assert.ok(!(await readdir(folder)).includes('missing'));
    });
    await check('oversized save preserves last valid checkpoint', async () => {
      const adapter = store(file), before = await readFile(file);
      const large = { tasks: Array.from({ length: 15000 }, (_, i) => task(`id${i}_${'x'.repeat(40)}`)) };
      await assert.rejects(async () => adapter.save(snapshot(large)));
      assert.deepEqual(await readFile(file), before);
    });
    await check('real file restart replays no persisted successes', async () => {
      await writeFile(file, JSON.stringify(snapshot(input, ['a']))+'\n');
      const calls = []; const value = await execute(input, { checkpoint: store(file), run(t) { calls.push(t.id); } });
      assert.deepEqual(value, result('completed', ['a', 'b'])); assert.deepEqual(calls, ['b']);
      assert.deepEqual(await store(file).load(), snapshot(input, ['a', 'b']));
    });
  } finally { await rm(folder, { recursive: true, force: true }); }
}
