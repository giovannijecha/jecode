import assert from 'node:assert/strict';
import * as fs from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = resolve(process.argv[2]);
const { parsePointer, patchDocument, migrateFiles } = await import(pathToFileURL(join(root, 'src/index.js')));
const results = [];
async function check(name, run) {
  try { await run(); results.push({ name, passed: true }); }
  catch (error) { results.push({ name, passed: false, error: String(error?.message ?? error).slice(0, 400) }); }
}
const set = (path, value) => ({ op: 'set', path, value });
const change = (path, value = 2) => ({ path, operations: [set('/n', value)] });
await check('pointer decoding is one pass', () => {
  assert.deepEqual(parsePointer('/a~1b/~01//~0'), ['a/b', '~1', '', '~']);
  assert.deepEqual(parsePointer(''), []);
});
for (const pointer of [null, 2, 'x', '/~', '/~2', '/a~9b', '/__proto__', '/x/constructor', '/prototype']) {
  await check(`invalid pointer ${JSON.stringify(pointer)}`, () => assert.throws(() => parsePointer(pointer)));
}
await check('ordered changes, splice and append', () => {
  const doc = { a: [1, 2, 3], keep: { value: true } };
  const ops = [set('/a/1', 7), { op: 'remove', path: '/a/0' }, set('/a/2', 9), set('/a/-', null)];
  const before = structuredClone([doc, ops]);
  assert.deepEqual(patchDocument(doc, ops), { a: [7, 3, 9, null], keep: { value: true } });
  assert.deepEqual([doc, ops], before);
});
await check('deep detachment from document and operation value', () => {
  const doc = { keep: { n: 1 } }, value = { nested: [2] };
  const result = patchDocument(doc, [set('/added', value)]);
  result.keep.n = 3; result.added.nested.push(4);
  assert.deepEqual(doc, { keep: { n: 1 } }); assert.deepEqual(value, { nested: [2] });
});
await check('root replacement and structural tests', () => {
  assert.deepEqual(patchDocument(null, [set('', { a: 1, b: [false] }),
    { op: 'test', path: '', value: { b: [false], a: 1 } }]), { a: 1, b: [false] });
  assert.equal(patchDocument(2, [{ op: 'test', path: '', value: 2 }]), 2);
});
for (const op of [set('/missing/leaf', 1), { op: 'remove', path: '/absent' },
  { op: 'remove', path: '' }, { op: 'test', path: '/n', value: 2 }, { op: 'copy', path: '/n' },
  { op: 'set', path: '/n' }, { op: 'test', path: '/n' }, { ...set('/n', 1), extra: true }]) {
  await check(`reject operation ${JSON.stringify(op)}`, () => {
    const doc = { n: 1 }; assert.throws(() => patchDocument(doc, [op])); assert.deepEqual(doc, { n: 1 });
  });
}
for (const path of ['/a/01', '/a/-1', '/a/3', '/a/1.0', '/a/1e0']) {
  await check(`reject array path ${path}`, () => assert.throws(() => patchDocument({ a: [1] }, [set(path, 4)])));
}
for (const value of [undefined, NaN, Infinity, 1n, () => {}, new Date(), [1, , 2], { x: undefined },
  JSON.parse('{"__proto__":{"polluted":true}}')]) {
  await check(`reject non-JSON ${String(value)}`, () => {
    assert.throws(() => patchDocument(value, []));
    assert.throws(() => patchDocument({}, [set('/x', value)]));
  });
}
await check('reject cycles', () => { const value = {}; value.self = value; assert.throws(() => patchDocument(value, [])); });
await check('failure after an earlier operation never mutates inputs', () => {
  const doc = { n: 1 };
  assert.throws(() => patchDocument(doc, [set('/n', 9), { op: 'test', path: '/n', value: 2 }]));
  assert.deepEqual(doc, { n: 1 });
});
async function fixture(run) {
  const base = await fs.mkdtemp(join(tmpdir(), 'config-eval-'));
  const area = join(base, 'root'); await fs.mkdir(area);
  await fs.writeFile(join(area, 'a.json'), '{"n":1}\n');
  await fs.writeFile(join(area, 'b.json'), '\uFEFF{ "n" : 1 }\n');
  const original = await Promise.all(['a.json', 'b.json'].map(name => fs.readFile(join(area, name))));
  try { await run(area, base, original); }
  finally { await fs.rm(base, { recursive: true, force: true }); }
}
const bytes = area => Promise.all(['a.json', 'b.json'].map(name => fs.readFile(join(area, name))));
const names = async area => (await fs.readdir(area)).sort();
await check('empty migration', () => fixture(async area => assert.deepEqual(await migrateFiles(area, []), { changed: [] })));
await check('BOM, permissions, unchanged bytes and ordered report', () => fixture(async area => {
  await fs.chmod(join(area, 'a.json'), 0o640);
  const before = await fs.readFile(join(area, 'a.json'));
  assert.deepEqual(await migrateFiles(area, [change('b.json'), change('a.json', 1)]), { changed: ['b.json'] });
  assert.deepEqual(await fs.readFile(join(area, 'a.json')), before);
  assert.equal(await fs.readFile(join(area, 'b.json'), 'utf8'), '\uFEFF{\n  "n": 2\n}\n');
  assert.deepEqual(await migrateFiles(area, [change('a.json')]), { changed: ['a.json'] });
  if (process.platform !== 'win32') assert.equal((await fs.stat(join(area, 'a.json'))).mode & 0o777, 0o640);
  assert.deepEqual(await names(area), ['a.json', 'b.json']);
}));
await check('dry run performs validation but no changes or callbacks', () => fixture(async (area, _, original) => {
  assert.deepEqual(await migrateFiles(area, [change('a.json'), change('b.json')], {
    dryRun: true, onCommit() { assert.fail('dry run committed'); },
  }), { changed: ['a.json', 'b.json'] });
  assert.deepEqual(await bytes(area), original); assert.deepEqual(await names(area), ['a.json', 'b.json']);
}));
for (const entries of [[change('a.json'), change('b.json', undefined), { path: 'missing', operations: [] }],
  [change('a.json'), change('./a.json')], [change('a.json'), change('../outside.json')],
  [change('a.json'), change('nested/../b.json')]]) {
  await check(`validate all before write ${JSON.stringify(entries)}`, () => fixture(async (area, _, original) => {
    await assert.rejects(migrateFiles(area, entries)); assert.deepEqual(await bytes(area), original);
  }));
}
await check('absolute paths rejected even within root', () => fixture(async area => {
  await assert.rejects(migrateFiles(area, [change(join(area, 'a.json'))]));
}));
await check('malformed later JSON preserves earlier target', () => fixture(async (area, _, original) => {
  await fs.writeFile(join(area, 'b.json'), '{broken');
  await assert.rejects(migrateFiles(area, [change('a.json'), change('b.json')]));
  assert.deepEqual(await fs.readFile(join(area, 'a.json')), original[0]);
}));
for (const reason of [new Error('callback'), 'callback-string', null]) {
  await check(`rollback thrown ${String(reason)}`, () => fixture(async (area, _, original) => {
    const commits = [];
    await assert.rejects(migrateFiles(area, [change('a.json'), change('b.json')], {
      async onCommit(name, index) { commits.push([name, index]); if (index === 1) throw reason; },
    }));
    assert.deepEqual(commits, [['a.json', 0], ['b.json', 1]]);
    assert.deepEqual(await bytes(area), original); assert.deepEqual(await names(area), ['a.json', 'b.json']);
  }));
}
for (const during of [false, true]) {
  await check(`cancel ${during ? 'during' : 'before'} commits`, () => fixture(async (area, _, original) => {
    const control = new AbortController(); if (!during) control.abort();
    await assert.rejects(migrateFiles(area, [change('a.json'), change('b.json')], {
      signal: control.signal, onCommit() { control.abort(); },
    }));
    assert.deepEqual(await bytes(area), original); assert.deepEqual(await names(area), ['a.json', 'b.json']);
  }));
}
await check('symlink target and ancestor are rejected', () => fixture(async (area, base) => {
  await fs.mkdir(join(base, 'outside')); await fs.writeFile(join(base, 'outside', 'x.json'), '{"n":1}');
  await fs.symlink(join(base, 'outside', 'x.json'), join(area, 'link.json'));
  await fs.symlink(join(base, 'outside'), join(area, 'dir'), 'dir');
  await assert.rejects(migrateFiles(area, [change('link.json')]));
  await assert.rejects(migrateFiles(area, [change('dir/x.json')]));
  assert.equal(await fs.readFile(join(base, 'outside', 'x.json'), 'utf8'), '{"n":1}');
}));
await check('CLI dry run, commit and invalid usage', () => fixture(async (area, base, original) => {
  const manifest = join(base, 'manifest.json'); await fs.writeFile(manifest, JSON.stringify([change('a.json')]));
  const cli = (...args) => spawnSync(process.execPath, [join(root, 'src/cli.js'), ...args], { encoding: 'utf8', timeout: 3000 });
  const dry = cli(area, manifest, '--dry-run'); assert.equal(dry.status, 0); assert.deepEqual(JSON.parse(dry.stdout), { changed: ['a.json'] });
  assert.deepEqual(await bytes(area), original);
  const actual = cli(area, manifest); assert.equal(actual.status, 0); assert.equal(JSON.parse(await fs.readFile(join(area, 'a.json'))).n, 2);
  for (const args of [[], [area, manifest, '--unknown'], [area, manifest, '--dry-run', 'extra']]) {
    const bad = cli(...args); assert.notEqual(bad.status, 0); assert.ok(bad.stderr.trim());
  }
}));
const passed = results.filter(row => row.passed).length;
console.log(JSON.stringify({ total: results.length, passed, results }, null, 2));
process.exitCode = passed === results.length ? 0 : 1;
