import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { createProcessLeaseDirectory, processLease } from "../src/process-lease.ts";
import {
  leaseOwner,
  removeLegacyLeaseExclusive,
  sessionLease,
} from "../src/sessions/lease.ts";

test("legacy lease inspection fails closed when the marker disappears", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-legacy-lease-"));
  const file = path.join(root, "active");
  try {
    await writeFile(file, "123:legacy", "utf8");
    await assert.rejects(
      leaseOwner(file, { afterLegacyStat: async () => unlink(file) }),
      /session lease is unsafe or too large, or invalid/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("legacy lease cleanup never removes a replacement marker", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-legacy-lease-"));
  const file = path.join(root, "active");
  const ownerDirectory = path.join(root, "owner");
  let owner: ReturnType<typeof sessionLease> | undefined;
  try {
    await writeFile(file, "123:observed", "utf8");
    const generation = await createProcessLeaseDirectory(
      ownerDirectory,
      `${process.pid}:00000000-0000-4000-8000-000000000123`,
    );
    owner = sessionLease("session", {}, processLease(ownerDirectory, generation));
    assert.equal(await removeLegacyLeaseExclusive(file, "123:different", owner), false);
    assert.deepEqual(await leaseOwner(file), {
      pid: 123,
      token: "123:observed",
      legacy: true,
    });
  } finally {
    await owner?.close().catch(() => undefined);
    await rm(root, { recursive: true, force: true });
  }
});

test("legacy lease cleanup rejects structural ownership lookalikes", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-legacy-lease-"));
  const file = path.join(root, "active");
  try {
    await writeFile(file, "123:observed", "utf8");
    await assert.rejects(
      removeLegacyLeaseExclusive(file, "123:observed", {
        id: "session",
        assertOwned: async () => undefined,
        close: async () => undefined,
      }),
      /requires a Jecode session lease/,
    );
    assert.deepEqual(await leaseOwner(file), {
      pid: 123,
      token: "123:observed",
      legacy: true,
    });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
