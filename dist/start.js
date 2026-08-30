// Application bootstrap: resolve one session, then choose its terminal surface.
import * as path from "node:path";
import { runBatch } from "./batch.js";
import { showCliInfo } from "./cli-info.js";
import { loadConfig } from "./config.js";
import { systemPrompt } from "./prompt.js";
import { selectProvider } from "./providers/index.js";
import { builtinTools } from "./tools/index.js";
import { configureColor } from "./ui/render.js";
import { STEEL } from "./ui/theme.js";
import { emptyUsage } from "./usage.js";
import { runApp } from "./tui/app.js";
import { interactive } from "./tui/screen.js";
export async function start(args = process.argv.slice(2), environment = {}) {
    const applicationRoot = environment.applicationRoot ?? path.resolve(import.meta.dirname, "..");
    const transcriptRoot = environment.transcriptRoot ?? process.cwd();
    const write = environment.write ?? ((text) => process.stdout.write(text));
    if (await showCliInfo(args, applicationRoot, write))
        return;
    const config = loadConfig(args);
    const provider = selectProvider(config.providerId);
    const hasScreen = environment.interactive?.() ?? interactive();
    // A provider whose catalogue is not fixed has no sensible default model.
    // The TUI can ask; a pipe cannot, so batch mode still requires one up front.
    const model = config.model === "" ? provider.defaultModel : config.model;
    if (model === "" && !hasScreen) {
        throw new Error(`${provider.id} has no default model — pass --model <id> (or set JECODE_MODEL)`);
    }
    configureColor(hasScreen);
    const session = {
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
    }
    else {
        await (environment.runNonInteractive ?? runBatch)(session);
    }
}
