import { createServer } from 'node:http';
import { createHandler } from './handler.js';
export async function startServer(root, { port = 0, host = '127.0.0.1' } = {}) {
  const server = createServer(await createHandler(root));
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, host, resolve); });
  return { server, port: server.address().port, close: () => new Promise(resolve => server.close(resolve)) };
}
