// Application bootstrap: resolve one session, then choose its terminal surface.

import * as path from "node:path";
import { runBatch } from "./batch.ts";
import { showCliInfo } from "./cli-info.ts";
import { loadConfig } from "./config.ts";
import { systemPrompt } from "./prompt.ts";
import { configureProviders, selectProvider } from "./providers/index.ts";
import type { Session } from "./session.ts";
import { builtinTools } from "./tools/index.ts";
import { configureColor } from "./ui/render.ts";
import { STEEL } from "./ui/theme.ts";
import { emptyUsage } from "./usage.ts";
import { runApp } from "./tui/app.ts";
import { interactive } from "./tui/screen.ts";

export type StartEnvironment = {
  applicationRoot?: string;
  transcriptRoot?: string;
  interactive?(): boolean;
  runInteractive?(session: Session, applicationRoot: string): Promise<void>;
  runNonInteractive?(session: Session): Promise<void>;
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
  const config = loadConfig(args);
  configureProviders(config);
  const provider = selectProvider(config.providerId);
  const hasScreen = environment.interactive?.() ?? interactive();

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
    history: [],
    usage: emptyUsage(),
  };

  if (hasScreen) {
    await (environment.runInteractive ?? runApp)(session, transcriptRoot);
  } else {
    await (environment.runNonInteractive ?? runBatch)(session);
  }
}
