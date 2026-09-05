// Prove the packed artifact works after a real isolated global installation.

import { spawnSync } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";

type Packed = { filename: string };
type Manifest = { name: string; version: string };

const npm = process.env.npm_execpath ?? fail("run this check through npm run check:install");
const manifest = JSON.parse(await readFile("package.json", "utf8")) as Manifest;
const packagePath = manifest.name.split("/");

const root = await mkdtemp(path.join(tmpdir(), "jecode-install-check-"));
const packRoot = path.join(root, "pack");
const prefix = path.join(root, "prefix");

try {
  await mkdir(packRoot);
  const packed = runNpm(["pack", "--ignore-scripts", "--json", "--pack-destination", packRoot]);
  const [artifact] = JSON.parse(packed.stdout) as Packed[];
  if (artifact === undefined) throw new Error("npm did not create a package");

  const tarball = path.join(packRoot, artifact.filename);
  await access(tarball);
  runNpm(["install", "--global", "--ignore-scripts", "--prefix", prefix, tarball]);

  const launcher = process.platform === "win32"
    ? path.join(prefix, "jecode.cmd")
    : path.join(prefix, "bin", "jecode");
  await access(launcher);

  const entry = process.platform === "win32"
    ? path.join(prefix, "node_modules", ...packagePath, "bin", "jecode.js")
    : path.join(prefix, "lib", "node_modules", ...packagePath, "bin", "jecode.js");
  await access(entry);
  const command = process.platform === "win32"
    ? { file: process.execPath, args: [entry, "--version"] }
    : { file: launcher, args: ["--version"] };
  const result = spawnSync(command.file, command.args, {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) {
    throw new Error(`installed jecode failed (${result.status})\n${result.stderr}`);
  }

  if (result.stdout.trim() !== manifest.version) {
    throw new Error(`installed jecode reported ${JSON.stringify(result.stdout.trim())}, expected ${manifest.version}`);
  }
  process.stdout.write(`installed cli: jecode ${manifest.version}\n`);
} finally {
  await rm(root, { recursive: true, force: true });
}

function runNpm(args: string[]): { stdout: string; stderr: string } {
  const result = spawnSync(process.execPath, [npm, ...args], {
    cwd: process.cwd(),
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) {
    throw new Error(`npm ${args[0]} failed (${result.status})\n${result.stdout}\n${result.stderr}`);
  }
  return { stdout: result.stdout, stderr: result.stderr };
}

function fail(message: string): never {
  throw new Error(message);
}
