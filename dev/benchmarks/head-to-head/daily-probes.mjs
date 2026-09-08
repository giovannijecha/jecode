// Post-hoc source-review probes. Never replace the frozen acceptance scores.
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { request } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const [task, workspace] = process.argv.slice(2);
if (!['config-edit', 'file-server'].includes(task) || !workspace) throw new Error('Expected TASK WORKSPACE');
const source = await import(pathToFileURL(join(resolve(workspace), 'src/index.js')));
const base = await fs.mkdtemp(join(tmpdir(), 'jecode-daily-probe-'));
const results = [];
const changes = names => names.map(path => ({ path, operations: [{ op: 'set', path: '/n', value: 2 }] }));

async function migrationProbe() {
  const root = join(base, 'rollback');
  await fs.mkdir(root);
  const names = ['a.json', 'b.json'];
  const original = Buffer.from('{ "n": 1 }\n');
  for (const name of names) await fs.writeFile(join(root, name), original);
  const originalOpen = fs.open;
  const originalWrite = fs.writeFile;
  let blocked = false;
  let rejected = false;
  let blockedWrites = 0;
  function guard() {
    if (!blocked) return;
    blockedWrites++;
    throw Object.assign(new Error('Simulated unavailable data writes'), { code: 'EIO' });
  }
  fs.writeFile = async (...args) => { guard(); return originalWrite(...args); };
  fs.open = async (...args) => {
    const handle = await originalOpen(...args);
    const write = handle.writeFile.bind(handle);
    handle.writeFile = async (...args) => { guard(); return write(...args); };
    return handle;
  };
  syncBuiltinESMExports();
  try {
    await source.migrateFiles(root, changes(names), { onCommit() {
      blocked = true;
      throw new Error('Stop after the first commit');
    } });
  } catch { rejected = true; }
  finally {
    fs.open = originalOpen;
    fs.writeFile = originalWrite;
    syncBuiltinESMExports();
  }
  const bytes = await Promise.all(names.map(name => fs.readFile(join(root, name))));
  const remaining = (await fs.readdir(root)).filter(name => !names.includes(name));
  results.push({ name: 'rollback without further data writes', callbackReached: blocked,
    rejected, blockedWrites, restored: bytes.every(value => value.equals(original)),
    stagingFilesRemaining: remaining.length,
    faultBoundary: 'fs.promises.writeFile and FileHandle.writeFile; rename remains available',
    interpretation: 'Compound failure resilience; not an original acceptance requirement.' });

  for (const count of [4, 16]) {
    const directory = join(base, `reads-${count}`);
    await fs.mkdir(directory);
    const paths = Array.from({ length: count }, (_, index) => `${index}.json`);
    for (const path of paths) await fs.writeFile(join(directory, path), original);
    let opened = 0;
    fs.open = async (...args) => { opened++; return originalOpen(...args); };
    syncBuiltinESMExports();
    let report;
    try { report = await source.migrateFiles(directory, changes(paths)); }
    finally { fs.open = originalOpen; syncBuiltinESMExports(); }
    results.push({ name: 'file-open operations by batch size', files: count,
      opened, changed: report.changed.length,
      interpretation: 'Explicit fs.promises.open calls; other APIs may open files internally. Not latency or correctness.' });
  }
}

async function serverProbe() {
  const root = join(base, 'server');
  await fs.mkdir(root);
  await fs.writeFile(join(root, 'data.txt'), '0123456789');
  const time = new Date('2025-02-03T04:05:06Z');
  await fs.utimes(join(root, 'data.txt'), time, time);
  const service = await source.startServer(root);
  const get = headers => new Promise((resolve, reject) => {
    const req = request({ host: '127.0.0.1', port: service.port, path: '/data.txt', headers, agent: false }, res => {
      const chunks = [];
      res.on('data', chunk => chunks.push(chunk));
      res.on('error', reject);
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers,
        body: Buffer.concat(chunks).toString() }));
    });
    req.setTimeout(1500, () => req.destroy(new Error('Request timeout')));
    req.on('error', reject);
    req.end();
  });
  try {
    const initial = await get({});
    if (initial.status !== 200 || initial.body !== '0123456789' || !initial.headers.etag) {
      throw new Error('Full-response control failed');
    }
    for (const [name, headers, expected] of [
      ['valid ETag control', { 'if-none-match': initial.headers.etag }, 304],
      ['valid wildcard control', { 'if-none-match': '*' }, 304],
      ['leading empty list element', { 'if-none-match': `, ${initial.headers.etag}` }, 304],
      ['interior empty list element', { 'if-none-match': `${initial.headers.etag}, , "other"` }, 304],
      ['comma inside a nonmatching opaque tag', { 'if-none-match': `"other,tag", ${initial.headers.etag}` }, 304],
      ['unparsed prefix before ETag', { 'if-none-match': `junk ${initial.headers.etag}` }, null],
      ['unparsed suffix after ETag', { 'if-none-match': `${initial.headers.etag}, junk` }, null],
      ['embedded wildcard', { 'if-none-match': 'junk*' }, null],
      ['non-HTTP modified-since date', { 'if-modified-since': '2099-01-01' }, 200],
      ['non-HTTP range date', { range: 'bytes=0-1', 'if-range': '2099-01-01' }, 200],
    ]) {
      const response = await get(headers);
      results.push({ name, expectedStatus: expected, actualStatus: response.status,
        passed: expected === null ? null : response.status === expected,
        interpretation: expected === null
          ? 'Descriptive invalid-input policy: HTTP permits recovery from invalid constructs. Not a correctness rank.'
          : 'Supplementary parser verification; original acceptance is unchanged.' });
    }
    const originalOpen = fs.open;
    const handles = [];
    let injectedReads = 0;
    fs.open = async (...args) => {
      const handle = await originalOpen(...args);
      handles.push(handle);
      handle.read = async buffer => { injectedReads++; return { bytesRead: 0, buffer }; };
      return handle;
    };
    syncBuiltinESMExports();
    let response;
    let requestError;
    try { response = await get({}); }
    catch (error) { requestError = error.code ?? error.message; }
    finally { fs.open = originalOpen; syncBuiltinESMExports(); }
    const deadline = Date.now() + 1000;
    while (handles.some(handle => handle.fd !== -1) && Date.now() < deadline) {
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    results.push({ name: 'EOF after metadata before the first byte', injectedReads,
      actualStatus: response?.status ?? null, requestError: requestError ?? null,
      handlesClosed: handles.length > 0 && handles.every(handle => handle.fd === -1),
      passed: injectedReads === 0 ? null : response?.status === 500 &&
        Number(response.headers['content-length']) === Buffer.byteLength(response.body) &&
        Buffer.byteLength(response.body) <= 100 &&
        handles.every(handle => handle.fd === -1),
      interpretation: 'Post-hoc premature-EOF resilience; null means the FileHandle.read fault was not exercised.' });
  } finally {
    service.server.closeAllConnections();
    await service.close();
  }
}

try {
  if (task === 'config-edit') await migrationProbe();
  else await serverProbe();
  console.log(JSON.stringify({ task, results }, null, 2));
} finally {
  await fs.rm(base, { recursive: true, force: true });
}
