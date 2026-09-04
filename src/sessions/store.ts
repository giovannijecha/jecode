// Durable, workspace-scoped conversation storage.
//
// Each node is its own atomically replaced file. The head is advanced only
// after that node is durable, so a crash leaves either the prior checkpoint or
// one strictly recoverable mutation -- never an ambiguous partial history.

import { createHash, randomUUID } from "node:crypto";
import {
  lstat,
  opendir,
  realpath,
  rename,
  rm,
} from "node:fs/promises";
import * as path from "node:path";
import { atomicWrite } from "../atomic.ts";
import {
  BoundedFileError,
  readBoundedText,
  stableFileExpectation,
} from "../bounded-file.ts";
import type { StableFileExpectation } from "../bounded-file.ts";
import { CONVERSATION_LIMITS, ConversationTree } from "../conversation.ts";
import type { TurnNode } from "../conversation.ts";
import {
  assertDirectoryAnchor,
  captureDirectDirectory,
  createPrivateDirectory,
  preparePrivateDirectory,
} from "../directory-anchor.ts";
import type { DirectoryAnchor } from "../directory-anchor.ts";
import { sameFileIdentity } from "../file-identity.ts";
import { readStableDirectory } from "../stable-directory.ts";
import { userDataPath } from "../user-data.ts";
import {
  advanceSessionCatalog,
  catalogMatches,
  decodeSessionCatalog,
  encodeSessionCatalog,
  sameSessionHead,
  sessionCatalog,
  SESSION_CATALOG_BYTES,
  SESSION_CATALOG_FILE,
  SESSION_CHECKPOINT_FILE,
} from "./catalog.ts";
import type { StoredSessionCatalog } from "./catalog.ts";
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
  claimLeaseDirectory,
  createLeaseDirectory,
  leaseFromGeneration,
  leaseOwner,
  leaseToken,
  pidIsAlive,
  removeLegacyLeaseExclusive,
  removeLease,
  sessionLease,
  sessionLeaseOwns,
} from "./lease.ts";
import type { SessionLease } from "./lease.ts";

export type { SessionLease } from "./lease.ts";

const DIRECTORY_MODE = 0o700;
const FILE_MODE = 0o600;
const MAX_CATALOG_ENTRIES = 4_096;
const CATALOG_READ_CONCURRENCY = 8;
const MAX_NODE_READ_CONCURRENCY = 8;
const MAX_NODE_READ_IN_FLIGHT_BYTES = 64 * 1_024 * 1_024;
const MAX_SESSION_NODE_BYTES = 192 * 1_024 * 1_024;
const NODE_READ_CONCURRENCY = Math.max(1, Math.min(
  MAX_NODE_READ_CONCURRENCY,
  Math.floor(MAX_NODE_READ_IN_FLIGHT_BYTES / SESSION_FILE_LIMITS.nodeBytes),
));
const SESSION_NAME = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/;
const NODE_NAME = /^(\d{6})\.json$/;
const ATOMIC_NODE_TEMP = /^\.\d{6}\.json\.\d+\.[a-f0-9-]+\.tmp$/;

export type SessionStoreHooks = Readonly<{
  /** Deterministic pre-lease race barrier used by the test suite. */
  beforeCheckpointLease?(): void | Promise<void>;
  /** Deterministic post-lease failure/race barrier used by the test suite. */
  afterCheckpointLease?(): void | Promise<void>;
}>;

