import { validateEvaluator } from '../../validate-evaluator.mjs';
await validateEvaluator(import.meta.url, [
  ['traversal', "decoded.split('/').includes('..')", 'false'],
  ['suffix range', 'Math.max(0, size - b)', '0'],
  ['HEAD', "request.method === 'GET' ? request.headers.range : undefined", 'request.headers.range'],
  ['conditional precedence', 'if (inm !== undefined ?', 'if (false ?'],
], { cli: `import { startServer } from './index.js';
try {
 const args = process.argv.slice(2), [root, flag, value] = args;
 if (!root || ![1,3].includes(args.length) || (args.length === 3 && (flag !== '--port' || !/^[0-9]+$/.test(value)))) throw new Error('usage');
 const port = value === undefined ? 0 : Number(value);
 if (!Number.isInteger(port) || port > 65535) throw new Error('port');
 const running = await startServer(root, { port });
 console.log(JSON.stringify({ port: running.port }));
 process.on('SIGINT', () => { void running.close(); }); process.on('SIGTERM', () => { void running.close(); });
} catch (error) { console.error(String(error)); process.exitCode = 1; }
` });
