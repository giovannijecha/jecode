import { startServer } from './index.js';
try {
  const [root, flag, port] = process.argv.slice(2);
  if (!root || (flag && flag !== '--port')) throw new Error('Usage: ROOT [--port PORT]');
  const running = await startServer(root, { port: port === undefined ? 0 : Number(port) });
  console.log(JSON.stringify({ port: running.port }));
  process.on('SIGINT', () => { void running.close(); });
} catch (error) { console.error(error.message); process.exitCode = 1; }
