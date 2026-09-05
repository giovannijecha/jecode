// Realistic, deterministic content for the production TUI catalogue.
//
// Nothing here calls a provider or a tool. The same facts are handed to every
// scene so the comparison is about component states, not changing copy.

import { providerLabel } from "../../src/provider-label.ts";

export type ToolTone = "ok" | "pending" | "fail";

export type ToolFixture = {
  readonly name: string;
  readonly target: string;
  readonly result: string;
  readonly durationMs?: number;
  readonly tone: ToolTone;
  readonly detail?: readonly string[];
};

export type DiffLine = {
  readonly kind: "keep" | "add" | "del" | "gap";
  readonly oldLine?: number;
  readonly newLine?: number;
  readonly text: string;
  readonly emphasis?: string;
};

export const workspace = "~/Codex/jecode";
export const model = "claude-sonnet-5";

export const reasoningSample = [
  "The request is about the lower terminal rhythm, so I should keep operational state out of the conversation model.",
  "The footer has fixed height and can carry one replaceable status without moving the composer.",
  "Reasoning can keep its complete source text while the renderer follows only the newest three wrapped rows.",
  "That also makes resize deterministic: reflow first, then take the visible tail from the same semantic block.",
].join(" ");

export const markdownSample = [
  "`README.md`",
  "",
  "```md",
  "# Sandbox",
  "",
  "Scratch files for exercising jecode. Nothing here matters.",
  "```",
  "",
  "`total.js` — sum the item quantities:",
  "",
  "```js",
  "export function total(rows) {",
  "  return rows.reduce((n, row) => n + row.qty, 0);",
  "}",
  "```",
  "",
  "So `total.js` returns **11** for the sample CSV.",
].join("\n");

export const literalPromptSample = [
  "Keep **these marks** and `src/main.ts` exactly as written.",
  "# This is input, not a heading",
  "",
  "```ts",
  "  const pattern = \"src/**/*.ts\";",
  "```",
].join("\n");

export const conversation = {
  user: "Harden the OpenAI retry path, keep error bodies bounded, and update the tests.",
  reasoning: "I’ll inspect the HTTP boundary first, then trace how provider errors reach the controller.",
  tools: [
    {
      name: "read_file",
      target: "src/providers/http.ts",
      result: "146 lines",
      durationMs: 7,
      tone: "ok",
      detail: [
        "export async function request(...) {",
        "  // retry and response parsing boundary",
        "}",
      ],
    },
    {
      name: "search_text",
      target: "\"request(\" in src/providers",
      result: "6 matches",
      durationMs: 11,
      tone: "ok",
      detail: [
        "src/providers/anthropic.ts:41",
        "src/providers/openai.ts:52",
        "src/providers/ollama.ts:38",
      ],
    },
    {
      name: "edit_file",
      target: "src/providers/http.ts",
      result: "+18 −6 · approved",
      durationMs: 42,
      tone: "ok",
      detail: ["The bounded response reader now stops at 64 KiB."],
    },
  ] satisfies readonly ToolFixture[],
  answer: [
    "Retry handling is hardened and verified.",
    "Abort now stops both backoff and response reads.",
    "Error bodies are capped before decoding.",
    "204 tests complete with no failures.",
  ],
};

export const editPreview = {
  user: "Make retry errors concise, but keep enough context to diagnose the provider response.",
  reasoning: "The response reader is the narrowest place to enforce the limit consistently.",
  path: "src/providers/http.ts",
  stat: "+4 −1",
  lines: [
    { kind: "keep", oldLine: 38, newLine: 38, text: "async function errorMessage(response: Response) {" },
    {
      kind: "del",
      oldLine: 39,
      text: "  const body = await response.text();",
      emphasis: "response.text()",
    },
    {
      kind: "add",
      newLine: 39,
      text: "  const body = await boundedText(response, 64 * 1024);",
      emphasis: "boundedText(response, 64 * 1024)",
    },
    { kind: "keep", oldLine: 40, newLine: 40, text: '  return response.status + " " + body;' },
    { kind: "gap", text: "14 unchanged lines" },
    { kind: "add", newLine: 56, text: "async function boundedText(response: Response, limit: number) {" },
    { kind: "add", newLine: 57, text: "  const reader = response.body?.getReader();" },
    { kind: "add", newLine: 58, text: "  // Stop collecting once the diagnostic budget is full." },
  ] satisfies readonly DiffLine[],
};

