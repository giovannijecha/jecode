// Bounded, opt-in numeric evidence for real development sessions.

import { channel } from "node:diagnostics_channel";
import { randomUUID } from "node:crypto";
import { open } from "node:fs/promises";
import * as path from "node:path";
import { assertDirectoryAnchor, preparePrivateDirectory } from "../../src/directory-anchor.ts";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../../src/context/diagnostics.ts";
import { userDataPath } from "../../src/user-data.ts";

const MAX_RECORDS = 4_096;
const MAX_PENDING = 64;

export async function contextRecorder(
  directory = userDataPath("diagnostics"),
  limit = MAX_RECORDS,
) {
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > MAX_RECORDS) {
    throw new Error("invalid context recording limit");
  }
  const anchor = await preparePrivateDirectory(directory, "context diagnostics", 0o700);
  const file = path.join(anchor.path, `context-${Date.now()}-${randomUUID()}.jsonl`);
  await assertDirectoryAnchor(anchor);
  const handle = await open(file, "wx", 0o600);
  try { await assertDirectoryAnchor(anchor); }
  catch (error) { await handle.close(); throw error; }
  const queue: string[] = [];
  let accepted = 0;
  let written = 0;
  let dropped = 0;
  let writeFailed = false;
  let stopped = false;
  let flushing: Promise<void> | undefined;
  let closing: Promise<void> | undefined;
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);

  async function flush(): Promise<void> {
    try {
      while (queue.length > 0 && !writeFailed) {
        const line = queue.shift() as string;
        try { await handle.writeFile(line, "utf8"); written++; }
        catch {
          writeFailed = true;
          dropped += queue.length + 1;
          queue.length = 0;
        }
      }
    } finally { flushing = undefined; }
  }

  function receive(value: unknown): void {
    if (stopped) return;
    // Channel publishers are not trusted to supply the declared TypeScript type.
    const event = safeDiagnostic(value);
    if (event === undefined) return;
    if (accepted >= limit || queue.length >= MAX_PENDING || writeFailed) { dropped++; return; }
    accepted++;
    queue.push(JSON.stringify({ version: 1, sequence: accepted, at: new Date().toISOString(), ...event }) + "\n");
    if (flushing === undefined) {
      flushing = flush();
    }
  }
  source.subscribe(receive);

  return {
    file,
    async close(): Promise<{ written: number; dropped: number; writeFailed: boolean }> {
      if (closing === undefined) {
        stopped = true;
        source.unsubscribe(receive);
        closing = (async () => {
          try {
            await flushing;
            if (!writeFailed) await handle.writeFile(JSON.stringify({
              version: 1, kind: "end", written, dropped,
            }) + "\n", "utf8");
          } catch { writeFailed = true; }
          finally { await handle.close(); }
        })();
      }
      await closing;
      return { written, dropped, writeFailed };
    },
  };
}
