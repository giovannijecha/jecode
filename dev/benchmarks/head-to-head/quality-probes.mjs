// Supplementary checks: never replace the frozen acceptance scores or timings.
import assert from 'node:assert/strict';
import { getEventListeners } from 'node:events';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [task, directory] = process.argv.slice(2);
if (!['cache', 'planner'].includes(task) || !directory) throw new Error('expected cache|planner WORKSPACE');
const root = resolve(directory);
const results = [];
const tick = () => new Promise(resolve => setImmediate(resolve));
function random(seed) {
  let state = seed >>> 0;
  return limit => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state % limit;
  };
}
async function check(name, run, origin = 'contract-derived') {
  let timer;
  try {
    await Promise.race([Promise.resolve().then(run), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('probe timed out')), 3000);
    })]);
    results.push({ name, origin, passed: true });
  } catch (error) {
    results.push({ name, origin, passed: false, error: String(error.message).slice(0, 500) });
  } finally { clearTimeout(timer); }
}

async function cacheSchedule(createCache, seed) {
  const choose = random(seed);
  const ttlMs = [0, 0.5, 7][choose(3)];
  const keys = ['', '__proto__', 'constructor', 'A', 'b'];
  const entries = new Map();
  const generations = [], loads = [], callers = [];
  let clock = 0;
  const cache = createCache(key => {
    const load = Promise.withResolvers();
    loads.push({ key, ...load });
    return load.promise;
  }, { ttlMs, now: () => clock });

  function settle(index, failure) {
    const generation = generations[index];
    const value = failure ? { failure: index } : [undefined, null, false, 0, '', index][choose(6)];
    generation.pending = false;
    generation.result = failure ? { error: value } : { value };
    if (entries.get(generation.key) === generation) {
      if (failure || ttlMs === 0) entries.delete(generation.key);
      else generation.expires = clock + ttlMs;
    }
    loads[index][failure ? 'reject' : 'resolve'](value);
  }

  async function verify() {
    await tick();
    assert.deepEqual(loads.map(load => load.key), generations.map(generation => generation.key));
    for (const caller of callers) {
      const expected = caller.cancelled ? { error: caller.reason }
        : caller.generation?.result ?? caller.immediate ?? null;
      assert.deepEqual(caller.actual, expected);
      assert.equal(getEventListeners(caller.controller.signal, 'abort').length, expected === null ? 1 : 0);
    }
  }

  for (let step = 0; step < 80; step++) {
    const action = choose(8), key = keys[choose(keys.length)];
    if (action < 3) {
      const controller = new AbortController(), preaborted = choose(7) === 0;
      const caller = { controller, reason: { seed, step }, actual: null, cancelled: preaborted };
      if (preaborted) controller.abort(caller.reason);
      else {
        let generation = entries.get(key);
        if (generation && !generation.pending && clock >= generation.expires) {
          entries.delete(key); generation = undefined;
        }
        if (generation && !generation.pending) caller.immediate = generation.result;
        else {
          if (!generation) {
            generation = { key, pending: true };
            generations.push(generation); entries.set(key, generation);
          }
          caller.generation = generation;
        }
      }
      callers.push(caller);
      // Observe both outcomes at creation; a broken implementation must not crash the scorer.
      cache.get(key, { signal: controller.signal }).then(
        value => { caller.actual = { value }; }, error => { caller.actual = { error }; },
      );
    } else if (action === 3) {
      cache.invalidate(key); entries.delete(key);
    } else if (action === 4) {
      cache.clear(); entries.clear();
    } else if (action === 5) {
      clock += choose(12) / 2;
    } else if (action === 6) {
      const pending = callers.filter(caller => caller.actual === null);
      if (pending.length) {
        const caller = pending[choose(pending.length)];
        caller.cancelled = true; caller.controller.abort(caller.reason);
      }
    } else {
      const pending = generations.flatMap((generation, index) => generation.pending ? [index] : []);
      if (pending.length) settle(pending[choose(pending.length)], choose(3) === 0);
    }
    await verify();
  }
  for (let index = generations.length - 1; index >= 0; index--) {
    if (generations[index].pending) settle(index, index % 3 === 0);
  }
  await verify();
}

