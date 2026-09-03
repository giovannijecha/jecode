// Durable, workspace-scoped conversation storage.
//
// Each node is its own atomically replaced file. The head is advanced only
// after that node is durable, so a crash leaves either the prior checkpoint or
// one strictly recoverable mutation -- never an ambiguous partial history.

import { createHash, randomUUID } from "node:crypto";
import {
  chmod,
  lstat,
  mkdir,
  open,
  opendir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  stat,
} from "node:fs/promises";
import * as path from "node:path";
import { atomicWrite } from "../atomic.ts";
import { CONVERSATION_LIMITS, ConversationTree } from "../conversation.ts";
import type { TurnNode } from "../conversation.ts";
import { leadingText } from "../text-boundary.ts";
import { userDataPath } from "../user-data.ts";
import {
  decodeHead,
  decodeMeta,
  decodeNode,
  encodeHead,
  encodeMeta,
  encodeNode,
  SESSION_FILE_LIMITS,
  SESSION_SCHEMA,
} from "./codec.ts";
import type { SessionHead, SessionMeta, StoredNode } from "./codec.ts";
import {
  leaseOwner,
  leaseToken,
  pidIsAlive,
  removeLease,
  sessionLease,
} from "./lease.ts";
import type { SessionLease } from "./lease.ts";

export type { SessionLease } from "./lease.ts";

const DIRECTORY_MODE = 0o700;
const FILE_MODE = 0o600;
const MAX_CATALOG_ENTRIES = 4_096;
const CATALOG_READ_CONCURRENCY = 8;
const NODE_READ_CONCURRENCY = 4;
const SESSION_NAME = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/;
const NODE_NAME = /^(\d{6})\.json$/;
const ATOMIC_NODE_TEMP = /^\.\d{6}\.json\.\d+\.[a-f0-9-]+\.tmp$/;

export type SessionSnapshot = Readonly<{
  meta: SessionMeta;
  head: SessionHead;
  conversation: ConversationTree;
}>;

export type ClaimedSessionSnapshot = SessionSnapshot & Readonly<{ lease: SessionLease }>;

export type SessionCatalogEntry = Readonly<{
  id: string;
  createdAt: string;
  updatedAt: string;
  turns: number;
  preview: string;
  active: boolean;
}>;

export class DurableSessionStore {
  readonly workspaceRoot: string;
  readonly workspaceDigest: string;
  readonly #sessionsRoot: string;
  readonly #bucket: string;

  private constructor(workspaceRoot: string, sessionsRoot: string) {
    this.workspaceRoot = workspaceRoot;
    this.workspaceDigest = digestWorkspace(workspaceRoot);
    this.#sessionsRoot = sessionsRoot;
    this.#bucket = path.join(sessionsRoot, this.workspaceDigest);
  }

  static async open(
    workspaceRoot: string,
    sessionsRoot: string = userDataPath("sessions"),
  ): Promise<DurableSessionStore> {
    const canonical = await realpath(path.resolve(workspaceRoot));
    return new DurableSessionStore(canonical, path.resolve(sessionsRoot));
  }

  async list(limit = 32): Promise<SessionCatalogEntry[]> {
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 64) {
      throw new Error("session catalogue limit is invalid");
    }
    const names = await catalogNames(this.#bucket);
    const catalog: SessionCatalogEntry[] = [];
    for (let start = 0; start < names.length; start += CATALOG_READ_CONCURRENCY) {
      const batch = await Promise.all(names.slice(start, start + CATALOG_READ_CONCURRENCY)
        .map(async (id): Promise<SessionCatalogEntry | undefined> => {
          try {
            const snapshot = await this.load(id);
            const conversation = snapshot.conversation.latestResumable();
            if (conversation === undefined) return undefined;
            return {
              id,
              createdAt: snapshot.meta.createdAt,
              updatedAt: snapshot.head.updatedAt,
              turns: selectedTurnCount(conversation),
              preview: firstUserText(conversation),
              active: await this.#leaseIsActive(id),
            };
          } catch {
            // Corrupt or foreign data never becomes a resume candidate.
            return undefined;
          }
        }));
      catalog.push(...batch.filter((entry) => entry !== undefined));
    }
    return catalog
      .sort((left, right) =>
        right.updatedAt.localeCompare(left.updatedAt) || right.id.localeCompare(left.id)
      )
      .slice(0, limit);
  }

