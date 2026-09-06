// Pinned data, loaded once on demand. No network, executable data, or user cache.
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { promisify } from "node:util";
import { gunzip } from "node:zlib";

let loading: Promise<ReadonlyMap<string, number>> | undefined;
const unpack = promisify(gunzip);
const digest = "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d";

export function vocabulary(): Promise<ReadonlyMap<string, number>> {
  loading ??= load();
  return loading;
}

async function load(): Promise<ReadonlyMap<string, number>> {
  const compressed = await readFile(new URL("../../../assets/tokenizers/o200k-base.tiktoken.gz", import.meta.url));
  if (compressed.length > 2_000_000) throw new Error("token vocabulary exceeds its size limit");
  const data = await unpack(compressed, { maxOutputLength: 4_000_000 });
  if (createHash("sha256").update(data).digest("hex") !== digest) {
    throw new Error("token vocabulary checksum mismatch");
  }
  const ranks = new Map<string, number>();
  const lines = data.toString("ascii").trimEnd().split("\n");
  for (let index = 0; index < lines.length; index++) {
    const [encoded, rank] = (lines[index] as string).split(" ");
    if (encoded === undefined || Number(rank) !== index) throw new Error("invalid token vocabulary");
    ranks.set(Buffer.from(encoded, "base64").toString("latin1"), index);
    if (index % 2_048 === 0) await new Promise<void>((resolve) => setImmediate(resolve));
  }
  if (ranks.size !== 199_998) throw new Error("incomplete token vocabulary");
  return ranks;
}
