# Temporary files

Each conversation has its own working area for disposable files: one-off
scripts, probes, logs, reports, screenshots and browser data. Using it keeps
throwaway files out of your project.

```text
~/.jecode/tmp/<project>/<session>/
```

The area is created on first use and moves with `JECODE_HOME`.

## How the agent uses it

- File tools accept `tmp:relative/path`, for example
  `{"path": "tmp:probes/check.sh", "content": "printf 'ok\\n'\n"}`. `write`
  creates missing parent directories inside the area.
- Commands start in the project directory with `JECODE_TMP`, `TMPDIR`, `TEMP`
  and `TMP` pointing to the area, so a command can write
  `"$JECODE_TMP/report.txt"`. Programs that ignore those variables still need
  explicit output paths.
- The `web` and `browser` tools save responses, crawl data, snapshots,
  screenshots, logs and unnamed browser profiles here.

The model is told the area's location in every request, including after
compaction and resume. Links, junctions and paths that escape the area are
rejected. Images and PDFs read from the area are copied into
[attachment storage](ATTACHMENTS.md), so their `attachment:ID` survives cleanup.

## Inspect and clean up

| Command | Effect |
| --- | --- |
| `/tmp` | Show the area's path, file and directory counts and total size. |
| `/tmp clean` | Delete this conversation's temporary files. |

Temporary files are never deleted automatically: they survive finished turns,
interruptions and quitting, and `/new` simply starts a new area. To clean an
older conversation's area, resume it and run `/tmp clean`. Deleting a
conversation from `/resume` removes its area entirely.

`/tmp clean` first stops browsers the conversation is using. It never touches
project files, saved conversations, complete command output or named browser
profiles under `~/.jecode/browser/profiles/`. It runs only when the conversation
is idle; during a turn it waits as a pending message.

Cleanup rejects links and special files, and records its intent and result in
the conversation. If it fails or Jecode stops during it, some files may already
be gone: run `/tmp` and check before retrying or relying on old `tmp:` paths.
