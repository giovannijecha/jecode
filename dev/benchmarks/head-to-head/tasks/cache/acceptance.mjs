// Frozen before live trials. Deferred loads expose races without timing sleeps.
import assert from 'node:assert/strict';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { getEventListeners } from 'node:events';

const root = resolve(process.argv[2]);
const { createCache } = await import(pathToFileURL(join(root, 'src/cache.js')));
const { createDirectory } = await import(pathToFileURL(join(root, 'src/service.js')));
const results = [];
const deferred = () => Promise.withResolvers();
const tick = () => new Promise(resolve => setImmediate(resolve));
// A broken cache can make several callers reject together. Observe each at creation
// so the scorer reports the behavioral failure rather than terminating itself.
const observed = promise => { promise.catch(() => {}); return promise; };
async function check(name, run) {
  let timer;
  try {
    await Promise.race([run(), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('check timed out')), 1500);
    })]);
    results.push({ name, passed: true });
  } catch (error) { results.push({ name, passed: false, error: String(error.message).slice(0, 500) }); }
  finally { clearTimeout(timer); }
}
for (const value of [undefined, null, false, 0, '', 'value']) {
  await check(`cache resolved ${String(value)}`, async () => {
    let calls = 0;
    const cache = createCache(() => { calls++; return value; });
    assert.equal(await cache.get(''), value);
    assert.equal(await cache.get(''), value);
    assert.equal(calls, 1);
  });
}
await check('coalesce per key and keep different keys independent', async () => {
  const loads = new Map(); let calls = 0;
  const cache = createCache(key => { calls++; const load = deferred(); loads.set(key, load); return load.promise; });
  const a = cache.get('__proto__'), b = cache.get('__proto__'), c = cache.get('constructor');
  await tick(); assert.equal(calls, 2);
  loads.get('constructor').resolve(2); assert.equal(await c, 2);
  loads.get('__proto__').resolve(1); assert.deepEqual(await Promise.all([a,b]), [1,1]);
});
await check('TTL starts on resolution and expiry is inclusive', async () => {
  let now = 0, calls = 0;
  const load = deferred();
  const cache = createCache(() => { calls++; return calls === 1 ? load.promise : 2; }, { ttlMs: 10, now: () => now });
  const first = cache.get('x'); now = 50;
  const second = cache.get('x'); await tick(); assert.equal(calls, 1);
  load.resolve(1); await Promise.all([first,second]);
  now = 59; assert.equal(await cache.get('x'), 1);
  now = 60; assert.equal(await cache.get('x'), 2);
});
await check('zero TTL coalesces pending but not resolved loads', async () => {
  const load = deferred(); let calls = 0;
  const cache = createCache(() => { calls++; return calls === 1 ? load.promise : 2; }, { ttlMs: 0 });
  const a = cache.get('x'), b = cache.get('x'); await tick(); assert.equal(calls,1);
  load.resolve(1); assert.deepEqual(await Promise.all([a,b]),[1,1]);
  assert.equal(await cache.get('x'), 2); assert.equal(calls,2);
});
for (const ttlMs of [-1, NaN, Infinity, '10', null]) {
  await check(`invalid TTL ${String(ttlMs)}`, async () => assert.throws(() => createCache(() => 1, { ttlMs })));
}
await check('invalid loader and clock', async () => {
  assert.throws(() => createCache(null));
  assert.throws(() => createCache(() => 1, { now: 1 }));
});
await check('invalid keys never load', async () => {
  let calls = 0; const cache = createCache(() => ++calls);
  for (const key of [1, null, undefined, {}, []]) {
    await assert.rejects(async () => cache.get(key));
    assert.throws(() => cache.invalidate(key));
  }
  assert.equal(calls,0);
});
for (const sync of [false,true]) {
  await check(`failure retries; synchronous=${sync}`, async () => {
    const failure = new Error('load failed'); let calls = 0;
    const cache = createCache(() => {
      if (++calls > 1) return 2;
      if (sync) throw failure;
      return Promise.reject(failure);
    });
    const pending = cache.get('x'); assert.equal(typeof pending.then,'function');
    await assert.rejects(pending, error => error === failure);
    assert.equal(await cache.get('x'), 2);
  });
}
for (const operation of ['invalidate','clear']) for (const lateFailure of [false,true]) {
  await check(`${operation}: detached ${lateFailure ? 'failure' : 'success'} cannot replace new value`, async () => {
    const old = deferred(), fresh = deferred(); let calls = 0;
    const cache = createCache(() => ++calls === 1 ? old.promise : fresh.promise);
    const a = cache.get('x').then(value => ({ value }), error => ({ error }));
    await tick(); cache[operation]('x');
    const b = observed(cache.get('x')); await tick(); assert.equal(calls,2);
    fresh.resolve('new'); assert.equal(await b,'new');
    if (lateFailure) old.reject('old failure'); else old.resolve('old');
    assert.deepEqual(await a, lateFailure ? { error:'old failure' } : { value:'old' });
    assert.equal(await cache.get('x'),'new'); assert.equal(calls,2);
  });
  await check(`${operation}: old cleanup cannot remove new pending load; failure=${lateFailure}`, async () => {
    const old = deferred(), fresh = deferred(); let calls = 0;
    const cache = createCache(() => ++calls === 1 ? old.promise : fresh.promise);
    const a = cache.get('x').catch(() => 'failed'); await tick(); cache[operation]('x');
    const b = observed(cache.get('x')); await tick();
    if (lateFailure) old.reject(new Error('old')); else old.resolve('old');
    await a; const c = observed(cache.get('x')); await tick(); assert.equal(calls,2);
    fresh.resolve('new'); assert.deepEqual(await Promise.all([b,c]),['new','new']);
  });
}
await check('preaborted misses and hits preserve exact reason and do not load', async () => {
  let calls=0; const cache=createCache(() => ++calls);
  const controller=new AbortController(); controller.abort({ stopped:true });
  await assert.rejects(cache.get('x',{signal:controller.signal}), error => error===controller.signal.reason);
  assert.equal(calls,0); await cache.get('x');
  await assert.rejects(cache.get('x',{signal:controller.signal}), error => error===controller.signal.reason);
  assert.equal(calls,1);
});
await check('one waiter cancels promptly while another succeeds; listeners cleaned', async () => {
  const load=deferred(), a=new AbortController(), b=new AbortController();
  const cache=createCache(() => load.promise);
  const cancelled=cache.get('x',{signal:a.signal}).then(() => assert.fail('resolved'),error => error);
  const success=cache.get('x',{signal:b.signal}); a.abort('stop');
  assert.equal(await cancelled,'stop'); assert.equal(getEventListeners(a.signal,'abort').length,0);
  load.resolve(7); assert.equal(await success,7);
  assert.equal(getEventListeners(b.signal,'abort').length,0);
});
await check('sole cancelled caller does not prevent population', async () => {
  const load=deferred(), controller=new AbortController(); let calls=0;
  const cache=createCache(() => { calls++; return load.promise; });
  const waiter=cache.get('x',{signal:controller.signal}).catch(error => error);
  controller.abort('stop'); assert.equal(await waiter,'stop');
  load.resolve(9); await tick(); assert.equal(await cache.get('x'),9); assert.equal(calls,1);
});
await check('cancelled failed load has no unhandled rejection', async () => {
  const errors=[]; const onError=error => errors.push(error);
  process.on('unhandledRejection',onError);
  try {
    const load=deferred(), controller=new AbortController();
    const cache=createCache(() => load.promise);
    const waiter=cache.get('x',{signal:controller.signal}).catch(error => error);
    controller.abort('stop'); assert.equal(await waiter,'stop');
    cache.clear(); load.reject(new Error('detached')); await tick(); await tick();
    assert.deepEqual(errors,[]); assert.equal(getEventListeners(controller.signal,'abort').length,0);
  } finally { process.off('unhandledRejection',onError); }
});
await check('service invalidates one key and resets all keys', async () => {
  let calls=0; const service=createDirectory(() => ++calls);
  assert.equal(await service.user('a'),1); assert.equal(await service.user('b'),2);
  service.changed('a'); assert.equal(await service.user('a'),3); assert.equal(await service.user('b'),2);
  service.reset(); assert.equal(await service.user('b'),4);
});
console.log(JSON.stringify({ total:results.length, passed:results.filter(r => r.passed).length, results },null,2));
process.exitCode=results.every(r => r.passed)?0:1;
