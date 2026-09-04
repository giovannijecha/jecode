import { test } from "node:test";
import assert from "node:assert/strict";
import { fileIdentity, sameFileIdentity } from "../src/file-identity.ts";

test("file identity preserves Windows identifiers above the safe integer boundary", () => {
  const left = fileIdentity({
    dev: 0n,
    ino: 134_826_513_844_467_146n,
    birthtimeNs: 1n,
  });
  const right = fileIdentity({
    dev: 0n,
    ino: 134_826_513_844_467_156n,
    birthtimeNs: 1n,
  });

  assert.equal(Number(left.ino), Number(right.ino));
  assert.equal(sameFileIdentity(left, right), false);
});

test("file identity rejects inode reuse with a different birth time", () => {
  const left = fileIdentity({ dev: 1n, ino: 2n, birthtimeNs: 3n });
  const right = fileIdentity({ dev: 1n, ino: 2n, birthtimeNs: 4n });

  assert.equal(sameFileIdentity(left, right), false);
});