export const toolTrace = {
  user: "Run the focused tests and diagnose any failure before changing code.",
  tools: [
    {
      name: "find_files",
      target: "test/**/*http*.test.ts",
      result: "2 files",
      durationMs: 8,
      tone: "ok",
      detail: ["test/http.test.ts", "test/openai-http.test.ts"],
    },
    {
      name: "run_command",
      target: "node --test test/http.test.ts",
      result: "failed",
      durationMs: 612,
      tone: "fail",
      detail: [
        "✖ abort stops a pending retry",
        "  AssertionError: expected 1 request, received 2",
        "  at test/http.test.ts:74:10",
        "",
        "request 1 started",
        "abort signal received",
        "backoff timer cleared",
        "request 2 started unexpectedly",
        "response body closed",
        "controller caught the failure",
        "focused suite stopped",
        "1 failed · 4 passed",
        "diagnostic tail",
      ],
    },
    {
      name: "search_text",
      target: '"signal.aborted" in src/providers/http.ts',
      result: "running",
      tone: "pending",
    },
  ] satisfies readonly ToolFixture[],
  diagnosis: "The retry loop checks the signal before backoff, but not again before the next fetch.",
};

export const suiteRun = {
  user: "Run the focused HTTP suite and keep the useful end of its output visible.",
  command: "node --test test/http.test.ts",
  result: "exit 0",
  durationMs: 1_398,
  output: [
    "TAP version 13",
    "# request boundary",
    "ok 1 - retries a rate limit",
    "ok 2 - abort stops pending backoff",
    "ok 3 - caps response bodies",
    "ok 4 - rejects redirects",
    "ok 5 - times out an idle stream",
    "ok 6 - preserves caller cancellation",
    "# provider adapters",
    "ok 7 - Anthropic errors stay bounded",
    "ok 8 - OpenAI errors stay bounded",
    "ok 9 - Ollama errors stay bounded",
    "1..9",
    "# tests 9",
    "# pass 9",
    "# fail 0",
    "# duration_ms 1398",
  ],
  answer: "The focused suite passes. The collapsed trace keeps the verdict and timing in view.",
};

export const outputCapDiff = {
  user: "Cap command output at the shared text boundary and keep the diff reviewable.",
  reasoning: "The collector is the single boundary shared by stdout and stderr.",
  path: "src/tools/shell.ts",
  stat: "+7 −3",
  answer: "Command output is bounded. The compact diff keeps the changed head and tail; expansion retains every hunk.",
  lines: [
    { kind: "keep", oldLine: 5, newLine: 5, text: 'import { spawn } from "node:child_process";' },
    { kind: "add", newLine: 6, text: 'import { OUTPUT_CAP } from "./text-boundary.ts";' },
    { kind: "keep", oldLine: 6, newLine: 7, text: "" },
    { kind: "del", oldLine: 8, text: "const MAX_OUTPUT = 1_000_000;", emphasis: "1_000_000" },
    { kind: "add", newLine: 9, text: "const MAX_OUTPUT = OUTPUT_CAP;", emphasis: "OUTPUT_CAP" },
    { kind: "gap", text: "… 31 unchanged" },
    { kind: "keep", oldLine: 41, newLine: 42, text: "function collect(chunk: string): void {" },
    { kind: "del", oldLine: 42, text: "  chunks.push(chunk);" },
    { kind: "add", newLine: 43, text: "  const room = MAX_OUTPUT - seen;" },
    { kind: "add", newLine: 44, text: "  if (room <= 0) return;" },
    { kind: "add", newLine: 45, text: "  chunks.push(chunk.slice(0, room));" },
    { kind: "add", newLine: 46, text: "  seen += chunk.length;" },
    { kind: "keep", oldLine: 43, newLine: 47, text: "}" },
    { kind: "gap", text: "… 18 unchanged" },
    { kind: "keep", oldLine: 62, newLine: 66, text: "const text = chunks.join(\"\");" },
    { kind: "del", oldLine: 63, text: "return text;" },
    { kind: "add", newLine: 67, text: "return capped(text, seen);" },
    { kind: "keep", oldLine: 64, newLine: 68, text: "}" },
  ] satisfies readonly DiffLine[],
};

export const commandApproval = {
  user: "Run only the release package checks.",
  reasoning: "This executes code, so the exact command needs an explicit decision.",
  command: "npm run check:package",
  context: ["package.json", "scripts/check-package.ts"],
};

export const modelQuery = "cla";
export const modelChoices = [
  { label: "claude-sonnet-5", hint: "balanced" },
  { label: "claude-opus-5", hint: "deep" },
  { label: "claude-haiku-4.5", hint: "fast" },
  { label: "gpt-5.4", hint: providerLabel("openai") },
  { label: "kimi-k2.7-code", hint: providerLabel("ollama") },
];
