// The non-interactive path: stdin is a pipe, so there is no screen to own.
//
// It exists so the agent can be scripted and tested. It shares the controller,
// the tools and the block renderer with the TUI — only the surface differs.

import * as readline from "node:readline/promises";
import { stdin, stdout } from "node:process";
import type { Session } from "./session.ts";
import type { ControllerEvents } from "./controller.ts";
import { runTurn } from "./controller.ts";
import { handleCommand } from "./commands.ts";
import type { Block } from "./tui/blocks.ts";
import { renderBatch } from "./batch-view.ts";
import { columns } from "./ui/render.ts";
import { terminalText } from "./ui/terminal-text.ts";
import { recordUsage } from "./usage.ts";

export type BatchEnvironment = {
  lines?: AsyncIterable<string>;
  write?(text: string): void;
  width?: number;
};

export async function runBatch(session: Session, environment: BatchEnvironment = {}): Promise<void> {
  const rl = environment.lines === undefined ? readline.createInterface({ input: stdin }) : undefined;
  const lines = environment.lines ?? (rl as AsyncIterable<string>);
  const write = environment.write ?? ((text: string) => stdout.write(text));
  const width = environment.width ?? columns();

  const emit = (block: Block): void => {
    for (const line of renderBatch(block, width, session.palette)) write(`${line}\n`);
  };

  try {
    for await (const raw of lines) {
      const line = raw.trim();
      if (line === "") continue;

      if (line.startsWith("/")) {
        if ((await handleCommand(line, session, { emit })) === "exit") break;
        continue;
      }

      write(`> ${terminalText(line)}\n`);
      session.history.push({ role: "user", content: [{ kind: "text", text: line }] });

      const turn = events(emit, session);
      try {
        await runTurn(session.history, options(session), turn);
      } catch (error) {
        emit({ kind: "notice", text: (error as Error).message, tone: "error" });
      } finally {
        turn.flush();
      }
    }
  } finally {
    rl?.close();
  }
}

function options(session: Session) {
  return {
    provider: session.provider,
    tools: session.tools,
    model: session.model,
    system: session.system,
    maxTokens: session.config.maxTokens,
    effort: session.config.effort,
    maxSteps: session.config.maxSteps,
    toolContext: { root: session.config.root },
  };
}

// Prose is buffered rather than streamed: without a screen to repaint there is
// nothing to gain from partial lines, and plenty to lose in readability.
function events(
  emit: (block: Block) => void,
  session: Session,
): ControllerEvents & { flush(): void } {
  let answer = "";

  const flush = (): void => {
    if (answer === "") return;
    emit({ kind: "answer", text: answer });
    answer = "";
  };

  return {
    flush,
    onStream(event) {
      if (event.kind === "text") answer += event.text;
    },
    onToolCall() {
      flush();
    },
    onToolResult(call, result, summary) {
      emit({
        kind: "tool",
        name: call.name,
        target: "",
        right: summary ?? "",
        tone: result.isError ? "fail" : "ok",
      });
    },
    approve() {
      flush();
      // Nobody is watching a pipe. Approval has to be granted up front with
      // --auto-approve, never inferred from silence.
      return Promise.resolve(session.config.autoApprove);
    },
    onUsage(usage) {
      recordUsage(session.usage, usage);
    },
  };
}
