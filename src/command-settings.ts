// Shared persistence boundary for commands that change non-secret defaults.

import type { Host } from "./commands.ts";
import type { SavedSettings } from "./settings.ts";
import { updateSettings } from "./settings.ts";

export async function saveCommandSettings(
  host: Host,
  patch: Partial<SavedSettings>,
): Promise<boolean> {
  try {
    if (host.saveSettings === undefined) await updateSettings(patch);
    else await host.saveSettings(patch);
    return true;
  } catch (error) {
    host.emit({
      kind: "notice",
      text: `could not save settings · ${(error as Error).message}`,
      tone: "error",
    });
    return false;
  }
}
