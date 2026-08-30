#!/usr/bin/env node

const major = Number.parseInt(process.versions.node, 10);
if (major < 24) {
  process.stderr.write(
    `jecode requires Node.js 24 or newer (current: ${process.versions.node})\n`,
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
