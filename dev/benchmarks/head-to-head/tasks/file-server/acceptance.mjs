import assert from 'node:assert/strict';
import * as fs from 'node:fs/promises';
import { request } from 'node:http';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';
const source = resolve(process.argv[2]);
const { createHandler, startServer } = await import(pathToFileURL(join(source, 'src/index.js')));
const results = [];
async function check(name, run) {
  try { await run(); results.push({ name, passed: true }); }
  catch (error) { results.push({ name, passed: false, error: String(error?.message ?? error).slice(0, 400) }); }
}
const base = await fs.mkdtemp(join(tmpdir(), 'http-eval-')), root = join(base, 'root');
await fs.mkdir(root); await fs.mkdir(join(root, 'directory'));
await fs.writeFile(join(root, 'data.txt'), '0123456789'); await fs.writeFile(join(root, 'empty.bin'), '');
await fs.writeFile(join(root, 'caf\u00e9 space.txt'), 'unicode'); await fs.writeFile(join(base, 'secret.txt'), 'private');
await fs.symlink(join(base, 'secret.txt'), join(root, 'link.txt'));
await fs.symlink(base, join(root, 'linked'), 'dir');
const old = new Date('2025-02-03T04:05:06Z'); await fs.utimes(join(root, 'data.txt'), old, old);
const service = await startServer(root);
function get(path, headers = {}, method = 'GET', port = service.port) {
  return new Promise((resolve, reject) => {
    const req = request({ host: '127.0.0.1', port, path, headers, method, agent: false }, res => {
      const chunks = [];
      res.on('data', chunk => chunks.push(chunk));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks).toString() }));
      res.on('error', reject);
    });
    req.setTimeout(1500, () => req.destroy(new Error('request timeout')));
    req.on('error', reject); req.end();
  });
}
try {
  await check('root validation', async () => { await assert.rejects(createHandler(join(base, 'missing'))); await assert.rejects(createHandler(join(root, 'data.txt'))); });
  const first = await get('/data.txt');
  await check('full response and validators', () => {
    assert.equal(first.status, 200); assert.equal(first.body, '0123456789');
    assert.equal(first.headers['content-length'], '10'); assert.match(first.headers.etag, /^W\/".+"$/u);
    assert.equal(first.headers['last-modified'], old.toUTCString()); assert.equal(first.headers['accept-ranges'], 'bytes');
  });
  for (const [ext, expected] of [['txt','text/plain'], ['html','text/html'], ['css','text/css'], ['js','text/javascript'], ['json','application/json'], ['svg','image/svg+xml'], ['bin','application/octet-stream']]) {
    await check(`MIME ${ext}`, async () => {
      await fs.writeFile(join(root, 'asset.' + ext), 'abc');
      assert.equal((await get('/asset.' + ext)).headers['content-type'], expected + (ext === 'bin' ? '' : '; charset=utf-8'));
    });
  }
  await check('HEAD is bodyless and ignores range', async () => {
    const res = await get('/data.txt', { range: 'bytes=0-1' }, 'HEAD');
    assert.equal(res.status, 200); assert.equal(res.body, ''); assert.equal(res.headers['content-length'], '10');
  });
  await check('method rejection', async () => {
    const res = await get('/data.txt', {}, 'POST'); assert.equal(res.status, 405); assert.equal(res.headers.allow, 'GET, HEAD');
  });
  for (const target of ['/missing', '/directory', '/']) {
    await check(`not found ${target}`, async () => assert.equal((await get(target)).status, 404));
  }
  await check('HEAD errors never contain a body', async () => { const res = await get('/missing', {}, 'HEAD'); assert.equal(res.status, 404); assert.equal(res.body, ''); });
  await check('escaped names and query', async () => { assert.equal((await get('/caf%C3%A9%20space.txt?ignored=1')).body, 'unicode'); });
  for (const target of ['/%zz', '/%00', '/%FF']) {
    await check(`malformed path ${target}`, async () => assert.equal((await get(target)).status, 400));
  }
  for (const target of ['/../secret.txt', '/%2e%2e/secret.txt', '/a/%2e%2e/secret.txt', '/%5csecret.txt', '/link.txt', '/linked/secret.txt']) {
    await check(`forbidden ${target}`, async () => { const res = await get(target); assert.equal(res.status, 403); assert.ok(!res.body.includes(base)); assert.ok(!res.body.includes('private')); });
  }
  for (const match of ['*', first.headers.etag ?? 'missing', `"other", ${first.headers.etag ?? 'missing'}`, first.headers.etag?.replace(/^W\//u, '') ?? 'missing']) {
    await check(`If-None-Match ${match}`, async () => { const res = await get('/data.txt', { 'if-none-match': match }); assert.equal(res.status, 304); assert.equal(res.body, ''); });
  }
  await check('nonmatching ETag overrides date', async () => {
    assert.equal((await get('/data.txt', { 'if-none-match': '"other"', 'if-modified-since': 'Thu, 01 Jan 2099 00:00:00 GMT' })).status, 200);
  });
  await check('modified since and invalid date', async () => {
    assert.equal((await get('/data.txt', { 'if-modified-since': old.toUTCString() })).status, 304);
    assert.equal((await get('/data.txt', { 'if-modified-since': 'invalid' })).status, 200);
    assert.equal((await get('/data.txt', { 'if-modified-since': 'Mon, 01 Jan 2024 00:00:00 GMT' })).status, 200);
  });
  for (const [range, start, end] of [['bytes=0-2',0,2], ['bytes=4-',4,9], ['bytes=-3',7,9], ['bytes=-99',0,9], ['bytes=8-99',8,9], ['bytes=0-0',0,0]]) {
    await check(`range ${range}`, async () => {
      const res = await get('/data.txt', { range }); assert.equal(res.status, 206);
      assert.equal(res.body, '0123456789'.slice(start, end + 1));
      assert.equal(res.headers['content-range'], `bytes ${start}-${end}/10`); assert.equal(res.headers['content-length'], String(end - start + 1));
    });
  }
  for (const range of ['bytes=10-', 'bytes=5-2', 'bytes=-0', 'bytes=-', 'bytes=0-1,3-4', 'bytes=x-2', 'bytes=9007199254740992-', 'bytes=0-9007199254740992']) {
    await check(`invalid range ${range}`, async () => { const res = await get('/data.txt', { range }); assert.equal(res.status, 416); assert.equal(res.headers['content-range'], 'bytes */10'); });
  }
  await check('empty file and empty range', async () => {
    const res = await get('/empty.bin'); assert.equal(res.status, 200); assert.equal(res.body, ''); assert.equal(res.headers['content-length'], '0');
    const ranged = await get('/empty.bin', { range: 'bytes=0-' }); assert.equal(ranged.status, 416); assert.equal(ranged.headers['content-range'], 'bytes */0');
  });
  await check('unknown range unit ignored', async () => assert.equal((await get('/data.txt', { range: 'items=0-1' })).status, 200));
  for (const condition of [old.toUTCString(), 'Mon, 01 Jan 2024 00:00:00 GMT', 'invalid', first.headers.etag ?? 'missing', '"other"']) {
    await check(`If-Range ${condition}`, async () => assert.equal((await get('/data.txt', { range: 'bytes=0-1', 'if-range': condition })).status,
      condition === old.toUTCString() ? 206 : 200));
  }
  await check('304 precedes unsatisfiable range', async () => assert.equal((await get('/data.txt', { range: 'bytes=99-', 'if-none-match': '*' })).status, 304));
  await check('disconnect while streaming leaves the server usable', async () => {
    const file = await fs.open(join(root, 'large.bin'), 'w'); await file.truncate(32 * 1024 * 1024); await file.close();
    await new Promise((resolve, reject) => {
      const req = request({ host: '127.0.0.1', port: service.port, path: '/large.bin', agent: false }, res => {
        res.once('data', () => { res.destroy(); resolve(); }); res.on('error', () => {});
      }); req.on('error', reject); req.setTimeout(2000, () => req.destroy(new Error('first chunk timeout'))); req.end();
    });
    assert.equal((await get('/data.txt')).body, '0123456789');
  });
  await check('CLI rejects invalid usage', () => {
    for (const args of [[], [root, '--bad'], [root, '--port'], [root, '--port', '-1'], [root, '--port', '65536'], [root, '--port', 'abc'], [root, '--port', '0', 'extra']]) {
      const result = spawnSync(process.execPath, [join(source, 'src/cli.js'), ...args], { encoding: 'utf8', timeout: 1500 });
      assert.notEqual(result.status, 0); assert.ok(result.stderr.trim()); assert.ok(!result.error, 'CLI did not exit');
    }
  });
  await check('CLI loopback startup and SIGTERM shutdown', async () => {
    const child = spawn(process.execPath, [join(source, 'src/cli.js'), root, '--port', '0'], { stdio: ['ignore','pipe','pipe'] });
    let timer;
    try {
      const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
      const port = await Promise.race([new Promise((resolve, reject) => {
        let text = ''; child.stdout.on('data', chunk => { text += chunk; if (text.includes('\n')) { try { resolve(JSON.parse(text.split('\n')[0]).port); } catch (error) { reject(error); } } });
        child.on('error', reject); child.on('exit', () => reject(new Error('CLI exited before listening')));
      }), new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('startup timeout')), 2000); })]);
      clearTimeout(timer); assert.equal((await get('/data.txt', {}, 'GET', port)).status, 200);
      child.kill('SIGTERM');
      const outcome = await Promise.race([exited, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('shutdown timeout')), 2000); })]);
      assert.equal(outcome.code, 0); assert.equal(outcome.signal, null);
    } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL'); }
  });
} finally {
  await service.close();
  await check('close is repeatable', () => service.close());
  await fs.rm(base, { recursive: true, force: true });
}
const passed = results.filter(row => row.passed).length;
console.log(JSON.stringify({ total: results.length, passed, results }, null, 2));
process.exitCode = passed === results.length ? 0 : 1;
