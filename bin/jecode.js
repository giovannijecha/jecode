#!/usr/bin/env node

const [major = 0, minor = 0] = process.versions.node.split(".").map(Number);
const supported = major >= 24 || (major === 22 && minor >= 18);
if (!supported) {
  process.stderr.write(
    `jecode requires Node.js 22.18+ (22.x) or Node.js 24+ (current: ${process.versions.node})\n`,
  );
  process.exitCode = 1;
} else {
  const runtime = new URL("../dist/main.js", import.meta.url);
  try {
    await import(runtime.href);
  } catch (error) {
    if (error?.code !== "ERR_MODULE_NOT_FOUND" || error.url !== runtime.href) throw error;
    process.stderr.write(
      "jecode's compiled runtime is missing; install @giovannijecha/jecode from npm " +
      "or run npm run build:release in a source checkout\n",
    );
    process.exitCode = 1;
  }
}
