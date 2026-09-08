// Offline reproduction of native child-process capture restrictions.
// Run separately from timed agent trials; no provider, file writes or network.
import { spawnSync } from 'node:child_process';

for (const stdio of ['pipe', 'ignore', 'inherit']) {
  const result = spawnSync(process.execPath, ['-e', 'process.exit(0)'], {
    stdio, timeout: 5000,
  });
  console.log(JSON.stringify({ node: process.version, stdio,
    status: result.status, error: result.error?.code ?? null }));
  if (result.status !== 0 || result.error !== undefined) process.exitCode = 1;
}
