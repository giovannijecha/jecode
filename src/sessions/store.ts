// Durable, workspace-scoped conversation storage.
//
// Each node is its own atomically replaced file. The head is advanced only
// after that node is durable, so a crash leaves either the prior checkpoint or
// one strictly recoverable mutation -- never an ambiguous partial history.

import { randomUUID } from "node:crypto";
import { lstat, rename } from "node:fs/promises";
import * as path from "node:path";
import { atomicWrite } from "../atomic.ts";
import { stableFileExpectation } from "../bounded-file.ts";
import type { ConversationTree, TurnNode } from "../conversation.ts";
import {
  assertDirectoryAnchor,
  captureDirectDirectory,
  createPrivateDirectory,
} from "../directory-anchor.ts";
import type { DirectoryAnchor } from "../directory-anchor.ts";
import { sameFileIdentity } from "../file-identity.ts";
import { userDataPath } from "../user-data.ts";
import { assertSessionId, SessionBucket } from "./bucket.ts";
import { SessionCatalogIO } from "./catalog-io.ts";
import type { SessionCatalogEntry } from "./catalog-io.ts";
import {
  advanceSessionCatalog,
  encodeSessionCatalog,
  sameSessionHead,
  sessionCatalog,
  SESSION_CATALOG_FILE,
  SESSION_CHECKPOINT_FILE,
} from "./catalog.ts";
import {
  decodeHead,
  encodeHead,
  encodeMeta,
  encodeNode,
  SESSION_FILE_LIMITS,
  SESSION_SCHEMA,
} from "./codec.ts";
import type { SessionHead, SessionMeta } from "./codec.ts";
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
import {
  assertMissingNode,
  DIRECTORY_MODE,
  FILE_MODE,
  nodeName,
  readJson,
  removeTemporaryDirectory,
} from "./files.ts";
import { loadSession } from "./load.ts";
import { assertSharedNodes, assertSnapshot } from "./snapshot.ts";
import type { ClaimedSessionSnapshot, SessionSnapshot } from "./snapshot.ts";

export type { SessionLease } from "./lease.ts";
export type { SessionCatalogEntry } from "./catalog-io.ts";
export type { ClaimedSessionSnapshot, SessionSnapshot } from "./snapshot.ts";

export type SessionStoreHooks = Readonly<{
  /** Deterministic pre-lease race barrier used by the test suite. */
  beforeCheckpointLease?(): void | Promise<void>;
  /** Deterministic post-lease failure/race barrier used by the test suite. */
  afterCheckpointLease?(): void | Promise<void>;
}>;

export class DurableSessionStore {
  readonly workspaceRoot: string;
  readonly workspaceDigest: string;
  readonly #bucket: SessionBucket;
  readonly #catalog: SessionCatalogIO;
  readonly #hooks: SessionStoreHooks;
  readonly #leaseScope = Object.freeze({});

  private constructor(bucket: SessionBucket, hooks: SessionStoreHooks) {
    this.workspaceRoot = bucket.workspaceRoot;
    this.workspaceDigest = bucket.workspaceDigest;
    this.#bucket = bucket;
    this.#hooks = hooks;
    this.#catalog = new SessionCatalogIO(bucket, {
      claim: (id) => this.claim(id),
      load: (id, lease) => this.load(id, lease),
    });
  }

  static async open(
    workspaceRoot: string,
    sessionsRoot: string = userDataPath("sessions"),
    hooks: SessionStoreHooks = {},
  ): Promise<DurableSessionStore> {
    return new DurableSessionStore(await SessionBucket.open(workspaceRoot, sessionsRoot), hooks);
  }

  async list(limit = 32): Promise<SessionCatalogEntry[]> {
    return this.#catalog.list(limit);
  }

  async load(id: string, recoveryLease?: SessionLease): Promise<SessionSnapshot> {
    return loadSession(this.#bucket, id, this.#leaseScope, recoveryLease);
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
    await this.#bucket.assert();

    const now = new Date().toISOString();
    const id = reservedId ?? reserveSessionId(now);
    assertSessionId(id);
    const token = claim === true ? leaseToken() : undefined;
    let leaseGeneration: Awaited<ReturnType<typeof createLeaseDirectory>> | undefined;
    let temporaryAnchor: DirectoryAnchor | undefined;
    const temporary = path.join(this.#bucket.anchor.path, `.${id}.${randomUUID()}.tmp`);
    const target = this.#bucket.directory(id);
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
        await this.#bucket.assert();
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
      await this.#bucket.assert();
      const targetAnchor = await captureDirectDirectory(target, "session directory");
      if (!sameFileIdentity(temporaryAnchor.identity, targetAnchor.identity)) {
        throw new Error("session directory changed while publishing");
      }
    } catch (error) {
      await removeTemporaryDirectory(temporary, this.#bucket.anchor, temporaryAnchor)
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
    const directory = this.#bucket.directory(id);
    const directoryAnchor = await this.#bucket.captureSession(id);
    const nodesDirectory = path.join(directory, "nodes");
    const nodesAnchor = await captureDirectDirectory(nodesDirectory, "session node directory");
    const validateNodesDirectory = async (): Promise<void> => {
      await Promise.all([
        lease.assertOwned(),
        this.#bucket.assert(),
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
      // Publish the next head's index first. Until the head commits, listing
      // treats it as suspect and loads the tree after ownership is released.
      // A failed node/head write must never leave an apparently current index.
      await atomicWrite(path.join(directory, SESSION_CATALOG_FILE), encodeSessionCatalog(catalog), {
        mode: FILE_MODE,
        validate: async () => assertBaseHead(),
      });
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
    const directory = this.#bucket.directory(id);
    const directoryAnchor = await this.#bucket.captureSession(id);
    const validateDirectory = async (): Promise<void> => {
      await this.#bucket.assertSession(directoryAnchor);
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
}

export function reserveSessionId(now: string = new Date().toISOString()): string {
  return `${now.replace(/[-:.]/g, "").replace("Z", "Z")}-${randomUUID()}`;
}
