// Head-tied resume listing and repairable catalogue-file IO.

import * as path from "node:path";
import { atomicWrite } from "../atomic.ts";
import { assertDirectoryAnchor } from "../directory-anchor.ts";
import type { DirectoryAnchor } from "../directory-anchor.ts";
import { readStableDirectory } from "../stable-directory.ts";
import {
  catalogMatches,
  decodeSessionCatalog,
  encodeSessionCatalog,
  sameSessionHead,
  SESSION_CATALOG_BYTES,
  SESSION_CATALOG_FILE,
  SESSION_CHECKPOINT_FILE,
} from "./catalog.ts";
import type { StoredSessionCatalog } from "./catalog.ts";
import { decodeHead, decodeMeta, SESSION_FILE_LIMITS } from "./codec.ts";
import { FILE_MODE, readJson, SESSION_NAME } from "./files.ts";
import { leaseOwner, pidIsAlive, removeLease } from "./lease.ts";
import type { SessionLease } from "./lease.ts";
import type { SessionSnapshot } from "./snapshot.ts";
import type { SessionBucket } from "./bucket.ts";

const MAX_CATALOG_ENTRIES = 4_096;
const CATALOG_READ_CONCURRENCY = 8;

export type SessionCatalogEntry = Readonly<{
  id: string;
  createdAt: string;
  updatedAt: string;
  turns: number;
  preview: string;
  active: boolean;
}>;

type SessionReader = Readonly<{
  claim(id: string): Promise<SessionLease>;
  load(id: string, recoveryLease: SessionLease): Promise<SessionSnapshot>;
}>;

export class SessionCatalogIO {
  readonly #bucket: SessionBucket;
  readonly #reader: SessionReader;

  constructor(bucket: SessionBucket, reader: SessionReader) {
    this.#bucket = bucket;
    this.#reader = reader;
  }

  async list(limit = 32): Promise<SessionCatalogEntry[]> {
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > 64) {
      throw new Error("session catalogue limit is invalid");
    }
    await this.#bucket.assert();
    const names = await catalogNames(this.#bucket.anchor);
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

  async write(
    id: string,
    catalog: StoredSessionCatalog,
    checkpointToken?: string,
  ): Promise<void> {
    const directory = this.#bucket.directory(id);
    const directoryAnchor = await this.#bucket.captureSession(id);
    const validateDirectory = async (): Promise<void> => {
      await this.#bucket.assertSession(directoryAnchor);
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

  async #leaseIsActive(id: string, directory?: DirectoryAnchor): Promise<boolean> {
    if (directory !== undefined) await this.#bucket.assertSession(directory);
    else await this.#bucket.assert();
    const owner = await leaseOwner(path.join(this.#bucket.directory(id), "active"));
    if (directory !== undefined) await this.#bucket.assertSession(directory);
    return owner !== undefined && pidIsAlive(owner.pid);
  }

  async #catalogEntry(id: string): Promise<SessionCatalogEntry | undefined> {
    try {
      const directory = this.#bucket.directory(id);
      const directoryAnchor = await this.#bucket.captureSession(id);
      const validateDirectory = async (): Promise<void> => {
        await this.#bucket.assertSession(directoryAnchor);
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
          this.#bucket.assertWorkspace(meta, id);
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
      const repairLease = await this.#reader.claim(id);
      let snapshot: SessionSnapshot;
      try {
        snapshot = await this.#reader.load(id, repairLease);
        await this.write(
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
