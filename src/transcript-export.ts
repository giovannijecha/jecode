// Write one automatically named transcript to Jecode's application root.

import type { Block } from "./tui/blocks.ts";
import { atomicWrite } from "./atomic.ts";
import {
  assertDirectWritableInRoot,
  displayPath,
  resolveDirectWritableInRoot,
  resolveExistingInRoot,
} from "./tools/paths.ts";
import { defaultTranscriptName, transcriptMarkdown } from "./transcript.ts";

export async function saveTranscript(
  applicationRoot: string,
  blocks: readonly Block[],
  now = new Date(),
): Promise<string> {
  const root = await resolveExistingInRoot(applicationRoot, ".");
  const target = await resolveDirectWritableInRoot(root, defaultTranscriptName(now));
  const validate = () => assertDirectWritableInRoot(root, target);
  await atomicWrite(target, transcriptMarkdown(blocks), { validate });
  return displayPath(root, target);
}
