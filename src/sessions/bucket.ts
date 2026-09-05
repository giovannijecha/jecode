// Stable workspace identity and directory anchors shared by session operations.

import { createHash } from "node:crypto";
import { realpath } from "node:fs/promises";
import * as path from "node:path";
import {
  assertDirectoryAnchor,
  captureDirectDirectory,
  preparePrivateDirectory,
} from "../directory-anchor.ts";
import type { DirectoryAnchor } from "../directory-anchor.ts";
import type { SessionMeta } from "./codec.ts";
import { DIRECTORY_MODE, SESSION_NAME } from "./files.ts";

export class SessionBucket {
  readonly workspaceRoot: string;
  readonly workspaceDigest: string;
  readonly anchor: DirectoryAnchor;
  readonly #root: DirectoryAnchor;

  private constructor(workspaceRoot: string, root: DirectoryAnchor, anchor: DirectoryAnchor) {
    this.workspaceRoot = workspaceRoot;
    this.workspaceDigest = digestWorkspace(workspaceRoot);
    this.#root = root;
    this.anchor = anchor;
    Object.freeze(this);
  }

  static async open(workspaceRoot: string, sessionsRoot: string): Promise<SessionBucket> {
    const canonical = await realpath(path.resolve(workspaceRoot));
    const digest = digestWorkspace(canonical);
    const root = await preparePrivateDirectory(
      path.resolve(sessionsRoot),
      "session storage root",
      DIRECTORY_MODE,
    );
    const anchor = await preparePrivateDirectory(
      path.join(root.path, digest),
      "workspace session directory",
      DIRECTORY_MODE,
    );
    return new SessionBucket(canonical, root, anchor);
  }

  directory(id: string): string {
    return path.join(this.anchor.path, id);
  }

  async captureSession(id: string): Promise<DirectoryAnchor> {
    await this.assert();
    return captureDirectDirectory(this.directory(id), "session directory");
  }

  async assertSession(anchor: DirectoryAnchor): Promise<void> {
    await Promise.all([this.assert(), assertDirectoryAnchor(anchor)]);
  }

  async assert(): Promise<void> {
    await Promise.all([assertDirectoryAnchor(this.#root), assertDirectoryAnchor(this.anchor)]);
  }

  assertWorkspace(meta: SessionMeta, id: string): void {
    if (
      meta.id !== id || meta.workspaceDigest !== this.workspaceDigest ||
      workspaceKey(meta.workspaceRoot) !== workspaceKey(this.workspaceRoot)
    ) throw new Error("session belongs to a different workspace");
  }
}

function digestWorkspace(workspaceRoot: string): string {
  return createHash("sha256").update(workspaceKey(workspaceRoot)).digest("hex");
}

export function workspaceKey(workspaceRoot: string): string {
  const normalized = path.normalize(workspaceRoot);
  return process.platform === "win32" ? normalized.toLocaleLowerCase("en-US") : normalized;
}

export function assertSessionId(id: string): void {
  if (!SESSION_NAME.test(id)) throw new Error("session id is invalid");
}
