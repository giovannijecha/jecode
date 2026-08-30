#!/usr/bin/env node

const major = Number.parseInt(process.versions.node, 10);
if (major < 24) {
  process.stderr.write(
    `jecode requires Node.js 24 or newer (current: ${process.versions.node})\n`,
  );
  process.exitCode = 1;
} else {
  await import("../dist/main.js");
}