export type SessionSnapshot = Readonly<{
  meta: SessionMeta;
  head: SessionHead;
  conversation: ConversationTree;
  catalog: StoredSessionCatalog;
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
  readonly #bucket: string;
  readonly #sessionsAnchor: DirectoryAnchor;
  readonly #bucketAnchor: DirectoryAnchor;
  readonly #hooks: SessionStoreHooks;
  readonly #leaseScope = Object.freeze({});

  private constructor(
    workspaceRoot: string,
    sessionsAnchor: DirectoryAnchor,
    bucketAnchor: DirectoryAnchor,
    hooks: SessionStoreHooks,
  ) {
    this.workspaceRoot = workspaceRoot;
    this.workspaceDigest = digestWorkspace(workspaceRoot);
    this.#bucket = bucketAnchor.path;
    this.#sessionsAnchor = sessionsAnchor;
    this.#bucketAnchor = bucketAnchor;
    this.#hooks = hooks;
  }

  static async open(
    workspaceRoot: string,
    sessionsRoot: string = userDataPath("sessions"),
    hooks: SessionStoreHooks = {},
  ): Promise<DurableSessionStore> {
    const canonical = await realpath(path.resolve(workspaceRoot));
    const digest = digestWorkspace(canonical);
    const sessionsAnchor = await preparePrivateDirectory(
      path.resolve(sessionsRoot),
      "session storage root",
      DIRECTORY_MODE,
    );
    const bucketAnchor = await preparePrivateDirectory(
      path.join(sessionsAnchor.path, digest),
      "workspace session directory",
      DIRECTORY_MODE,
    );
    return new DurableSessionStore(canonical, sessionsAnchor, bucketAnchor, hooks);
  }

  async list(limit = 32): Promise<SessionCatalogEntry[]> {
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 64) {
      throw new Error("session catalogue limit is invalid");
    }
    await this.#ensureBucket();
    const names = await catalogNames(this.#bucketAnchor);
    const catalog: SessionCatalogEntry[] = [];
    for (let start = 0; start < names.length; start += CATALOG_READ_CONCURRENCY) {
      const batch = await Promise.all(names.slice(start, start + CATALOG_READ_CONCURRENCY)
        .map(async (id): Promise<SessionCatalogEntry | undefined> => {
          return await this.#catalogEntry(id);
        }));
      catalog.push(...batch.filter((entry) => entry !== undefined));
    }
    return catalog
      .sort((left, right) =>
        right.updatedAt.localeCompare(left.updatedAt) || right.id.localeCompare(left.id)
      )
      .slice(0, limit);
  }

  async load(id: string, recoveryLease?: SessionLease): Promise<SessionSnapshot> {
    assertSessionId(id);
    await this.#ensureBucket();
    const directory = this.#sessionDirectory(id);
    const directoryAnchor = await captureDirectDirectory(directory, "session directory");
    const nodesAnchor = await captureDirectDirectory(
      path.join(directory, "nodes"),
      "session node directory",
    );
    const validateSession = async (): Promise<void> => {
      await Promise.all([
        this.#ensureBucket(),
        assertDirectoryAnchor(directoryAnchor),
        assertDirectoryAnchor(nodesAnchor),
      ]);
    };
    const meta = decodeMeta(await readJson(
      path.join(directory, "meta.json"),
      SESSION_FILE_LIMITS.metadataBytes,
      undefined,
      validateSession,
    ));
    assertSessionWorkspace(meta, id, this.workspaceRoot, this.workspaceDigest);
    const persistedHead = decodeHead(await readJson(
      path.join(directory, "head.json"),
      SESSION_FILE_LIMITS.metadataBytes,
      undefined,
      validateSession,
    ));
    let head = persistedHead;
    const stored = await readNodes(nodesAnchor, validateSession);
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
      if (
        recoveryLease === undefined ||
        !sessionLeaseOwns(recoveryLease, id, this.#leaseScope)
      ) {
        throw new Error("session recovery requires exclusive ownership");
      }
      await recoveryLease.assertOwned();
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
        validate: async (phase) => {
          await validateSession();
          if (phase !== "before-rename") return;
          await recoveryLease.assertOwned();
          const current = decodeHead(await readJson(
            path.join(directory, "head.json"),
            SESSION_FILE_LIMITS.metadataBytes,
            undefined,
            validateSession,
          ));
          if (!sameSessionHead(current, persistedHead)) {
            throw new Error("session head changed while recovering its checkpoint");
          }
        },
      });
    }

    const nodes = stored.map((entry) => entry.node);
    const conversation = ConversationTree.restore(nodes, head.nodeId);
    const catalog = sessionCatalog(meta, head, conversation);
    return Object.freeze({ meta, head, conversation, catalog });
  }

  async publish(conversation: ConversationTree): Promise<SessionSnapshot>;
  async publish(
    conversation: ConversationTree,
    claim: true,
    reservedId?: string,
  ): Promise<ClaimedSessionSnapshot>;
  async publish(conversation: ConversationTree, claim?: true, reservedId?: string):
    Promise<SessionSnapshot | ClaimedSessionSnapshot> {
    const active = conversation.activeNode;
    if (active === undefined) throw new Error("an empty conversation cannot be persisted");
    await this.#ensureBucket();

    const now = new Date().toISOString();
    const id = reservedId ?? reserveSessionId(now);
    assertSessionId(id);
    const token = claim === true ? leaseToken() : undefined;
    let leaseGeneration: Awaited<ReturnType<typeof createLeaseDirectory>> | undefined;
    let temporaryAnchor: DirectoryAnchor | undefined;
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
    const catalog = sessionCatalog(meta, head, conversation);

    try {
      temporaryAnchor = await createPrivateDirectory(
        temporary,
        "temporary session directory",
        DIRECTORY_MODE,
      );
      const nodes = path.join(temporary, "nodes");
      const nodesAnchor = await createPrivateDirectory(
        nodes,
        "temporary session node directory",
        DIRECTORY_MODE,
      );
      const validateTemporary = async (): Promise<void> => {
        await this.#ensureBucket();
        await assertDirectoryAnchor(temporaryAnchor as DirectoryAnchor);
        await assertDirectoryAnchor(nodesAnchor);
      };
      for (let index = 0; index < conversation.nodes.length; index++) {
        const node = conversation.nodes[index] as TurnNode;
        await atomicWrite(
          path.join(nodes, nodeName(node.id)),
          encodeNode(node, index + 1, now),
          { mode: FILE_MODE, validate: async () => validateTemporary() },
        );
      }
      await atomicWrite(path.join(temporary, "meta.json"), encodeMeta(meta), {
        mode: FILE_MODE,
        validate: async () => validateTemporary(),
      });
      await atomicWrite(path.join(temporary, "head.json"), encodeHead(head), {
        mode: FILE_MODE,
        validate: async () => validateTemporary(),
      });
      await atomicWrite(
        path.join(temporary, SESSION_CATALOG_FILE),
        encodeSessionCatalog(catalog),
        { mode: FILE_MODE, validate: async () => validateTemporary() },
      );
      if (token !== undefined) {
        leaseGeneration = await createLeaseDirectory(path.join(temporary, "active"), token);
      }
      await validateTemporary();
      await rename(temporary, target);
      await this.#ensureBucket();
      const targetAnchor = await captureDirectDirectory(target, "session directory");
      if (!sameFileIdentity(temporaryAnchor.identity, targetAnchor.identity)) {
        throw new Error("session directory changed while publishing");
      }
    } catch (error) {
      await removeTemporaryDirectory(temporary, this.#bucketAnchor, temporaryAnchor)
        .catch(() => undefined);
      throw error;
    }

    const snapshot = Object.freeze({ meta, head, conversation, catalog });
    if (token === undefined) return snapshot;
    if (leaseGeneration === undefined) throw new Error("session lease was not initialized");
    const lease = sessionLease(
      id,
      this.#leaseScope,
      leaseFromGeneration(path.join(target, "active"), leaseGeneration),
    );
    return Object.freeze({ ...snapshot, lease });
  }

  async checkpoint(
    previous: SessionSnapshot,
    conversation: ConversationTree,
    lease: SessionLease,
  ): Promise<SessionSnapshot> {
    assertSnapshot(previous, this.workspaceRoot, this.workspaceDigest);
    const id = previous.meta.id;
    if (!sessionLeaseOwns(lease, id, this.#leaseScope)) {
      throw new Error("session checkpoint requires exclusive ownership");
    }
    await lease.assertOwned();
    const directory = this.#sessionDirectory(id);
    const directoryAnchor = await this.#sessionAnchor(id);
    const nodesDirectory = path.join(directory, "nodes");
    const nodesAnchor = await captureDirectDirectory(nodesDirectory, "session node directory");
    const validateNodesDirectory = async (): Promise<void> => {
      await Promise.all([
        lease.assertOwned(),
        this.#ensureBucket(),
        assertDirectoryAnchor(directoryAnchor),
        assertDirectoryAnchor(nodesAnchor),
      ]);
    };
    await validateNodesDirectory();
    const headFile = path.join(directory, "head.json");
    const headExpectation = stableFileExpectation(await lstat(headFile, { bigint: true }));
    await validateNodesDirectory();
    const currentHead = decodeHead(await readJson(
      headFile,
      SESSION_FILE_LIMITS.metadataBytes,
      headExpectation,
    ));
    await validateNodesDirectory();
    if (!sameSessionHead(currentHead, previous.head)) {
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
    const catalog = advanceSessionCatalog(previous.catalog, previous.meta, head, conversation);
    const checkpointToken = leaseToken();
    const checkpointFile = path.join(directory, SESSION_CHECKPOINT_FILE);
    await this.#hooks.beforeCheckpointLease?.();
    await validateNodesDirectory();
    const checkpointLease = await claimLeaseDirectory(checkpointFile, checkpointToken);
    if (checkpointLease === undefined) throw new Error("session checkpoint is already active");
    let primaryFailure: unknown;
    try {
      await this.#hooks.afterCheckpointLease?.();
      const assertBaseHead = async (): Promise<void> => {
        await Promise.all([checkpointLease.assertOwned(), validateNodesDirectory()]);
        let verified: SessionHead;
        try {
          verified = decodeHead(await readJson(
            headFile,
            SESSION_FILE_LIMITS.metadataBytes,
            headExpectation,
          ));
        } catch (error) {
          throw new Error("session head changed after its verified snapshot", { cause: error });
        }
        await Promise.all([checkpointLease.assertOwned(), validateNodesDirectory()]);
        if (!sameSessionHead(verified, previous.head)) {
          throw new Error("session head changed after its verified snapshot");
        }
      };
      await assertBaseHead();
      if (extendsTree) {
        await assertMissingNode(
          path.join(nodesDirectory, nodeName(active.id)),
          validateNodesDirectory,
        );
      }
      await atomicWrite(
        path.join(nodesDirectory, nodeName(active.id)),
        encodeNode(active, head.sequence, now),
        {
          mode: FILE_MODE,
          validate: async (phase) => {
            await assertBaseHead();
            if (extendsTree && phase === "before-rename") {
              await assertMissingNode(
                path.join(nodesDirectory, nodeName(active.id)),
                validateNodesDirectory,
              );
            }
          },
        },
      );
      await atomicWrite(path.join(directory, "head.json"), encodeHead(head), {
        mode: FILE_MODE,
        validate: async () => assertBaseHead(),
      });
      await this.#writeCatalog(previous.meta.id, catalog, checkpointToken).catch(() => undefined);
      return Object.freeze({ meta: previous.meta, head, conversation, catalog });
    } catch (error) {
      primaryFailure = error;
      throw error;
    } finally {
      try {
        if (!(await checkpointLease.release()) && primaryFailure === undefined) {
          throw new Error("session checkpoint ownership was lost");
        }
      } catch (error) {
        if (primaryFailure === undefined) throw error;
      }
    }
  }

  async claim(id: string): Promise<SessionLease> {
    assertSessionId(id);
    const directory = this.#sessionDirectory(id);
    const directoryAnchor = await this.#sessionAnchor(id);
    const validateDirectory = async (): Promise<void> => {
      await this.#assertSessionAnchor(directoryAnchor);
    };
    await validateDirectory();
    const file = path.join(directory, "active");
    const previous = await leaseOwner(file);
    await validateDirectory();
    if (previous?.legacy === true) {
      if (pidIsAlive(previous.pid)) {
        throw new Error("session is already open in an older Jecode process");
      }
      throw new Error(
        "session has a stale legacy active marker; close older Jecode processes and remove it before retrying",
      );
    }
    const token = leaseToken();
    const lease = await claimLeaseDirectory(file, token);
    if (lease === undefined) {
      throw new Error("session is already open in another Jecode process");
    }
    const owned = sessionLease(id, this.#leaseScope, lease);
    const checkpointFile = path.join(directory, SESSION_CHECKPOINT_FILE);
    try {
      await owned.assertOwned();
      await validateDirectory();
      const checkpoint = await leaseOwner(checkpointFile);
      if (checkpoint !== undefined && pidIsAlive(checkpoint.pid)) {
        throw new Error("session has a live checkpoint from another Jecode process");
      }
      if (checkpoint?.legacy === true) {
        if (!await removeLegacyLeaseExclusive(checkpointFile, checkpoint.token, owned)) {
          throw new Error("session legacy checkpoint changed during migration");
        }
      } else if (
        checkpoint !== undefined &&
        !await removeLease(checkpointFile, checkpoint.token)
      ) {
        throw new Error("session checkpoint changed during recovery");
      }
      await owned.assertOwned();
      await validateDirectory();
    } catch (error) {
      await owned.close().catch(() => undefined);
      throw error;
    }
    return owned;
  }

  #sessionDirectory(id: string): string {
    return path.join(this.#bucket, id);
  }

  async #sessionAnchor(id: string): Promise<DirectoryAnchor> {
    await this.#ensureBucket();
    return captureDirectDirectory(this.#sessionDirectory(id), "session directory");
  }

  async #assertSessionAnchor(anchor: DirectoryAnchor): Promise<void> {
    await Promise.all([this.#ensureBucket(), assertDirectoryAnchor(anchor)]);
  }

  async #ensureBucket(): Promise<void> {
    await Promise.all([
      assertDirectoryAnchor(this.#sessionsAnchor),
      assertDirectoryAnchor(this.#bucketAnchor),
    ]);
  }

  async #leaseIsActive(id: string, directory?: DirectoryAnchor): Promise<boolean> {
    if (directory !== undefined) await this.#assertSessionAnchor(directory);
    else await this.#ensureBucket();
    const owner = await leaseOwner(path.join(this.#sessionDirectory(id), "active"));
    if (directory !== undefined) await this.#assertSessionAnchor(directory);
    return owner !== undefined && pidIsAlive(owner.pid);
  }

  async #catalogEntry(id: string): Promise<SessionCatalogEntry | undefined> {
    try {
      const directory = this.#sessionDirectory(id);
      const directoryAnchor = await this.#sessionAnchor(id);
      const validateDirectory = async (): Promise<void> => {
        await this.#assertSessionAnchor(directoryAnchor);
      };
      const checkpointFile = path.join(directory, SESSION_CHECKPOINT_FILE);

      // A second head read closes the only useful race: a checkpoint landing
      // between the small record reads. A changing marker gets one retry.
      for (let attempt = 0; attempt < 2; attempt++) {
        await validateDirectory();
        const checkpointBefore = await leaseOwner(checkpointFile);
        try {
          const [metaValue, headValue, catalogValue] = await Promise.all([
            readJson(
              path.join(directory, "meta.json"),
              SESSION_FILE_LIMITS.metadataBytes,
              undefined,
              validateDirectory,
            ),
            readJson(
              path.join(directory, "head.json"),
              SESSION_FILE_LIMITS.metadataBytes,
              undefined,
              validateDirectory,
            ),
            readJson(
              path.join(directory, SESSION_CATALOG_FILE),
              SESSION_CATALOG_BYTES,
              undefined,
              validateDirectory,
            ),
          ]);
          const meta = decodeMeta(metaValue);
          const head = decodeHead(headValue);
          const storedCatalog = decodeSessionCatalog(catalogValue);
          assertSessionWorkspace(meta, id, this.workspaceRoot, this.workspaceDigest);
          const confirmedHead = decodeHead(await readJson(
            path.join(directory, "head.json"),
            SESSION_FILE_LIMITS.metadataBytes,
            undefined,
            validateDirectory,
          ));
          await validateDirectory();
          const checkpointAfter = await leaseOwner(checkpointFile);
          if (!sameLease(checkpointBefore, checkpointAfter)) continue;
          if (
            !sameSessionHead(head, confirmedHead) ||
            !catalogMatches(storedCatalog, meta, head)
          ) break;
          if (checkpointAfter !== undefined && !pidIsAlive(checkpointAfter.pid)) break;
          const active = await this.#leaseIsActive(id, directoryAnchor) ||
            checkpointAfter !== undefined;
          return catalogEntry(storedCatalog, active);
        } catch {
          break;
        }
      }

      // Missing, stale, or malformed summaries are rebuilt only while the
      // session is idle. Selecting a session still performs this strict load.
      const checkpoint = await leaseOwner(checkpointFile);
      if (
        await this.#leaseIsActive(id, directoryAnchor) ||
        (checkpoint !== undefined && pidIsAlive(checkpoint.pid))
      ) return undefined;
      const repairLease = await this.claim(id);
      let snapshot: SessionSnapshot;
      try {
        snapshot = await this.load(id, repairLease);
        await this.#writeCatalog(
          id,
          snapshot.catalog,
          checkpoint?.legacy === true ? undefined : checkpoint?.token,
        ).catch(() => undefined);
      } finally {
        await repairLease.close();
      }
      const currentCheckpoint = await leaseOwner(checkpointFile);
      const active = await this.#leaseIsActive(id, directoryAnchor) ||
        (currentCheckpoint !== undefined && pidIsAlive(currentCheckpoint.pid));
      return catalogEntry(snapshot.catalog, active);
    } catch {
      // Corrupt, unsafe, active-without-a-summary, or foreign data never
      // becomes a resume candidate.
      return undefined;
    }
  }

  async #writeCatalog(
    id: string,
    catalog: StoredSessionCatalog,
    checkpointToken?: string,
  ): Promise<void> {
    const directory = this.#sessionDirectory(id);
    const directoryAnchor = await this.#sessionAnchor(id);
    const validateDirectory = async (): Promise<void> => {
      await this.#assertSessionAnchor(directoryAnchor);
    };
    const validate = async (): Promise<void> => {
      await validateDirectory();
      const currentHead = decodeHead(await readJson(
        path.join(directory, "head.json"),
        SESSION_FILE_LIMITS.metadataBytes,
        undefined,
        validateDirectory,
      ));
      if (!sameSessionHead(currentHead, catalog.head)) {
        throw new Error("session head changed while updating its catalogue");
      }
    };
    await atomicWrite(
      path.join(directory, SESSION_CATALOG_FILE),
      encodeSessionCatalog(catalog),
      { mode: FILE_MODE, validate },
    );
    if (checkpointToken !== undefined) {
      await validateDirectory();
      await removeLease(path.join(directory, SESSION_CHECKPOINT_FILE), checkpointToken);
      await validateDirectory();
    }
  }
}

async function catalogNames(directory: DirectoryAnchor): Promise<string[]> {
  await assertDirectoryAnchor(directory);
  const inspected = await readStableDirectory(directory.path, directory.path, {
    maxEntries: MAX_CATALOG_ENTRIES + 1,
  });
  if (inspected.capped || inspected.entries.length > MAX_CATALOG_ENTRIES) {
    throw new Error(`session catalogue exceeds ${MAX_CATALOG_ENTRIES} entries`);
  }
  return inspected.entries
    .filter((entry) => entry.kind === "directory" && SESSION_NAME.test(entry.name))
    .map((entry) => entry.name)
    .sort((left, right) => right.localeCompare(left));
}

function catalogEntry(
  catalog: StoredSessionCatalog,
  active: boolean,
): SessionCatalogEntry | undefined {
  if (catalog.resumeNodeId === 0) return undefined;
  return Object.freeze({
    id: catalog.id,
    createdAt: catalog.createdAt,
    updatedAt: catalog.head.updatedAt,
    turns: catalog.turns,
    preview: catalog.preview,
    active,
  });
}

function sameLease(
  left: Awaited<ReturnType<typeof leaseOwner>>,
  right: Awaited<ReturnType<typeof leaseOwner>>,
): boolean {
  return left?.token === right?.token && left?.legacy === right?.legacy;
}

function assertSessionWorkspace(
  meta: SessionMeta,
  id: string,
  workspaceRoot: string,
  workspaceDigest: string,
): void {
  if (
    meta.id !== id || meta.workspaceDigest !== workspaceDigest ||
    workspaceKey(meta.workspaceRoot) !== workspaceKey(workspaceRoot)
  ) throw new Error("session belongs to a different workspace");
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
  encodeSessionCatalog(snapshot.catalog);
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
  if (!catalogMatches(snapshot.catalog, snapshot.meta, snapshot.head)) {
    throw new Error("session snapshot does not match its verified catalogue");
  }
}

async function assertMissingNode(
  file: string,
  validate?: () => Promise<void>,
): Promise<void> {
  await validate?.();
  try {
    await lstat(file);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      await validate?.();
      return;
    }
    throw error;
  }
  await validate?.();
  throw new Error("session has an incomplete node outside its verified snapshot");
}

async function readNodes(
  directory: DirectoryAnchor,
  validate: () => Promise<void>,
): Promise<StoredNode[]> {
  await validate();
  await assertDirectoryAnchor(directory);
  const names: string[] = [];
  let entries = 0;
  const handle = await opendir(directory.path);
  for await (const entry of handle) {
    entries++;
    if (entries > CONVERSATION_LIMITS.nodes + 64) {
      throw new Error("session node directory contains unsupported data");
    }
    if (
      !entry.isFile() || (!NODE_NAME.test(entry.name) && !ATOMIC_NODE_TEMP.test(entry.name))
    ) {
      throw new Error("session node directory contains unsupported data");
    }
    if (NODE_NAME.test(entry.name)) names.push(entry.name);
  }
  await validate();
  await assertDirectoryAnchor(directory);
  names.sort();
  if (names.length === 0 || names.length > CONVERSATION_LIMITS.nodes) {
    throw new Error("session has an invalid conversation size");
  }
  const files: Array<Readonly<{
    name: string;
    id: number;
    expected: StableFileExpectation;
  }>> = [];
  let storedBytes = 0;
  for (let index = 0; index < names.length; index++) {
    const name = names[index] as string;
    const id = Number(NODE_NAME.exec(name)?.[1]);
    if (id !== index + 1) {
      throw new Error("session conversation nodes are not contiguous");
    }
    const details = await lstat(path.join(directory.path, name), { bigint: true });
    if (
      details.isSymbolicLink() || !details.isFile() || details.size < 0n ||
      details.size > BigInt(SESSION_FILE_LIMITS.nodeBytes)
    ) throw new Error("session node file is unsafe or too large");
    storedBytes += Number(details.size);
    if (storedBytes > MAX_SESSION_NODE_BYTES) {
      throw new Error("session node files exceed their aggregate storage limit");
    }
    files.push({ name, id, expected: stableFileExpectation(details) });
  }
  await validate();
  const stored: StoredNode[] = [];
  const sequences = new Set<number>();
  let messageCodeUnits = 0;
  let transcriptCodeUnits = 0;
  let contextCodeUnits = 0;
  for (let start = 0; start < files.length; start += NODE_READ_CONCURRENCY) {
    const decoded = await Promise.all(files.slice(start, start + NODE_READ_CONCURRENCY)
      .map(async ({ name, id, expected }): Promise<StoredNode> => {
        const entry = decodeNode(await readJson(
          path.join(directory.path, name),
          SESSION_FILE_LIMITS.nodeBytes,
          expected,
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
      messageCodeUnits += JSON.stringify(entry.node.messages).length;
      transcriptCodeUnits += JSON.stringify(entry.node.blocks).length;
      contextCodeUnits += entry.node.context?.summary.length ?? 0;
      if (
        messageCodeUnits > CONVERSATION_LIMITS.messageCodeUnits ||
        transcriptCodeUnits > CONVERSATION_LIMITS.transcriptCodeUnits ||
        contextCodeUnits > CONVERSATION_LIMITS.contextCodeUnits
      ) {
        throw new Error("session conversation exceeds its aggregate limit");
      }
      sequences.add(entry.sequence);
      stored.push(entry);
    }
  }
  await validate();
  return stored;
}

async function readJson(
  file: string,
  limit: number,
  expected?: StableFileExpectation,
  validate?: () => Promise<void>,
): Promise<unknown> {
  try {
    return JSON.parse(await readBoundedText(file, limit, {
      label: "session file",
      expected,
      validate,
    }));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("session file is not valid JSON");
    }
    if (error instanceof BoundedFileError) {
      throw new Error("session file is unsafe or too large, or changed while opening");
    }
    throw error;
  }
}

async function removeTemporaryDirectory(
  directory: string,
  bucket: DirectoryAnchor,
  expected?: DirectoryAnchor,
): Promise<void> {
  const relative = path.relative(bucket.path, directory);
  if (
    relative === "" || relative.startsWith("..") || path.isAbsolute(relative) ||
    !path.basename(directory).startsWith(".") || !path.basename(directory).endsWith(".tmp")
  ) throw new Error("refusing to remove an unverified session directory");
  await assertDirectoryAnchor(bucket);
  let observed: DirectoryAnchor;
  try {
    observed = await captureDirectDirectory(directory, "temporary session directory");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  if (
    expected !== undefined &&
    !sameFileIdentity(expected.identity, observed.identity)
  ) throw new Error("refusing to remove a replaced session directory");

  const quarantine = path.join(
    bucket.path,
    `.discard-${process.pid}-${randomUUID()}.tmp`,
  );
  await assertDirectoryAnchor(bucket);
  await rename(directory, quarantine);
  const moved = await captureDirectDirectory(quarantine, "discarded session directory");
  if (!sameFileIdentity(observed.identity, moved.identity)) {
    throw new Error("session cleanup target changed during quarantine");
  }
  await assertDirectoryAnchor(bucket);
  await assertDirectoryAnchor(moved);
  await rm(quarantine, { recursive: true });
}

function nodeName(id: number): string {
  return `${String(id).padStart(6, "0")}.json`;
}

export function reserveSessionId(now: string = new Date().toISOString()): string {
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
