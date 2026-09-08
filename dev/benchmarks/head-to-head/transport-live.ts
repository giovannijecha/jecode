// Authorized, development-only transport experiment. Synthetic prompts only;
// account material is read through the runtime's isolated data home.
import { writeFile } from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import { openaiCodex } from "../../../src/providers/openai-codex.ts";
import { normalizeProviderError } from "../../../src/providers/failure.ts";
import type { Message, TransportObservation } from "../../../src/types.ts";

const [transport, destination] = process.argv.slice(2);
if ((transport !== "http" && transport !== "websocket") || destination === undefined) {
  throw new Error("Usage: transport-live.ts http|websocket NEW_REPORT");
}
const model = "gpt-6-astra";
const identity = { conversationId: randomUUID(), cacheKey: randomUUID(), purpose: "turn" as const };
// The HTTP run uses the independently hashed forced-HTTP source variant,
// retaining turn routing headers just like a production upgrade fallback.
const scope = openaiCodex.openTurn!();
const messages: Message[] = [];
const requests: Record<string, unknown>[] = [];
const prompts = [
  "Write a self-contained JavaScript module implementing an asynchronous bounded worker pool. " +
  "Accept items, an async worker and a positive concurrency limit. Preserve result order, stop launching new jobs " +
  "after the first worker rejection, await already started jobs before rejecting, and avoid attaching a new " +
  "Promise.race handler to a slow job on each completion. Include meaningful node:test tests using deferred " +
  "promises for ordering, failure, and concurrent execution. Return the module and tests as code. No tools.",
  "Review the module you just wrote for cancellation or rejection handling, resource growth and ordering. " +
  "Identify a concrete remaining flaw if present and provide a focused correction. If none is found, say so " +
  "with the key invariants. Do not add features. Do not claim tests were executed.",
];
try {
  for (const prompt of prompts) {
    messages.push({ role: "user", content: [{ kind: "text", text: prompt }] });
    const started = performance.now();
    let firstEventMs: number | undefined;
    let observations: TransportObservation[] = [];
    let stage = "starting";
    const heartbeat = setInterval(() => process.stderr.write(JSON.stringify({ transport,
      request: requests.length + 1, stage, elapsedSeconds: Math.round((performance.now() - started) / 1000) }) + "\n"), 30_000);
    try {
      const reply = await scope.send({
        model, effort: "high", maxTokens: 8192, identity,
        system: "Solve the supplied programming request accurately and stay within its scope.",
        messages, tools: [], signal: AbortSignal.timeout(360_000),
        onStream() { firstEventMs ??= performance.now() - started; },
        onStatus(value) { stage = value; }, onTransport(value) { observations.push(value); },
      });
      if (observations.at(-1)?.transport !== transport) throw new Error("Observed transport differs from the declared variant");
      messages.push(reply);
      const text = reply.content.filter(block => block.kind === "text").map(block => block.text).join("\n");
      requests.push({ status: "completed", elapsedMs: performance.now() - started, firstEventMs,
        usage: reply.usage, observations, outputChars: text.length,
        outputHash: createHash("sha256").update(text).digest("hex") });
    } catch (error) {
      const failure = normalizeProviderError("openai-codex", error);
      requests.push({ status: "failed", elapsedMs: performance.now() - started, firstEventMs,
        observations, failure: { kind: failure.kind, message: failure.message } });
      process.exitCode = 1;
      break;
    } finally { clearInterval(heartbeat); observations = []; }
  }
} finally {
  scope.close();
  await writeFile(destination, JSON.stringify({ transport, model, effort: "high", requests }, null, 2) + "\n", { flag: "wx", mode: 0o600 });
}
