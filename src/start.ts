// Application bootstrap: resolve one session, then choose its terminal surface.

import * as path from "node:path";
import { runBatch } from "./batch.ts";
import { showCliInfo } from "./cli-info.ts";
import { loadConfig } from "./config.ts";
import { ConversationTree } from "./conversation.ts";
import { parseLaunch } from "./launch.ts";
import { systemPrompt } from "./prompt.ts";
import { selectProvider } from "./providers/index.ts";
import type { Session } from "./session.ts";
import { SessionPersistence } from "./sessions/runtime.ts";
import { DurableSessionStore } from "./sessions/store.ts";
import { builtinTools } from "./tools/index.ts";
import { configureColor } from "./ui/render.ts";
import { STEEL } from "./ui/theme.ts";
import { emptyUsage, usageFromHistory } from "./usage.ts";
import { runApp } from "./tui/app.ts";
import { interactive } from "./tui/screen.ts";

export type StartEnvironment = {
  applicationRoot?: string;
  transcriptRoot?: string;
  sessionsRoot?: string;
  signal?: AbortSignal;
  interactive?(): boolean;
  runInteractive?(session: Session, applicationRoot: string, signal?: AbortSignal): Promise<void>;
  runNonInteractive?(session: Session, signal?: AbortSignal): Promise<void>;
  write?(text: string): void;
};

export async function start(
  args: string[] = process.argv.slice(2),
  environment: StartEnvironment = {},
): Promise<void> {
  const applicationRoot = environment.applicationRoot ?? path.resolve(import.meta.dirname, "..");
  const transcriptRoot = environment.transcriptRoot ?? process.cwd();
  const write = environment.write ?? ((text: string) => process.stdout.write(text));
  if (await showCliInfo(args, applicationRoot, write)) return;
  const launch = parseLaunch(args);
  const config = loadConfig(launch.configArgs);
  const provider = selectProvider(config.providerId);
  const hasScreen = environment.interactive?.() ?? interactive();
  if (launch.kind === "resume" && !hasScreen) {
    throw new Error("resume needs an interactive terminal");
  }
  if (launch.kind === "resume" && config.ephemeral) {
    throw new Error("--ephemeral cannot be combined with resume");
  }

  // A provider whose catalogue is not fixed has no sensible default model.
  // The TUI can ask; a pipe cannot, so batch mode still requires one up front.
  const model = config.model === "" ? provider.defaultModel : config.model;
  if (model === "" && !hasScreen) {
    throw new Error(`${provider.id} has no default model — pass --model <id> (or set JECODE_MODEL)`);
  }

  configureColor(hasScreen);
  const session: Session = {
    config,
    provider,
    model,
    palette: STEEL,
    tools: builtinTools(),
    system: systemPrompt(config),
    conversation: ConversationTree.empty(),
    usage: emptyUsage(),
  };

  if (hasScreen) {
    if (!config.ephemeral) {
      const store = await DurableSessionStore.open(config.root, environment.sessionsRoot);
      if (launch.kind === "resume") {
        const candidates = await SessionPersistence.candidates(store);
        if (candidates.length === 0) throw new Error("no resumable sessions found for this workspace");
        const open = async (id: string): Promise<void> => {
          const resumed = await SessionPersistence.resume(store, id);
          try {
            applyResumedSession(session, resumed.conversation, resumed.persistence);
          } catch (error) {
            await resumed.persistence.close();
            throw error;
          }
        };
        if (launch.latest) await open((candidates[0] as (typeof candidates)[number]).id);
        else session.resume = { candidates, open };
      } else {
        session.persistence = SessionPersistence.fresh(store);
      }
    }
    try {
      if (environment.runInteractive === undefined) {
        await runApp(session, transcriptRoot, { shutdownSignal: environment.signal });
      } else {
        await environment.runInteractive(session, transcriptRoot, environment.signal);
      }
    } finally {
      await session.persistence?.close();
    }
  } else {
    if (environment.runNonInteractive === undefined) {
      await runBatch(session, { signal: environment.signal });
    } else {
      await environment.runNonInteractive(session, environment.signal);
    }
  }
}

function applyResumedSession(
  session: Session,
  conversation: ConversationTree,
  persistence: SessionPersistence,
): void {
  const identity = conversation.activeNode?.identity;
  if (identity === undefined) throw new Error("resumed session has no active turn");
  const provider = selectProvider(identity.providerId);
  const config = {
    ...session.config,
    providerId: identity.providerId,
    model: identity.model,
    effort: identity.effort,
  };
  const system = systemPrompt(config);
  const usage = usageFromHistory(conversation.history);

  session.config = config;
  session.provider = provider;
  session.model = identity.model;
  session.system = system;
  session.conversation = conversation;
  session.usage = usage;
  session.persistence = persistence;
  session.resume = undefined;
}
