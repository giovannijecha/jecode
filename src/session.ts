// What one run holds. Its own module so the app and the slash commands can
// both depend on the shape without depending on each other.

import type { Config } from "./config.ts";
import type { Message, Provider } from "./types.ts";
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
  history: Message[];
  usage: UsageTotals;
};
