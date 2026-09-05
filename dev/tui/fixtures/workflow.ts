// Synthetic source and full evidence for the complete tool workflow.

import type { Detail } from "../../../src/tui/blocks.ts";
import { diff } from "../../../src/ui/diff.ts";
import type { ToolFixture } from "../fixtures.ts";

const PHASES = [
  ["resolveEndpoint", "resolve-endpoint"],
  ["prepareHeaders", "prepare-headers"],
  ["openConnection", "open-connection"],
  ["sendRequest", "send-request"],
  ["receiveHeaders", "receive-headers"],
  ["readErrorBody", "read-error-body"],
  ["readStream", "read-stream"],
  ["closeResponse", "close-response"],
  ["waitBeforeRetry", "wait-before-retry"],
  ["retryRequest", "retry-request"],
  ["finishAttempt", "finish-attempt"],
  ["flushDiagnostics", "flush-diagnostics"],
] as const;

const BEFORE = retrySource(false);
const AFTER = retrySource(true);
const EDIT_DETAILS = sourceDiff(BEFORE, AFTER);
const added = EDIT_DETAILS.filter((detail) => detail.kind === "add").length;
const removed = EDIT_DETAILS.filter((detail) => detail.kind === "del").length;

const CASES = PHASES.flatMap(([, phase]) => [
  { phase, checkpoint: "before-work", expectedRequests: 0 },
  { phase, checkpoint: "after-backoff", expectedRequests: 1 },
]);
const CREATED = [
  "// Cancellation cases shared by the focused HTTP checks.",
  "export const retryBoundaryCases = [",
  ...CASES.flatMap((item, index) => [
    "  {",
    `    phase: ${JSON.stringify(item.phase)},`,
    `    checkpoint: ${JSON.stringify(item.checkpoint)},`,
    `    expectedRequests: ${item.expectedRequests},`,
    "    expectedAbort: true,",
    `    label: ${JSON.stringify(`Case ${index + 1} · café · Δ · 👩🏽‍💻 · é · 漢字`)},`,
    `    diagnostic: ${JSON.stringify(
      `The ${item.phase} phase must stop at ${item.checkpoint}; preserve this complete diagnostic even when a narrow terminal clips its visible row. `.repeat(2),
    )},`,
    "  },",
  ]),
  "] as const;",
].join("\n");
const WRITE_DETAILS = sourceDiff("", CREATED);

const EXISTING_CASES = PHASES.flatMap(([, phase]) => [
  `${phase} handles an empty response`,
  `${phase} bounds diagnostic text`,
  `${phase} preserves provider errors`,
]);
const failedCount = EXISTING_CASES.length + 1;
const verifiedCount = failedCount + CASES.length;
const COMMAND = "node --test test/http.test.ts";
const FAILED_OUTPUT = [
  "TAP version 13",
  "# existing HTTP coverage",
  ...EXISTING_CASES.map((name, index) => `ok ${index + 1} - ${name}`),
  `not ok ${failedCount} - abort after backoff prevents another request`,
  "  ---",
  "  failureType: 'testCodeFailure'",
  "  error: 'expected 1 request, received 2'",
  "  code: 'ERR_ASSERTION'",
  "  expected: 1",
  "  actual: 2",
  "  operator: 'strictEqual'",
  "  stack: |-",
  "    TestContext.<anonymous> (test/http.test.ts:74:10)",
  "  ...",
  "# cancellation trace",
  "# request 1 started",
  "# backoff completed",
  "# abort signal received",
  "# request 2 started unexpectedly",
  `1..${failedCount}`,
  `# tests ${failedCount}`,
  `# pass ${EXISTING_CASES.length}`,
  "# fail 1",
  "# cancelled 0",
  "# skipped 0",
  "# duration_ms 4400",
];
const VERIFIED_OUTPUT = [
  "TAP version 13",
  "# existing HTTP coverage",
  ...EXISTING_CASES.map((name, index) => `ok ${index + 1} - ${name}`),
  `ok ${failedCount} - abort after backoff prevents another request`,
  "# generated cancellation boundary cases",
  ...CASES.map((item, index) => `ok ${failedCount + index + 1} - ${item.phase} stops at ${item.checkpoint}`),
  `1..${verifiedCount}`,
  `# tests ${verifiedCount}`,
  `# pass ${verifiedCount}`,
  "# fail 0",
  "# cancelled 0",
  "# skipped 0",
  "# duration_ms 4000",
];

