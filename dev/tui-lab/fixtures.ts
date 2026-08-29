// Realistic, deterministic content for the production TUI catalogue.
//
// Nothing here calls a provider or a tool. The same facts are handed to every
// scene so the comparison is about component states, not changing copy.

export type ToolTone = "ok" | "pending" | "fail";

export type ToolFixture = {
  readonly name: string;
  readonly target: string;
  readonly result: string;
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

export const conversation = {
  user: "Harden the OpenAI retry path, keep error bodies bounded, and update the tests.",
  reasoning: "I’ll inspect the HTTP boundary first, then trace how provider errors reach the controller.",
  tools: [
    {
      name: "read_file",
      target: "src/providers/http.ts",
      result: "146 lines · 7ms",
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
      result: "6 matches · 11ms",
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
      tone: "ok",
      detail: ["The bounded response reader now stops at 64 KiB."],
    },
    {
      name: "run_command",
      target: "npm test",
      result: "202 passed · 1.54s",
      tone: "ok",
      detail: ["tests 204", "pass 202", "fail 0", "skipped 2"],
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
  stat: "+8 −3",
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
      result: "2 files · 8ms",
      tone: "ok",
      detail: ["test/http.test.ts", "test/openai-http.test.ts"],
    },
    {
      name: "run_command",
      target: "node --test test/http.test.ts",
      result: "failed · 612ms",
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
      name: "read_file",
      target: "test/http.test.ts:58-80",
      result: "23 lines · 5ms",
      tone: "ok",
      detail: ["The test aborts during backoff and expects no second request."],
    },
    {
      name: "search_text",
      target: '"signal.aborted" in src/providers/http.ts',
      result: "running · 4.0s",
      tone: "pending",
    },
  ] satisfies readonly ToolFixture[],
  diagnosis: "The retry loop checks the signal before backoff, but not again before the next fetch.",
};
