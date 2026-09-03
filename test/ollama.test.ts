import { test } from "node:test";
import assert from "node:assert/strict";
import type { Message, StreamEvent } from "../src/types.ts";
import { assembleOllama } from "../src/providers/ollama-stream.ts";
import { fromWireReply, toWireMessages } from "../src/providers/ollama-wire.ts";
import { supportsAdaptiveThinking } from "../src/providers/anthropic.ts";
import { MAX_TOOL_ARGUMENT_CHARS } from "../src/providers/stream-limits.ts";
import { configureOllama, ollama } from "../src/providers/ollama.ts";

async function* feed(events: unknown[]): AsyncGenerator<unknown> {
  for (const event of events) yield event;
}

function chunk(delta: unknown, finish: string | null = null): unknown {
  return { choices: [{ delta, finish_reason: finish }] };
}

test("offers and sends Ollama's supported reasoning effort levels", async (context) => {
  const previousFetch = globalThis.fetch;
  let requestBody: Record<string, unknown> | undefined;
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
    return new Response(
      'data: {"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}\n\n',
      { status: 200, headers: { "content-type": "text/event-stream" } },
    );
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  assert.deepEqual(
    await ollama.efforts?.("deepseek-v4-flash:0731"),
    ["low", "medium", "high"],
  );

  const message = await ollama.send({
    model: "deepseek-v4-flash:0731",
    system: "be useful",
    messages: [],
    tools: [],
    maxTokens: 100,
    effort: "medium",
  });

  assert.equal(requestBody?.["reasoning_effort"], "medium");
  assert.deepEqual(requestBody?.["stream_options"], { include_usage: true });
  assert.equal(message.usage, undefined, "compatible endpoints may omit the usage trailer");
});

test("prefers Ollama's currently allocated runtime context", async (context) => {
  const previousFetch = globalThis.fetch;
  const model = "runtime-context-fixture:latest";
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async (input: string | URL | Request) => {
    assert.equal(String(input), "http://127.0.0.1:11434/api/ps");
    return new Response(JSON.stringify({
      models: [{ name: model, context_length: 32_768 }],
    }), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 31_129 });
});

test("keeps a minimum 4K Ollama allocation after safety headroom", async (context) => {
  const previousFetch = globalThis.fetch;
  const model = "small-context-fixture:latest";
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async () => new Response(JSON.stringify({
    models: [{ name: model, context_length: 4_096 }],
  }), { status: 200, headers: { "content-type": "application/json" } })) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 3_891 });
});