export const WORKFLOW_EVIDENCE = {
  user: "Harden cancellation across the retry phases, add a boundary-case fixture, and run the focused HTTP checks.",
  reasoning: "I’ll inspect the provider directory and the retry phases, then run the focused HTTP suite. " +
    "The trace should show which work continues after cancellation. " +
    "I’ll apply the same explicit guard at each asynchronous boundary and add cases that cover both sides of backoff.",
  diagnosis: "The failed trace starts another request after cancellation. The same unguarded pattern appears in twelve phases. " +
    "Each phase needs a check before work, after its result, and after backoff. The new fixture will cover both cancellation checkpoints.",
  answer: [
    "Cancellation is guarded across all twelve retry phases, and the focused verification passes.",
    "",
    `- Updated \`src/providers/http.ts\` in twelve sections: ${added} added lines and ${removed} removed lines.`,
    `- Created \`test/fixtures/retry-boundaries.ts\` with ${CASES.length} cases, including Unicode and long diagnostics.`,
    `- Ran \`${COMMAND}\`: **${verifiedCount} passed, 0 failed**.`,
    "",
    "The original failing run remains above the changes. The full project check has not been run.",
  ].join("\n"),
  list: { name: "list_dir", target: "src/providers", result: "18 entries", tone: "ok" } satisfies ToolFixture,
  read: {
    name: "read_file", target: "src/providers/http.ts", result: `${BEFORE.split("\n").length} lines`, tone: "ok",
  } satisfies ToolFixture,
  failedRun: { name: "run_command", target: COMMAND, result: "exit 1", tone: "fail", detail: FAILED_OUTPUT } satisfies ToolFixture,
  edit: {
    name: "edit_file", target: "src/providers/http.ts", result: `+${added} −${removed} · applied`, tone: "ok",
  } satisfies ToolFixture,
  write: {
    name: "write_file", target: "test/fixtures/retry-boundaries.ts", result: `${WRITE_DETAILS.length} lines · created`, tone: "ok",
  } satisfies ToolFixture,
  verify: { name: "run_command", target: COMMAND, result: "exit 0", tone: "ok", detail: VERIFIED_OUTPUT } satisfies ToolFixture,
  editDetails: EDIT_DETAILS,
  writeDetails: WRITE_DETAILS,
  sourceBefore: BEFORE,
  sourceAfter: AFTER,
  createdSource: CREATED,
};

function retrySource(guarded: boolean): string {
  return [
    "type RetryContext = {",
    "  signal: AbortSignal;",
    "  run(phase: string): Promise<string>;",
    "  wait(milliseconds: number): Promise<void>;",
    '  record(phase: string, outcome: "completed"): void;',
    "};",
    "",
    ...PHASES.flatMap(([name, phase], index) => [
      `export async function ${name}(ctx: RetryContext): Promise<string> {`,
      ...(guarded ? [
        "  ctx.signal.throwIfAborted();",
        `  const value = await ctx.run("${phase}");`,
        "  ctx.signal.throwIfAborted();",
      ] : [
        `  const pending = ctx.run("${phase}");`,
        "  const value = await pending;",
      ]),
      `  await ctx.wait(${80 + index * 20});`,
      ...(guarded ? [
        "  ctx.signal.throwIfAborted();",
        `  ctx.record("${phase}", "completed");`,
      ] : []),
      "  return value;",
      "}",
      "",
    ]),
  ].join("\n").trimEnd();
}

function sourceDiff(before: string, after: string): Detail[] {
  let oldLine = 1;
  let newLine = 1;
  return diff(before, after).map((line): Detail => {
    if (line.kind === "add") return { ...line, newLine: newLine++ };
    if (line.kind === "del") return { ...line, oldLine: oldLine++ };
    return { ...line, oldLine: oldLine++, newLine: newLine++ };
  });
}
