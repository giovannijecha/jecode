// Write one automatically named transcript to Jecode's application root.
import { atomicWrite } from "./atomic.js";
import { assertDirectWritableInRoot, displayPath, resolveDirectWritableInRoot, resolveExistingInRoot, } from "./tools/paths.js";
import { defaultTranscriptName, transcriptMarkdown } from "./transcript.js";
export async function saveTranscript(applicationRoot, blocks, now = new Date()) {
    const root = await resolveExistingInRoot(applicationRoot, ".");
    const target = await resolveDirectWritableInRoot(root, defaultTranscriptName(now));
    const validate = () => assertDirectWritableInRoot(root, target);
    await atomicWrite(target, transcriptMarkdown(blocks), { validate });
    return displayPath(root, target);
}
