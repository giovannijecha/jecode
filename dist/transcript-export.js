// Write one automatically named transcript to Jecode's application root.
import { atomicWrite } from "./atomic.js";
import { displayPath, resolveExistingInRoot, resolveWritableInRoot } from "./tools/paths.js";
import { defaultTranscriptName, transcriptMarkdown } from "./transcript.js";
export async function saveTranscript(applicationRoot, blocks, now = new Date()) {
    const root = await resolveExistingInRoot(applicationRoot, ".");
    const target = await resolveWritableInRoot(root, defaultTranscriptName(now));
    await atomicWrite(target, transcriptMarkdown(blocks));
    return displayPath(root, target);
}
