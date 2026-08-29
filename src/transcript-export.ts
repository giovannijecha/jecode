// Write one automatically named transcript to Jecode's application root.

import type { Block } from "./tui/blocks.ts";
import { atomicWrite } from "./atomic.ts";
import { displayPath, resolveExistingInRoot, resolveWritableInRoot } from "./tools/paths.ts";
import { defaultTranscriptName, transcriptMarkdown } from "./transcript.ts";

export async function saveTranscript(
  applicationRoot: string,
  blocks: readonly Block[],
  now = new Date(),
): Promise<string> {
  const root = await resolveExistingInRoot(applicationRoot, ".");
  const target = await resolveWritableInRoot(root, defaultTranscriptName(now));
  await atomicWrite(target, transcriptMarkdown(blocks));
  return displayPath(root, target);
}