// A deliberately simple scanning oracle; it shares no heap or adjacency code.
function reference(tasks, targets) {
  const included = new Set(targets ?? tasks.map(task => task.id));
  let changed = true;
  while (changed) {
    changed = false;
    for (const task of tasks) if (included.has(task.id)) for (const id of task.deps) {
      if (!included.has(id)) { included.add(id); changed = true; }
    }
  }
  const order = [], waves = [], depths = new Map(), finishes = new Map();
  let totalDuration = 0;
  while (order.length < included.size) {
    const next = tasks.filter(task => included.has(task.id) && !depths.has(task.id)
      && task.deps.every(id => depths.has(id))).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0)[0];
    assert.ok(next, 'oracle input must be a DAG');
    const depth = Math.max(-1, ...next.deps.map(id => depths.get(id))) + 1;
    depths.set(next.id, depth); (waves[depth] ??= []).push(next.id);
    finishes.set(next.id, Math.max(0, ...next.deps.map(id => finishes.get(id))) + next.duration);
    order.push(next.id); totalDuration += next.duration;
  }
  return { order, waves: waves.map(wave => wave.sort()), totalDuration,
    criticalPathDuration: Math.max(0, ...finishes.values()) };
}

if (task === 'cache') {
  const { createCache } = await import(pathToFileURL(join(root, 'src/cache.js')));
  for (let seed = 1; seed <= 128; seed++) {
    await check(`interleaved generations seed ${seed}`, () => cacheSchedule(createCache, seed));
  }
} else {
  const { planBuild } = await import(pathToFileURL(join(root, 'src/index.js')));
  for (let seed = 1; seed <= 256; seed++) await check(`DAG and selection seed ${seed}`, () => {
    const choose = random(seed), count = 1 + choose(18);
    const ids = ['__proto__', 'constructor', 'Z', 'a', 'A', '0', '_', '-'];
    const tasks = [];
    for (let index = 0; index < count; index++) tasks.push({
      id: ids[index] ?? `n${index}`, deps: tasks.filter(() => choose(4) === 0).map(task => task.id),
      duration: choose(19),
    });
    for (let index = tasks.length - 1; index > 0; index--) {
      const other = choose(index + 1); [tasks[index], tasks[other]] = [tasks[other], tasks[index]];
    }
    const targets = tasks.filter(() => choose(3) === 0).map(task => task.id);
    const options = targets.length ? { targets } : {};
    const before = JSON.stringify({ tasks, options });
    for (const task of tasks) { Object.freeze(task.deps); Object.freeze(task); }
    Object.freeze(tasks); if (options.targets) Object.freeze(options.targets); Object.freeze(options);
    assert.deepEqual(planBuild(Object.freeze({ tasks }), options), reference(tasks, options.targets));
    assert.equal(JSON.stringify({ tasks, options }), before);
  });
  for (const suffix of ['\n', '\r', '\u2028', '\u2029', '\0', 'é']) {
    await check(`reject non-ASCII ID suffix ${JSON.stringify(suffix)}`, () => {
      assert.throws(() => planBuild({ tasks: [{ id: `a${suffix}`, deps: [], duration: 0 }] }));
    });
  }
  const cli = args => {
    const result = spawnSync(process.execPath, [join(root, 'src/cli.js'), ...args], {
      cwd: root, encoding: 'utf8', timeout: 3000, maxBuffer: 1024 * 1024,
    });
    if (result.error) throw result.error;
    assert.equal(result.status, 1); assert.equal(result.stdout, '');
    return result.stderr;
  };
  await check('CLI unknown flag cannot inject terminal controls', () => {
    assert.doesNotMatch(cli(['--bad\x1b[2J']), /[\x00-\x09\x0b-\x1f\x7f-\x9f]/);
  }, 'review-derived hardening');
  await check('CLI unknown flag remains concise', () => {
    assert.ok(cli(['--' + 'x'.repeat(8192)]).length <= 1024);
  }, 'review-derived hardening');
}
console.log(JSON.stringify({ task, supplementary: true, total: results.length,
  passed: results.filter(result => result.passed).length, results }, null, 2));
process.exitCode = results.every(result => result.passed) ? 0 : 1;
