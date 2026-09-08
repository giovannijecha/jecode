// Evaluator calibration only; not visible to either measured client.
import * as fs from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import path from 'node:path';
import { createServer } from 'node:http';
import { pipeline } from 'node:stream';
const mime = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript',
  '.json': 'application/json', '.svg': 'image/svg+xml', '.txt': 'text/plain' };
const weak = tag => tag.trim().replace(/^W\//u, '');
function selectedRange(value, size) {
  const match = /^bytes=(\d*)-(\d*)$/u.exec(value);
  if (!match || (!match[1] && !match[2]) || size === 0) return null;
  const a = match[1] ? Number(match[1]) : undefined, b = match[2] ? Number(match[2]) : undefined;
  if ([a,b].some(number => number !== undefined && !Number.isSafeInteger(number))) return null;
  if (a === undefined) return b > 0 ? [Math.max(0, size - b), size - 1] : null;
  if (a >= size || (b !== undefined && b < a)) return null;
  return [a, Math.min(b ?? size - 1, size - 1)];
}
export async function createHandler(root) {
  root = await fs.realpath(root);
  if (!(await fs.stat(root)).isDirectory()) throw new Error('root');
  return (request, response) => {
    const fail = status => {
      if (response.headersSent) { response.destroy(); return; }
      const body = { 400: 'Bad request', 403: 'Forbidden', 404: 'Not found', 405: 'Method not allowed', 416: 'Range not satisfiable', 500: 'Internal error' }[status];
      response.statusCode = status;
      response.end(request.method === 'HEAD' ? undefined : body);
    };
    const send = async () => {
      if (!['GET','HEAD'].includes(request.method)) { response.setHeader('Allow', 'GET, HEAD'); fail(405); return; }
      let decoded;
      try { decoded = decodeURIComponent(request.url.split('?')[0]); } catch { fail(400); return; }
      if (decoded.includes('\0')) { fail(400); return; }
      if (decoded.includes('\\') || decoded.split('/').includes('..')) { fail(403); return; }
      let file = root;
      for (const part of decoded.split('/').filter(part => part && part !== '.')) {
        file = path.join(file, part);
        if ((await fs.lstat(file)).isSymbolicLink()) { fail(403); return; }
      }
      const stat = await fs.stat(file);
      if (!stat.isFile()) { fail(404); return; }
      const type = mime[path.extname(file).toLowerCase()];
      const etag = `W/"${stat.size.toString(16)}-${Math.trunc(stat.mtimeMs).toString(16)}"`;
      const modified = Math.floor(stat.mtimeMs / 1000) * 1000;
      response.setHeader('Content-Type', type ? type + '; charset=utf-8' : 'application/octet-stream');
      response.setHeader('ETag', etag); response.setHeader('Last-Modified', new Date(modified).toUTCString());
      response.setHeader('Accept-Ranges', 'bytes');
      const inm = request.headers['if-none-match'], ims = request.headers['if-modified-since'];
      if (inm !== undefined ? inm.split(',').some(tag => tag.trim() === '*' || weak(tag) === weak(etag)) :
        ims !== undefined && Number.isFinite(Date.parse(ims)) && Date.parse(ims) >= modified) {
        response.statusCode = 304; response.end(); return;
      }
      let range = request.method === 'GET' ? request.headers.range : undefined;
      const condition = request.headers['if-range'];
      if (condition !== undefined && (!Number.isFinite(Date.parse(condition)) || Date.parse(condition) < modified || /"/u.test(condition))) range = undefined;
      let start = 0, end = stat.size - 1;
      if (range?.startsWith('bytes=')) {
        const span = selectedRange(range, stat.size);
        if (span === null) { response.setHeader('Content-Range', `bytes */${stat.size}`); fail(416); return; }
        [start, end] = span; response.statusCode = 206;
        response.setHeader('Content-Range', `bytes ${start}-${end}/${stat.size}`);
      }
      response.setHeader('Content-Length', Math.max(0, end - start + 1));
      if (request.method === 'HEAD' || stat.size === 0) { response.end(); return; }
      const stream = createReadStream(file, { start, end });
      stream.once('open', () => {
        if (response.destroyed) { stream.destroy(); return; }
        pipeline(stream, response, () => {});
      });
      stream.once('error', () => fail(500));
      response.once('close', () => stream.destroy());
    };
    void send().catch(error => fail(['ENOENT','ENOTDIR'].includes(error.code) ? 404 : 500));
  };
}
export async function startServer(root, { port = 0, host = '127.0.0.1' } = {}) {
  const server = createServer(await createHandler(root));
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, host, resolve); });
  let closing;
  return { server, port: server.address().port, close() {
    return closing ??= new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  } };
}
