// Post-hoc probe inspired by a live output's self-review. Not a frozen score.
import assert from 'node:assert/strict';
import { resolve, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const { createCache } = await import(pathToFileURL(join(resolve(process.argv[2]),'src/cache.js')));
const controller = new AbortController();
let cancelOnRead = false;
let loads = 0;
const cache = createCache(() => ++loads, { now() {
  // A finite, monotone clock can run caller code during get's cache-hit path.
  if (cancelOnRead) controller.abort('cancelled during clock callback');
  return 0;
} });
await cache.get('key');
cancelOnRead = true;
let result;
try {
  await assert.rejects(cache.get('key',{signal:controller.signal}),error => error===controller.signal.reason);
  assert.equal(loads,1);
  result = { passed:true };
} catch (error) { result = { passed:false,error:String(error.message) }; }
console.log(JSON.stringify({ postHoc:true,case:'cache-hit cancellation inside injected clock',...result },null,2));
process.exitCode = result.passed ? 0 : 1;
