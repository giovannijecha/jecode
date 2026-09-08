// Frozen before follow-up trials; the original planner scorer is also mandatory.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(process.argv[2]);
const { planBuild } = await import(pathToFileURL(join(root, 'src/index.js')));
const results = [];
function check(name, run) {
  try { run(); results.push({ name, passed: true }); }
  catch (error) { results.push({ name, passed: false, error: String(error.message).slice(0, 500) }); }
}
const original = spawnSync(process.execPath, [fileURLToPath(new URL('../planner/acceptance.mjs', import.meta.url)), root],
  { encoding: 'utf8', timeout: 20000, maxBuffer: 1024 * 1024 });
check('original planner acceptance is preserved', () => {
  if (original.error) throw original.error;
  assert.equal(original.status, 0, original.stderr || original.stdout.slice(-500));
  const score = JSON.parse(original.stdout); assert.equal(score.passed, score.total); assert.equal(score.total, 57);
});

const task = (id, deps = [], duration = 1) => ({ id, deps, duration });
const document = { tasks: [task('a', [], 2), task('b', ['a'], 3), task('c', ['a'], 5), task('d', ['b', 'c'], 7)] };
const empty = { order: [], waves: [], totalDuration: 0, criticalPathDuration: 0 };
const remaining = { order: ['a', 'c', 'd'], waves: [['a'], ['c'], ['d']], totalDuration: 14, criticalPathDuration: 14 };
check('completed branch cuts traversal without hiding shared prerequisites', () => {
  assert.deepEqual(planBuild(document, { targets: ['d'], completed: ['b'] }), remaining);
});
check('completed target cuts its prerequisites', () => {
  assert.deepEqual(planBuild(document, { targets: ['b'], completed: ['b'] }), empty);
});
check('independently requested prerequisite remains', () => {
  assert.deepEqual(planBuild(document, { targets: ['a', 'b'], completed: ['b'] }),
    { order: ['a'], waves: [['a']], totalDuration: 2, criticalPathDuration: 2 });
});
check('no targets starts from all unfinished tasks', () => {
  assert.deepEqual(planBuild(document, { completed: ['b'] }), remaining);
});
check('all completed and empty documents return empty plans', () => {
  assert.deepEqual(planBuild(document, { completed: ['a', 'b', 'c', 'd'] }), empty);
  assert.deepEqual(planBuild({ tasks: [] }, { completed: [] }), empty);
});
check('empty completed is equivalent to omission', () => {
  assert.deepEqual(planBuild(document, { completed: [] }), planBuild(document));
});
check('completed outside selected work does not affect the result', () => {
  assert.deepEqual(planBuild(document, { targets: ['a'], completed: ['c'] }), planBuild(document, { targets: ['a'] }));
});
for (const completed of [undefined, null, false, 'a', {}, [1], ['missing'], ['a', 'a']]) {
  check(`invalid completed ${JSON.stringify(completed)}`, () => assert.throws(() => planBuild(document, { completed })));
}
for (const tasks of [
  [task('ok'), task('x', ['y']), task('y', ['x'])],
  [task('ok'), task('x', ['missing'])],
  [task('ok'), { ...task('x'), command: 'forbidden' }],
]) check('completed invalid component is still validated', () => {
  assert.throws(() => planBuild({ tasks }, { targets: ['ok'], completed: ['x'] }));
});
check('completed durations cannot overflow remaining aggregates', () => {
  const tasks = [task('a', [], Number.MAX_SAFE_INTEGER), task('b', ['a'], 1)];
  assert.deepEqual(planBuild({ tasks }, { completed: ['a'] }),
    { order: ['b'], waves: [['b']], totalDuration: 1, criticalPathDuration: 1 });
  assert.throws(() => planBuild({ tasks }, { completed: [] }));
});
check('API allows special and reserved IDs', () => {
  const ids = ['__proto__', 'constructor', '--help', '--target', '--completed'];
  assert.deepEqual(planBuild({ tasks: ids.map(id => task(id)) }, { completed: ids }), empty);
});
check('deep completed boundary has no recursion overflow', () => {
  const tasks = Array.from({ length: 14000 }, (_, i) => task(`n${i}`, i ? [`n${i - 1}`] : []));
  const result = planBuild({ tasks }, { targets: ['n13999'], completed: ['n7000'] });
  assert.equal(result.order.length, 6999); assert.equal(result.order[0], 'n7001');
  assert.equal(result.criticalPathDuration, 6999);
});

