import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
export async function createHandler(root) {
  return async (request, response) => {
    try {
      const content = await readFile(resolve(root, '.' + request.url.split('?')[0]));
      response.writeHead(200, { 'content-length': content.length }); response.end(content);
    } catch { response.writeHead(404); response.end('Not found'); }
  };
}