  async load(id: string): Promise<SessionSnapshot> {
    assertSessionId(id);
    const directory = this.#sessionDirectory(id);
    await assertDirectory(directory);
    const meta = decodeMeta(await readJson(
      path.join(directory, "meta.json"),
      SESSION_FILE_LIMITS.metadataBytes,
    ));
    if (
      meta.id !== id || meta.workspaceDigest !== this.workspaceDigest ||
      workspaceKey(meta.workspaceRoot) !== workspaceKey(this.workspaceRoot)
    ) throw new Error("session belongs to a different workspace");
    let head = decodeHead(await readJson(
      path.join(directory, "head.json"),
      SESSION_FILE_LIMITS.metadataBytes,
    ));
    const stored = await readNodes(path.join(directory, "nodes"));
    const ahead = stored.filter((entry) => entry.sequence > head.sequence);
    if (ahead.some((entry) => entry.sequence !== head.sequence + 1) || ahead.length > 1) {
      throw new Error("session has an ambiguous incomplete checkpoint");
    }

    const byId = new Map(stored.map((entry) => [entry.node.id, entry]));
    const headed = byId.get(head.nodeId);
    if (headed === undefined) throw new Error("session head is missing its conversation node");

    if (ahead.length === 0) {
      if (
        headed.sequence !== head.sequence || headed.node.revision !== head.revision ||
        headed.node.parentId !== head.parentId
      ) {
        throw new Error("session head does not match its conversation node");
      }
    } else {
      const candidate = ahead[0] as StoredNode;
      const replacesHead = candidate.node.id === head.nodeId &&
        candidate.node.parentId === head.parentId &&
        candidate.node.revision === head.revision + 1;
      const candidateParent = byId.get(candidate.node.parentId);
      const extendsTree = candidate.node.id === stored.length &&
        candidate.node.revision === 1 && candidateParent !== undefined &&
        candidateParent.sequence <= head.sequence;
      if (!replacesHead && !extendsTree) {
        throw new Error("session checkpoint cannot be recovered safely");
      }
      head = Object.freeze({
        version: SESSION_SCHEMA,
        sequence: candidate.sequence,
        nodeId: candidate.node.id,
        parentId: candidate.node.parentId,
        revision: candidate.node.revision,
        updatedAt: candidate.updatedAt,
      });
      await atomicWrite(path.join(directory, "head.json"), encodeHead(head), {
        mode: FILE_MODE,
        validate: async () => assertDirectory(directory),
      });
    }

    const nodes = stored.map((entry) => entry.node);
    const conversation = ConversationTree.restore(nodes, head.nodeId);
    return Object.freeze({ meta, head, conversation });
  }