// The small oracle uses repeated scans, not the implementation's graph helpers.
function reference(tasks, targets, completed) {
  const done = new Set(completed), included = new Set((targets ?? tasks.map(t => t.id)).filter(id => !done.has(id)));
  let grew = true;
  while (grew) {
    grew = false;
    for (const t of tasks) if (included.has(t.id)) for (const id of t.deps) {
      if (!done.has(id) && !included.has(id)) { included.add(id); grew = true; }
    }
  }
  const order = [], waves = [], depth = new Map(), finish = new Map();
  let totalDuration = 0;
  while (order.length < included.size) {
    const next = tasks.filter(t => included.has(t.id) && !depth.has(t.id)
      && t.deps.every(id => done.has(id) || depth.has(id))).sort((a, b) => a.id < b.id ? -1 : 1)[0];
    assert.ok(next);
    const dependencies = next.deps.filter(id => !done.has(id));
    const level = Math.max(-1, ...dependencies.map(id => depth.get(id))) + 1;
    depth.set(next.id, level); (waves[level] ??= []).push(next.id);
    finish.set(next.id, next.duration + Math.max(0, ...dependencies.map(id => finish.get(id))));
    order.push(next.id); totalDuration += next.duration;
  }
  return { order, waves: waves.map(wave => wave.sort()), totalDuration,
    criticalPathDuration: Math.max(0, ...finish.values()) };
}
for (let seed = 1; seed <= 128; seed++) check(`completed DAG seed ${seed}`, () => {
  let state = seed;
  const choose = n => { state = (Math.imul(state, 1664525) + 1013904223) >>> 0; return (state >>> 8) % n; };
  const tasks = [];
  for (let i = 0, count = 1 + choose(14); i < count; i++) {
    tasks.push(task(`n${i}`, tasks.filter(() => choose(3) === 0).map(t => t.id), choose(19)));
  }
  const completed = tasks.filter(() => choose(3) === 0).map(t => t.id);
  const targets = tasks.filter(() => choose(3) === 0).map(t => t.id);
  const options = targets.length ? { targets, completed } : { completed };
  const before = JSON.stringify({ tasks, options });
  for (const t of tasks) { Object.freeze(t.deps); Object.freeze(t); }
  Object.freeze(tasks); Object.freeze(completed); Object.freeze(targets); Object.freeze(options);
  assert.deepEqual(planBuild(Object.freeze({ tasks }), options), reference(tasks, options.targets, completed));
  assert.equal(JSON.stringify({ tasks, options }), before);
});
const temporary = mkdtempSync(join(tmpdir(), 'planner-progress-'));
try {
  const file = join(temporary, 'input.json'); writeFileSync(file, JSON.stringify(document));
  const cli = args => {
    const result = spawnSync(process.execPath, [join(root, 'src/cli.js'), ...args],
      { encoding: 'utf8', timeout: 3000, maxBuffer: 1024 * 1024 });
    if (result.error) throw result.error;
    return result;
  };
  for (const args of [
    [file, '--target', 'd', '--completed', 'b'], ['--completed', 'b', file, '--target', 'd'],
    ['--target', 'd', '--completed', 'b', file],
  ]) check('CLI option order', () => {
    const result = cli(args); assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, ''); assert.deepEqual(JSON.parse(result.stdout), remaining); assert.ok(result.stdout.endsWith('\n'));
  });
  for (const args of [
    [file, '--completed'], [file, '--completed', 'b', '--completed', 'b'],
    [file, '--completed', 'missing'], [file, '--completed', 'b', '--help'],
    [file, '--bad\x1b[2J'], [file, '--' + 'x'.repeat(8192)],
  ]) check('CLI invalid input is bounded and terminal-safe', () => {
    const result = cli(args); assert.equal(result.status, 1); assert.equal(result.stdout, '');
    assert.ok(result.stderr.trim()); assert.ok(result.stderr.length <= 1024); assert.ok(result.stderr.endsWith('\n'));
    assert.doesNotMatch(result.stderr.slice(0, -1), /[\x00-\x1f\x7f-\x9f\u2028\u2029]/);
  });
  check('CLI help documents completed tasks', () => {
    const result = cli(['--help']); assert.equal(result.status, 0); assert.match(result.stdout, /--completed/);
  });
} finally { rmSync(temporary, { recursive: true, force: true }); }
console.log(JSON.stringify({ total: results.length, passed: results.filter(row => row.passed).length, results }, null, 2));
process.exitCode = results.every(row => row.passed) ? 0 : 1;