test("falls back to Ollama model capacity when the model is not loaded", async (context) => {
  const previousFetch = globalThis.fetch;
  const model = "stored-context-fixture:latest";
  const urls: string[] = [];
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    if (url.endsWith("/api/ps")) {
      return new Response(JSON.stringify({ models: [] }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response(JSON.stringify({
      model_info: { "fixture.context_length": 131_072 },
    }), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 124_518 });
  assert.deepEqual(urls, [
    "http://127.0.0.1:11434/api/ps",
    "http://127.0.0.1:11434/api/show",
  ]);
});

test("reuses Ollama model capacity while still checking for a runtime allocation", async (context) => {
  const previousFetch = globalThis.fetch;
  const model = "cached-context-fixture:latest";
  const urls: string[] = [];
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async (input: string | URL | Request) => {
    const url = String(input);
    urls.push(url);
    if (url.endsWith("/api/ps")) {
      return new Response(JSON.stringify({ models: [] }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response(JSON.stringify({
      model_info: { "fixture.context_length": 131_072 },
    }), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 124_518 });
  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 124_518 });
  assert.deepEqual(urls, [
    "http://127.0.0.1:11434/api/ps",
    "http://127.0.0.1:11434/api/show",
    "http://127.0.0.1:11434/api/ps",
  ]);
});

test("does not cache Ollama's theoretical capacity over a later runtime allocation", async (context) => {
  const previousFetch = globalThis.fetch;
  const model = "changing-context-fixture:latest";
  let loaded = false;
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async (input: string | URL | Request) => {
    const url = String(input);
    if (url.endsWith("/api/ps")) {
      return new Response(JSON.stringify({
        models: loaded ? [{ name: model, context_length: 32_768 }] : [],
      }), { status: 200, headers: { "content-type": "application/json" } });
    }
    loaded = true;
    return new Response(JSON.stringify({
      model_info: { "fixture.context_length": 131_072 },
    }), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 124_518 });
  assert.deepEqual(await ollama.contextWindow?.(model), { tokens: 31_129 });
});

test("rejects an Ollama effort outside the documented vocabulary", async (context) => {
  const previousFetch = globalThis.fetch;
  let requests = 0;
  configureOllama("http://127.0.0.1:11434");
  globalThis.fetch = (async () => {
    requests++;
    throw new Error("request should not be made");
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = previousFetch;
    configureOllama(undefined);
  });

  await assert.rejects(
    ollama.send({
      model: "deepseek-v4-flash:0731",
      system: "be useful",
      messages: [],
      tools: [],
      maxTokens: 100,
      effort: "max",
    }),
    /does not support effort "max"/,
  );
  assert.equal(requests, 0);
});

test("accumulates text deltas and streams them as they land", async () => {
  const seen: StreamEvent[] = [];
  const reply = await assembleOllama(
    feed([chunk({ content: "Ciao" }), chunk({ content: " mondo" }), chunk({}, "stop")]),
    (event) => seen.push(event),
  );

  assert.equal(reply.content, "Ciao mondo");
  assert.equal(reply.finishReason, "stop");
  assert.deepEqual(seen, [
    { kind: "text", text: "Ciao" },
    { kind: "text", text: " mondo" },
  ]);
});

test("rebuilds a tool call split across chunks", async () => {
  const seen: StreamEvent[] = [];
  const reply = await assembleOllama(
    feed([
      chunk({ tool_calls: [{ index: 0, id: "c1", function: { name: "read_file", arguments: "" } }] }),
      chunk({ tool_calls: [{ index: 0, function: { arguments: '{"path"' } }] }),
      chunk({ tool_calls: [{ index: 0, function: { arguments: ':"a.ts"}' } }] }),
      chunk({}, "tool_calls"),
    ]),
    (event) => seen.push(event),
  );

  const message = fromWireReply(reply);
  assert.deepEqual(message.content, [
    { kind: "tool_call", id: "c1", name: "read_file", input: { path: "a.ts" } },
  ]);
  assert.deepEqual(seen, [{ kind: "tool", name: "read_file" }]);
});

test("bounds tool arguments accumulated across Ollama calls", async () => {
  await assert.rejects(
    assembleOllama(
      feed([
        chunk({
          tool_calls: [{ index: 0, function: { arguments: "x".repeat(MAX_TOOL_ARGUMENT_CHARS) } }],
        }),
        chunk({ tool_calls: [{ index: 1, function: { arguments: "x" } }] }),
      ]),
    ),
    /streamed tool arguments exceeded/,
  );
});

test("keeps parallel calls apart by index and invents an id when none is sent", async () => {
  const seen: StreamEvent[] = [];
  const reply = await assembleOllama(
    feed([
      chunk({
        tool_calls: [
          { index: 0, function: { name: "list_dir", arguments: "{}" } },
          { index: 1, function: { name: "read_file", arguments: '{"path":"b.ts"}' } },
        ],
      }),
      chunk({}, "tool_calls"),
    ]),
    (event) => seen.push(event),
  );

  assert.deepEqual(
    reply.toolCalls.map((call) => [call.id, call.name]),
    [
      ["call_0", "list_dir"],
      ["call_1", "read_file"],
    ],
  );
  assert.deepEqual(seen, [
    { kind: "tool", name: "list_dir" },
    { kind: "tool", name: "read_file" },
  ]);
});

test("streams reasoning under either field name", async () => {
  const seen: StreamEvent[] = [];
  const reply = await assembleOllama(
    feed([chunk({ reasoning: "hm" }), chunk({ reasoning_content: "ok" }), chunk({}, "stop")]),
    (event) => seen.push(event),
  );

  assert.equal(reply.reasoning, "hmok");
  assert.deepEqual(seen, [
    { kind: "thinking", text: "hm" },
    { kind: "thinking", text: "ok" },
  ]);
});

test("preserves malformed and non-object tool arguments as invalid calls", () => {
  const calls = [
    ["malformed", "{not json", "tool arguments were not valid JSON"],
    ["array", "[]", "tool arguments must be a JSON object"],
    ["scalar", "42", "tool arguments must be a JSON object"],
  ] as const;

  for (const [id, args, inputError] of calls) {
    const message = fromWireReply({
      content: "",
      reasoning: "",
      toolCalls: [{ id, name: "list_dir", args }],
      finishReason: "tool_calls",
    });
    assert.deepEqual(message.content, [{
      kind: "tool_call",
      id,
      name: "list_dir",
      input: {},
      inputError,
    }]);
  }
});

test("keeps partial text but rejects a tool call truncated by the output limit", async () => {
  const reply = await assembleOllama(feed([
    chunk({ content: "partial", reasoning: "unfinished" }),
    chunk({
      tool_calls: [{
        index: 0,
        id: "partial-call",
        function: { name: "write_file", arguments: '{"path":"partial' },
      }],
    }),
    chunk({}, "length"),
  ]));

  const message = fromWireReply(reply);
  assert.deepEqual(message.content, [
    { kind: "text", text: "partial" },
    { kind: "text", text: "[truncated: hit the output limit — raise --max-tokens]" },
  ]);
  assert.equal(message.raw, undefined);
  assert.equal(message.rawFrom, undefined);
});

test("echoes Ollama reasoning with an assistant tool call on continuation", async () => {
  const reply = await assembleOllama(feed([
    chunk({ reasoning: "inspect first" }),
    chunk({
      tool_calls: [{
        index: 0,
        id: "c1",
        function: { name: "read_file", arguments: '{"path":"a.ts"}' },
      }],
    }),
    chunk({}, "tool_calls"),
  ]));
  const assistant = fromWireReply(reply);

  assert.deepEqual(toWireMessages("", [assistant]), [{
    role: "assistant",
    content: "",
    reasoning: "inspect first",
    tool_calls: [{
      id: "c1",
      type: "function",
      function: { name: "read_file", arguments: '{"path":"a.ts"}' },
    }],
  }]);
});

test("lifts every tool result in order before following steering", () => {
  const history: Message[] = [
    { role: "user", content: [{ kind: "text", text: "leggi a.ts" }] },
    {
      role: "assistant",
      content: [{ kind: "tool_call", id: "c1", name: "read_file", input: { path: "a.ts" } }],
    },
    {
      role: "user",
      content: [{ kind: "tool_result", id: "c1", output: "export {}", isError: false }],
    },
    { role: "user", content: [{ kind: "text", text: "change direction" }] },
  ];

  assert.deepEqual(toWireMessages("be useful", history), [
    { role: "system", content: "be useful" },
    { role: "user", content: "leggi a.ts" },
    {
      role: "assistant",
      content: "",
      tool_calls: [
        { id: "c1", type: "function", function: { name: "read_file", arguments: '{"path":"a.ts"}' } },
      ],
    },
    { role: "tool", tool_call_id: "c1", content: "export {}" },
    { role: "user", content: "change direction" },
  ]);
});

test("omits tool_calls entirely when the assistant only spoke", () => {
  const wire = toWireMessages("", [
    { role: "assistant", content: [{ kind: "text", text: "fatto" }] },
  ]);

  assert.deepEqual(wire, [{ role: "assistant", content: "fatto" }]);
});

test("adaptive thinking is sent only to the models that accept it", () => {
  assert.equal(supportsAdaptiveThinking("claude-sonnet-5"), true);
  assert.equal(supportsAdaptiveThinking("claude-opus-5"), true);
  assert.equal(supportsAdaptiveThinking("claude-opus-4-6"), true);
  assert.equal(supportsAdaptiveThinking("claude-haiku-4-5-20251001"), false);
  assert.equal(supportsAdaptiveThinking("claude-sonnet-4-5"), false);
});

test("normalizes usage from the final compatible chunk", async () => {
  const reply = await assembleOllama(
    feed([
      chunk({}, "stop"),
      { choices: [], usage: { prompt_tokens: 14, completion_tokens: 4 } },
    ]),
  );

  assert.deepEqual(fromWireReply(reply).usage, {
    inputTokens: 14,
    outputTokens: 4,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  });
});

test("preserves zero usage from the final compatible chunk", async () => {
  const reply = await assembleOllama(
    feed([
      chunk({}, "stop"),
      { choices: [], usage: { prompt_tokens: 0, completion_tokens: 0 } },
    ]),
  );

  assert.deepEqual(fromWireReply(reply).usage, {
    inputTokens: 0,
    outputTokens: 0,
    cachedInputTokens: 0,
    cacheWriteInputTokens: 0,
    reasoningTokens: 0,
  });
});

test("rejects a stream that ends before a finish reason", async () => {
  await assert.rejects(
    assembleOllama(feed([chunk({ content: "partial" })])),
    /ended before a finish reason/,
  );
});
