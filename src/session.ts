// What one run holds. Its own module so the app and the slash commands can
// both depend on the shape without depending on each other.

import type { Config } from "./config.ts";
import type { ConversationTree } from "./conversation.ts";
import type { Provider } from "./types.ts";
import type { SessionCatalogEntry } from "./sessions/store.ts";
import type { SessionPersistence } from "./sessions/runtime.ts";
import type { Tool } from "./tools/index.ts";
import type { Palette } from "./ui/theme.ts";
import type { UsageTotals } from "./usage.ts";

export type Session = {
  config: Config;
  provider: Provider;
  model: string;
  palette: Palette;
  tools: Tool[];
  system: string;
  conversation: ConversationTree;
  usage: UsageTotals;
  /** Absent in batch mode and when --ephemeral was requested. */
  persistence?: SessionPersistence;
  /** Present only for an unresolved `jecode resume` selector. */
  resume?: {
    candidates: readonly SessionCatalogEntry[];
    open(id: string): Promise<void>;
  };
};
