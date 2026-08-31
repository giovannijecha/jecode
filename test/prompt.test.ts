import assert from "node:assert/strict";
import test from "node:test";
import type { Config } from "../src/config.ts";
import { systemPrompt } from "../src/prompt.ts";

const config: Config = {
  providerId: "ollama",
  model: "example-model",
  reducedMotion: false,
  effort: "high",
  maxTokens: 1_000,
  maxSteps: 4,
  root: "/workspace/jecode",
  autoApprove: false,
};

test("the system prompt stays product-neutral while retaining runtime context", () => {
  const prompt = systemPrompt(config);
  const runtimeLines = new Set([
    `Workspace root: ${config.root}`,
    `Platform: ${process.platform}`,
  ]);
  const globalRules = prompt
    .split("\n")
    .filter((line) => !runtimeLines.has(line))
    .join("\n");

  assert.doesNotMatch(globalRules, /\bjecode\b/i);
  assert.doesNotMatch(globalRules, /^You are\b/im);
  assert.ok(prompt.includes(`Workspace root: ${config.root}`));
  assert.ok(prompt.includes(`Platform: ${process.platform}`));
});
