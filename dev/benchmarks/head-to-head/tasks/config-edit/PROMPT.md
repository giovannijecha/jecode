Implement the requested JSON configuration migration in this repository. Preserve
the existing API and smoke tests. Use Node standard-library modules and no
dependencies. Keep the changes limited to this task; do not add unrelated
features, publish, or commit. Implement, verify with meaningful regression tests,
and briefly report the result and any remaining limitation.

The existing `src/index.js` exports must remain available:

- `parsePointer(pointer)` returns decoded RFC 6901 path segments. An empty pointer
  means the root; otherwise require a leading `/`. Decode only `~0` and `~1`;
  reject malformed escapes, nonstrings, and any decoded segment equal to
  `__proto__`, `prototype`, or `constructor`.
- `patchDocument(document, operations)` applies ordered `set`, `remove`, and
  `test` operations and returns a detached JSON document. Never mutate either
  input, including on failure. Require JSON-compatible values (finite numbers,
  strings, booleans, null, arrays without holes and plain objects); reject cycles,
  unsupported values and unsafe object keys. Each operation has `op`, `path`,
  and a `value` for set/test. Reject unknown operation fields and missing values.
  Every parent must already exist. Object set can create/replace its own leaf;
  remove/test require an existing own leaf. Array segments must be canonical
  nonnegative integers (no leading zero except `0`). Set replaces an existing
  index, appends at length, or appends with `-`; never create holes. Remove uses
  splice. `test` compares JSON values structurally, ignoring object key order,
  and throws on mismatch. Root set/test work; root remove is rejected.
- `migrateFiles(root, changes, {dryRun=false, signal, onCommit}={})` accepts unique
  relative JSON file paths mapped to operation arrays. Require an existing
  directory root and existing regular files; reject absolute paths, `..` path
  segments, symlinks in any component below root, duplicates after path
  normalization, malformed JSON, and invalid operations before changing any
  target. Empty changes return an empty report. Preserve an optional UTF-8 BOM
  and each file's POSIX permission bits. Changed files use two-space JSON plus
  one newline (and their original BOM); logically unchanged files remain byte
  identical and are not committed. Return `{changed: string[]}` in input order,
  containing only logically changed normalized relative paths, including during
  dryRun. Do not modify files during dryRun.
  Stage writes beside the targets and atomically replace each target. Before
  committing revalidate that every target still has its original bytes and is
  a regular nonsymlink file. Invoke `onCommit(relativePath,index)` after each
  committed change (index counts changed files starting at zero), awaiting it.
  If that callback throws, a write fails, or cancellation occurs before the
  final successful settlement, restore already committed targets byte-for-byte
  with their original permission bits and clean up owned staging files. Honor
  already-aborted signals before writes and signals aborted by onCommit.
  This is in-process rollback, not crash-atomicity across multiple files.
- `node src/cli.js ROOT MANIFEST [--dry-run]` reads a JSON array of
  `{path,operations}` entries from MANIFEST, invokes migrateFiles, prints its
  JSON report followed by a newline, and exits zero. Invalid usage, JSON or
  migration failure exits nonzero with an error on stderr. No stack trace is
  required. The manifest may be outside ROOT. Do not alter it.

Update the README to document the behavior, usage and rollback limitation.