  async publish(conversation: ConversationTree): Promise<SessionSnapshot>;
  async publish(conversation: ConversationTree, claim: true): Promise<ClaimedSessionSnapshot>;
  async publish(conversation: ConversationTree, claim?: true):
    Promise<SessionSnapshot | ClaimedSessionSnapshot> {
    const active = conversation.activeNode;
    if (active === undefined) throw new Error("an empty conversation cannot be persisted");
    await this.#ensureBucket();

    const now = new Date().toISOString();
    const id = sessionId(now);
    const token = claim === true ? leaseToken() : undefined;
    const temporary = path.join(this.#bucket, `.${id}.${randomUUID()}.tmp`);
    const target = this.#sessionDirectory(id);
    const meta: SessionMeta = Object.freeze({
      version: SESSION_SCHEMA,
      id,
      workspaceRoot: this.workspaceRoot,
      workspaceDigest: this.workspaceDigest,
      createdAt: now,
    });
    const head: SessionHead = Object.freeze({
      version: SESSION_SCHEMA,
      sequence: conversation.nodes.length,
      nodeId: active.id,
      parentId: active.parentId,
      revision: active.revision,
      updatedAt: now,
    });

    try {
      await makePrivateDirectory(temporary);
      const nodes = path.join(temporary, "nodes");
      await makePrivateDirectory(nodes);
      for (let index = 0; index < conversation.nodes.length; index++) {
        const node = conversation.nodes[index] as TurnNode;
        await atomicWrite(
          path.join(nodes, nodeName(node.id)),
          encodeNode(node, index + 1, now),
          { mode: FILE_MODE },
        );
      }
      await atomicWrite(path.join(temporary, "meta.json"), encodeMeta(meta), { mode: FILE_MODE });
      await atomicWrite(path.join(temporary, "head.json"), encodeHead(head), { mode: FILE_MODE });
      if (token !== undefined) {
        await atomicWrite(path.join(temporary, "active"), token, { mode: FILE_MODE });
      }
      await rename(temporary, target);
    } catch (error) {
      await removeTemporaryDirectory(temporary, this.#bucket);
      throw error;
    }

    const snapshot = Object.freeze({ meta, head, conversation });
    if (token === undefined) return snapshot;
    const lease = sessionLease(id, path.join(target, "active"), token);
    return Object.freeze({ ...snapshot, lease });
  }

  async checkpoint(
    previous: SessionSnapshot,
    conversation: ConversationTree,
  ): Promise<SessionSnapshot> {
    assertSnapshot(previous, this.workspaceRoot, this.workspaceDigest);
    const id = previous.meta.id;
    const directory = this.#sessionDirectory(id);
    await assertDirectory(directory);
    const nodesDirectory = path.join(directory, "nodes");
    const validateNodesDirectory = async (): Promise<void> => {
      await assertDirectory(directory);
      await assertDirectory(nodesDirectory);
    };
    await validateNodesDirectory();
    const currentHead = decodeHead(await readJson(
      path.join(directory, "head.json"),
      SESSION_FILE_LIMITS.metadataBytes,
    ));
    if (!sameHead(currentHead, previous.head)) {
      throw new Error("session head changed after its verified snapshot");
    }

    const active = conversation.activeNode;
    if (active === undefined) throw new Error("an empty conversation cannot be checkpointed");
    if (
      active === previous.conversation.activeNode &&
      conversation.nodes === previous.conversation.nodes
    ) {
      return Object.freeze({ ...previous, conversation });
    }

    const replacesHead = active.id === previous.head.nodeId &&
      active.revision === previous.head.revision + 1 &&
      conversation.nodes.length === previous.conversation.nodes.length;
    const extendsTree = active.id === previous.conversation.nodes.length + 1 &&
      previous.conversation.node(active.parentId) !== undefined && active.revision === 1 &&
      conversation.nodes.length === previous.conversation.nodes.length + 1;
    if (!replacesHead && !extendsTree) {
      throw new Error("session checkpoint does not extend its durable tree");
    }
    assertSharedNodes(previous.conversation, conversation, replacesHead ? active.id : undefined);
    if (extendsTree) {
      await assertMissingNode(path.join(nodesDirectory, nodeName(active.id)));
    }

    const now = new Date().toISOString();
    const head: SessionHead = Object.freeze({
      version: SESSION_SCHEMA,
      sequence: previous.head.sequence + 1,
      nodeId: active.id,
      parentId: active.parentId,
      revision: active.revision,
      updatedAt: now,
    });
    await atomicWrite(
      path.join(nodesDirectory, nodeName(active.id)),
      encodeNode(active, head.sequence, now),
      { mode: FILE_MODE, validate: async () => validateNodesDirectory() },
    );
    await atomicWrite(path.join(directory, "head.json"), encodeHead(head), {
      mode: FILE_MODE,
      validate: async () => assertDirectory(directory),
    });
    return Object.freeze({ meta: previous.meta, head, conversation });
  }

  async claim(id: string): Promise<SessionLease> {
    assertSessionId(id);
    await assertDirectory(this.#sessionDirectory(id));
    const file = path.join(this.#sessionDirectory(id), "active");
    const token = leaseToken();
    for (let attempt = 0; attempt < 2; attempt++) {
      try {
        const handle = await open(file, "wx", FILE_MODE);
        try {
          await handle.writeFile(token, "utf8");
          await handle.sync();
        } finally {
          await handle.close();
        }
        return sessionLease(id, file, token);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
        const owner = await leaseOwner(file);
        if (owner !== undefined && pidIsAlive(owner.pid)) {
          throw new Error("session is already open in another Jecode process");
        }
        if (owner !== undefined) {
          await removeLease(file, owner.token);
        }
      }
    }
    throw new Error("session could not be claimed");
  }

  #sessionDirectory(id: string): string {
    return path.join(this.#bucket, id);
  }

  async #ensureBucket(): Promise<void> {
    await makePrivateDirectory(this.#sessionsRoot);
    await makePrivateDirectory(this.#bucket);
  }

  async #leaseIsActive(id: string): Promise<boolean> {
    const owner = await leaseOwner(path.join(this.#sessionDirectory(id), "active"));
    return owner !== undefined && pidIsAlive(owner.pid);
  }
}

async function catalogNames(directory: string): Promise<string[]> {
  try {
    await assertDirectory(directory);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }

  const names: string[] = [];
  let entries = 0;
  const handle = await opendir(directory);
  for await (const entry of handle) {
    entries++;
    if (entries > MAX_CATALOG_ENTRIES) {
      throw new Error(`session catalogue exceeds ${MAX_CATALOG_ENTRIES} entries`);
    }
    if (entry.isDirectory() && SESSION_NAME.test(entry.name)) names.push(entry.name);
  }
  return names.sort((left, right) => right.localeCompare(left));
}

function assertSharedNodes(
  previous: ConversationTree,
  next: ConversationTree,
  replacedId: number | undefined,
): void {
  for (const node of previous.nodes) {
    if (node.id === replacedId) continue;
    const candidate = next.node(node.id);
    if (candidate !== node) {
      throw new Error("session checkpoint rewrites prior conversation history");
    }
  }
}

function assertSnapshot(
  snapshot: SessionSnapshot,
  workspaceRoot: string,
  workspaceDigest: string,
): void {
  encodeMeta(snapshot.meta);
  encodeHead(snapshot.head);
  assertSessionId(snapshot.meta.id);
  if (
    snapshot.meta.workspaceDigest !== workspaceDigest ||
    workspaceKey(snapshot.meta.workspaceRoot) !== workspaceKey(workspaceRoot)
  ) throw new Error("session snapshot belongs to a different workspace");
  const active = snapshot.conversation.activeNode;
  if (
    active === undefined ||
    active.id !== snapshot.head.nodeId ||
    active.parentId !== snapshot.head.parentId ||
    active.revision !== snapshot.head.revision ||
    snapshot.head.sequence < snapshot.conversation.nodes.length
  ) throw new Error("session snapshot does not match its verified head");
}

function sameHead(left: SessionHead, right: SessionHead): boolean {
  return left.version === right.version &&
    left.sequence === right.sequence &&
    left.nodeId === right.nodeId &&
    left.parentId === right.parentId &&
    left.revision === right.revision &&
    left.updatedAt === right.updatedAt;
}

async function assertMissingNode(file: string): Promise<void> {
  try {
    await lstat(file);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  throw new Error("session has an incomplete node outside its verified snapshot");
}

async function readNodes(directory: string): Promise<StoredNode[]> {
  await assertDirectory(directory);
  const entries = await readdir(directory, { withFileTypes: true });
  const names = entries.filter((entry) => entry.isFile() && NODE_NAME.test(entry.name))
    .map((entry) => entry.name).sort();
  if (names.length === 0 || names.length > CONVERSATION_LIMITS.nodes) {
    throw new Error("session has an invalid conversation size");
  }
  if (
    entries.length > CONVERSATION_LIMITS.nodes + 64 ||
    entries.some((entry) =>
      !entry.isFile() || (!NODE_NAME.test(entry.name) && !ATOMIC_NODE_TEMP.test(entry.name))
    )
  ) {
    throw new Error("session node directory contains unsupported data");
  }
  const stored: StoredNode[] = [];
  const sequences = new Set<number>();
  for (let start = 0; start < names.length; start += NODE_READ_CONCURRENCY) {
    const decoded = await Promise.all(names.slice(start, start + NODE_READ_CONCURRENCY)
      .map(async (name, offset): Promise<StoredNode> => {
        const id = Number(NODE_NAME.exec(name)?.[1]);
        if (id !== start + offset + 1) {
          throw new Error("session conversation nodes are not contiguous");
        }
        const entry = decodeNode(await readJson(
          path.join(directory, name),
          SESSION_FILE_LIMITS.nodeBytes,
        ));
        if (entry.node.id !== id) {
          throw new Error("session conversation node identity is invalid");
        }
        return entry;
      }));
    for (const entry of decoded) {
      if (sequences.has(entry.sequence)) {
        throw new Error("session conversation node identity is invalid");
      }
      sequences.add(entry.sequence);
      stored.push(entry);
    }
  }
  return stored;
}

async function readJson(file: string, limit: number): Promise<unknown> {
  const details = await lstat(file);
  if (details.isSymbolicLink() || !details.isFile() || details.size > limit) {
    throw new Error("session file is unsafe or too large");
  }
  try {
    return JSON.parse(await readFile(file, "utf8"));
  } catch {
    throw new Error("session file is not valid JSON");
  }
}

async function assertDirectory(directory: string): Promise<void> {
  const details = await lstat(directory);
  if (details.isSymbolicLink() || !details.isDirectory()) {
    throw new Error("session path is not a direct directory");
  }
}

async function makePrivateDirectory(directory: string): Promise<void> {
  await mkdir(directory, { recursive: true, mode: DIRECTORY_MODE });
  await assertDirectory(directory);
  if (process.platform !== "win32") await chmod(directory, DIRECTORY_MODE);
}

async function directoryEntries(directory: string) {
  try {
    await assertDirectory(directory);
    return await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
}

async function removeTemporaryDirectory(directory: string, bucket: string): Promise<void> {
  const relative = path.relative(bucket, directory);
  if (
    relative === "" || relative.startsWith("..") || path.isAbsolute(relative) ||
    !path.basename(directory).startsWith(".") || !path.basename(directory).endsWith(".tmp")
  ) throw new Error("refusing to remove an unverified session directory");
  await rm(directory, { recursive: true, force: true });
}

function selectedTurnCount(conversation: ConversationTree): number {
  let count = 0;
  let id = conversation.activeNodeId;
  while (id !== 0) {
    count++;
    id = conversation.node(id)?.parentId ?? 0;
  }
  return count;
}

function firstUserText(conversation: ConversationTree): string {
  for (const message of conversation.history) {
    if (message.role !== "user") continue;
    const text = message.content.find((block) => block.kind === "text")?.text
      .replace(/\s+/gu, " ").trim();
    if (text !== undefined && text !== "") return leadingText(text, 160);
  }
  return "Untitled session";
}

function nodeName(id: number): string {
  return `${String(id).padStart(6, "0")}.json`;
}

function sessionId(now: string): string {
  return `${now.replace(/[-:.]/g, "").replace("Z", "Z")}-${randomUUID()}`;
}

function digestWorkspace(workspaceRoot: string): string {
  return createHash("sha256").update(workspaceKey(workspaceRoot)).digest("hex");
}

function workspaceKey(workspaceRoot: string): string {
  const normalized = path.normalize(workspaceRoot);
  return process.platform === "win32" ? normalized.toLocaleLowerCase("en-US") : normalized;
}

function assertSessionId(id: string): void {
  if (!SESSION_NAME.test(id)) throw new Error("session id is invalid");
}
