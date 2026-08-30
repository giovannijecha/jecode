// Refuse to publish when the immutable Git tag and package version diverge.

import { appendFile, readFile } from "node:fs/promises";

type Manifest = { name?: unknown; version?: unknown };

const [tag, ...extra] = process.argv.slice(2);
if (tag === undefined || extra.length > 0) {
  throw new Error("usage: npm run check:release-tag -- v<package-version>");
}

const manifest = JSON.parse(await readFile("package.json", "utf8")) as Manifest;
if (manifest.name !== "@giovannijecha/jecode") {
  throw new Error("release tags require the canonical npm package name");
}
if (typeof manifest.version !== "string" || manifest.version === "") {
  throw new Error("package version is missing");
}

const expected = `v${manifest.version}`;
if (tag !== expected) {
  throw new Error(`release tag ${JSON.stringify(tag)} does not match ${JSON.stringify(expected)}`);
}

const channel = manifest.version.split("+", 1)[0]?.includes("-") === true ? "next" : "latest";
if (process.env.GITHUB_OUTPUT !== undefined) {
  await appendFile(process.env.GITHUB_OUTPUT, `channel=${channel}\n`, "utf8");
}
process.stdout.write(`release tag: ${tag} -> npm ${channel}\n`);
