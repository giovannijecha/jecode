// Development-only recovery experiment. Re-publish a verified conversation in
// a copied workspace; never rewrite the original experiment's session files.
import { pathToFileURL } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const [source, originalWorkspace, stagedSessions, sessionId, workspace, sessions] = process.argv.slice(2);
const { DurableSessionStore } = await import(pathToFileURL(path.join(source, 'src/sessions/store.ts')).href);
const previous = await DurableSessionStore.open(originalWorkspace, stagedSessions);
const loaded = await previous.load(sessionId);
assert.equal(loaded.conversation.activeNode.settlement, 'failed');
const destination = await DurableSessionStore.open(workspace, sessions);
const published = await destination.publish(loaded.conversation);
assert.deepEqual(published.conversation.nodes, loaded.conversation.nodes);
process.stdout.write(JSON.stringify({ sourceSession: sessionId, copiedSession: published.meta.id,
  nodes: published.conversation.nodes.length, conversationPreserved: true }) + '\n');
