// Release-package guard: inspect the tarball manifest without creating one.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { posix } from "node:path";
import { assertReleaseDocumentation } from "./release-policy.ts";

const npm = process.env.npm_execpath;
if (npm === undefined) throw new Error("run this check through npm run check:package");
const output = execFileSync(process.execPath, [npm, "pack", "--dry-run", "--ignore-scripts", "--json"], {
  encoding: "utf8",
});
const [packed] = JSON.parse(output) as [{ size: number; files: { path: string }[] }];
if (packed === undefined) throw new Error("npm did not describe a package");

const paths = packed.files.map((file) => file.path.replaceAll("\\", "/"));
const allowedPackageFile = (file: string): boolean =>
  file === "LICENSE" ||
  file === "README.md" ||
  file === "package.json" ||
  file === "assets/jeco-256.png" ||
  file === "assets/wordmark-steel.svg" ||
  file.startsWith("bin/") ||
  file.startsWith("dist/");
if (!paths.every(allowedPackageFile)) {
  throw new Error(
    `unexpected package files: ${paths.filter((file) => !allowedPackageFile(file)).join(", ")}`,
  );
}
if (!paths.includes("bin/jecode.js")) throw new Error("the jecode executable is missing from the package");
if (!paths.includes("dist/main.js")) throw new Error("the compiled entry point is missing from the package");
if (!paths.includes("assets/jeco-256.png")) throw new Error("the OAuth callback mascot is missing from the package");
if (!paths.includes("assets/wordmark-steel.svg")) throw new Error("the README wordmark is missing from the package");
if (paths.some((file) => file.endsWith(".ts"))) throw new Error("release packages must not contain TypeScript runtime files");
if (packed.size > 1_000_000) throw new Error(`package is unexpectedly large: ${packed.size} bytes`);

const manifest = JSON.parse(readFileSync("package.json", "utf8")) as {
  bin?: Record<string, unknown>;
  dependencies?: Record<string, unknown>;
  name?: string;
  private?: boolean;
  publishConfig?: { access?: string; registry?: string };
  scripts?: Record<string, unknown>;
  version?: string;
};
if (manifest.name !== "@giovannijecha/jecode") {
  throw new Error("release packages must use the canonical npm scope");
}
if (typeof manifest.version !== "string" || manifest.version === "") {
  throw new Error("release packages must declare a version");
}
if (manifest.bin?.jecode !== "bin/jecode.js") {
  throw new Error("the jecode executable path must use npm's canonical bin form");
}
if (Object.keys(manifest.dependencies ?? {}).length !== 0) {
  throw new Error("runtime dependencies must stay empty");
}
if (manifest.private === true) throw new Error("release packages must be publishable");
if (manifest.publishConfig?.access !== "public") {
  throw new Error("release packages must publish with public access");
}
if (manifest.publishConfig?.registry !== "https://registry.npmjs.org/") {
  throw new Error("release packages must target the public npm registry");
}
const readme = readFileSync("README.md", "utf8");
assertReleaseDocumentation(manifest.version, readme);
assertPackagedReferences(readme, paths);
if (manifest.scripts?.["build:release"] !== "node dev/build-release.ts") {
  throw new Error("release packages must keep one explicit clean build command");
}
if (manifest.scripts?.["pack:release"] !== "npm run build:release && npm pack --ignore-scripts") {
  throw new Error("release tarballs must build explicitly and disable lifecycle scripts while packing");
}
const implicitLifecycleScripts = [
  "build",
  "dependencies",
  "preinstall",
  "install",
  "postinstall",
  "prepublish",
  "prepublishOnly",
  "preprepare",
  "prepare",
  "postprepare",
  "prepack",
  "postpack",
  "publish",
  "postpublish",
]
  .filter((name) => manifest.scripts?.[name] !== undefined);
if (implicitLifecycleScripts.length > 0) {
  throw new Error(
    `release packages must not define implicit lifecycle scripts: ${implicitLifecycleScripts.join(", ")}`,
  );
}

process.stdout.write(`package: ${paths.length} files, ${packed.size} bytes, zero runtime dependencies\n`);

function assertPackagedReferences(markdown: string, packagePaths: readonly string[]): void {
  const exact = new Set(packagePaths);
  const folded = new Map(packagePaths.map((file) => [file.toLocaleLowerCase("en-US"), file]));
  for (const reference of localReferences(markdown)) {
    const target = packageTarget(reference);
    if (exact.has(target)) continue;
    const actual = folded.get(target.toLocaleLowerCase("en-US"));
    if (actual !== undefined) {
      throw new Error(`README package reference has incorrect casing: ${target} (expected ${actual})`);
    }
    throw new Error(`README package reference is missing from the package: ${target}`);
  }
}

function localReferences(markdown: string): string[] {
  const markdownLinks = [...markdown.matchAll(/!?\[[^\]]*\]\(\s*<?([^\s)>]+)>?(?:\s+[^)]*)?\)/gu)]
    .map((match) => match[1] as string);
  const htmlLinks = [...markdown.matchAll(/\b(?:href|src)=(['"])(.*?)\1/gu)]
    .map((match) => match[2] as string);
  return [...markdownLinks, ...htmlLinks].filter((reference) =>
    reference !== "" && !reference.startsWith("#") && !reference.startsWith("//") &&
    !/^[a-z][a-z0-9+.-]*:/iu.test(reference)
  );
}

function packageTarget(reference: string): string {
  const encoded = reference.split(/[?#]/u, 1)[0] as string;
  let decoded: string;
  try {
    decoded = decodeURIComponent(encoded);
  } catch {
    throw new Error(`README package reference is not valid URL text: ${reference}`);
  }
  const target = posix.normalize(decoded.replaceAll("\\", "/")).replace(/^\.\//u, "");
  if (
    target === "." || target === ".." || target.startsWith("../") ||
    posix.isAbsolute(target)
  ) throw new Error(`README package reference leaves the package: ${reference}`);
  return target;
}
