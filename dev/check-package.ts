// Release-package guard: inspect the tarball manifest without creating one.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const npm = process.env.npm_execpath;
if (npm === undefined) throw new Error("run this check through npm run check:package");
const output = execFileSync(process.execPath, [npm, "pack", "--dry-run", "--json"], {
  encoding: "utf8",
});
const [packed] = JSON.parse(output) as [{ size: number; files: { path: string }[] }];
if (packed === undefined) throw new Error("npm did not describe a package");

const paths = packed.files.map((file) => file.path.replaceAll("\\", "/"));
const allowed = paths.every((file) =>
  file === "LICENSE" ||
  file === "README.md" ||
  file === "package.json" ||
  file.startsWith("bin/") ||
  file.startsWith("src/")
);
if (!allowed) {
  throw new Error(`unexpected package files: ${paths.filter((file) =>
    !(file === "LICENSE" || file === "README.md" || file === "package.json" || file.startsWith("bin/") || file.startsWith("src/"))
  ).join(", ")}`);
}
if (!paths.includes("bin/jecode.js")) throw new Error("the jecode executable is missing from the package");
if (packed.size > 1_000_000) throw new Error(`package is unexpectedly large: ${packed.size} bytes`);

const manifest = JSON.parse(readFileSync("package.json", "utf8")) as {
  dependencies?: Record<string, unknown>;
};
if (Object.keys(manifest.dependencies ?? {}).length !== 0) {
  throw new Error("runtime dependencies must stay empty");
}

process.stdout.write(`package: ${paths.length} files, ${packed.size} bytes, zero runtime dependencies\n`);
