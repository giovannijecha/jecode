import { test } from "node:test";
import assert from "node:assert/strict";
import { fromWireResponse as fromAnthropic } from "../src/providers/anthropic-wire.ts";
import { fromWireReply as fromOllama } from "../src/providers/ollama-wire.ts";
import { fromWireResponse as fromOpenAI } from "../src/providers/openai-wire.ts";

const EMPTY_USAGE = {
  inputTokens: 0,
  outputTokens: 0,
  cachedInputTokens: 0,
  cacheWriteInputTokens: 0,
  reasoningTokens: 0,
};

test("provider boundaries discard malformed token usage", () => {
  assert.deepEqual(fromOpenAI({
    output: [],
    usage: {
      input_tokens: "12",
      output_tokens: -1,
      input_tokens_details: { cached_tokens: 1.5, cache_write_tokens: Number.MAX_VALUE },
      output_tokens_details: { reasoning_tokens: Number.MAX_SAFE_INTEGER + 1 },
    },
  }).usage, EMPTY_USAGE);

  assert.deepEqual(fromAnthropic({
    content: [],
    usage: {
      input_tokens: "12",
      output_tokens: -1,
      cache_read_input_tokens: 1.5,
      cache_creation_input_tokens: Number.MAX_VALUE,
    },
  }).usage, EMPTY_USAGE);

  assert.deepEqual(fromOllama({
    content: "",
    reasoning: "",
    toolCalls: [],
    usage: { prompt_tokens: "12", completion_tokens: -1 },
  }).usage, EMPTY_USAGE);
});
