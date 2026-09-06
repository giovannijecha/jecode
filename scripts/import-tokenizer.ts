// Explicit development import. The installed runtime never downloads vocabularies.
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { gzipSync } from "node:zlib";

const url = "https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken";
const digest = "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d";
const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
if (!response.ok) throw new Error(`vocabulary download failed (${response.status})`);
if (response.body === null) throw new Error("missing vocabulary response");
const chunks: Uint8Array[] = [];
let size = 0;
for await (const chunk of response.body) {
  size += chunk.length;
  if (size > 4_000_000) throw new Error("vocabulary download exceeds its size limit");
  chunks.push(chunk);
}
const bytes = Buffer.concat(chunks);
if (createHash("sha256").update(bytes).digest("hex") !== digest) {
  throw new Error("vocabulary checksum mismatch");
}
const directory = new URL("../assets/tokenizers/", import.meta.url);
await mkdir(directory, { recursive: true });
await writeFile(new URL("o200k-base.tiktoken.gz", directory), gzipSync(bytes, { level: 9 }));
process.stdout.write(`verified o200k_base: ${bytes.length} source bytes\n`);
