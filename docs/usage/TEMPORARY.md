# Session temporary files

Jecode provides an owned working area for disposable scripts, experiments, dumps,
one-off verification files and isolated browser profiles:

```text
~/.jecode/tmp/FOLDER_BUCKET/SESSION_ID/
```

`JECODE_HOME` moves it together with configuration and saved sessions. The folder
bucket uses the existing canonical project identity. An ownership record checks
both project and session before files are used or cleared. The area is created
on first use; its metadata is excluded from file and size counts.

## Put disposable files here

Keep source code and reusable tests in the project. Put one-off verification
scripts, fixtures, logs, reports, screenshots and browser traces in the active
session area. Use `tmp:` with file tools or quoted `$JECODE_TMP` paths in Bash.
Honor explicit user or project output requirements.
The `web` and `browser` tools also save fetched responses, snapshots, screenshots
and logs here. A named persistent browser profile lives separately under
`~/.jecode/browser/profiles/` and survives `/tmp clean` and session deletion.
Browser screenshots are also copied into session attachment storage and shown
to the model on the next request. Their returned `attachment:ID` references
survive temporary cleanup.

Set output paths explicitly: `TMPDIR`, `TEMP` and `TMP` affect only programs
that use them. Jecode does not move or classify files automatically.

## Use from file and shell tools

`read`, `write` and `edit` accept `tmp:relative/path` in the active session area.
For example, a `write` call can use these arguments:

```json
{"path": "tmp:probes/check.sh", "content": "printf 'ok\\n'\n"}
```

`write` creates parent directories for temporary paths. Project writes retain
their existing parent-directory requirement. Absolute paths inside the same
temporary area are accepted too, including Git Bash `/c/...` paths on Windows.
Other sessions and configuration directories are not extra file-tool roots.
Traversal, ownership metadata paths, links and Windows junctions are rejected
within temporary paths. `write` and `edit` use UTF-8 text. `read` also recognizes
images and PDFs and shows them to the model through a saved attachment copy;
its returned `attachment:ID` remains readable after temporary cleanup. Bash
can create other file types in the area. See [Attachments](ATTACHMENTS.md).

Shell commands run in the project with `JECODE_TMP`, `TMPDIR`, `TEMP` and `TMP`
pointing to this session area. Use quoted paths:

```bash
bash "$JECODE_TMP/probes/check.sh"
printf 'probe\n' > "$JECODE_TMP/report.txt"
created=$(mktemp "$TMPDIR/check.XXXXXXXXXX")
```

Pass an explicit template to `mktemp` to use the session area on every platform.
macOS can use its native user temporary directory instead of `TMPDIR` when no
template is supplied.

On Windows, Git Bash initializes its shared `/tmp` mount from the inherited
environment before Jecode exports the session's `TEMP` and `TMP`. Deleting a
session therefore cannot leave another live shell's `/tmp` pointing to that
removed session area.

Git Bash can normalize temporary environment paths into its `/c/...` notation;
native Windows child programs receive paths usable by Windows APIs. Jecode does
not change the parent terminal's environment. Temporary scripts must reference
project paths explicitly when their imports resolve relative to the script.
Programs that generate files beside their inputs need explicit output paths.
Bash keeps normal system access; providing an area does not redirect arbitrary
writes or make it a sandbox.

The tool contracts and each model request describe the active area, including
after context compaction and resume.

## Retention and explicit cleanup

Completed turns, interruption and application exit keep working files. Explicit
resume reuses the same area. `/new` selects a new area and preserves the previous
one for its saved session. There is no age limit or automatic eviction.

Jecode stops browsers using the active session before cleaning its temporary area.
Read-only inventory skips links and special files without following them and
reports their count; Chromium can create such entries in a live profile.
`/tmp` shows the current path, file/directory counts, logical file bytes and
retention policy. `/tmp clean` clears only that session's working files, keeping
its ownership record so the area can be reused. It does not clear project files,
saved conversations or complete tool-output streams. Older areas can be cleared
by explicitly resuming their sessions and running the same command.

Deleting a conversation from `/resume` removes its entire owned working
area and owned tool outputs. It uses the same marker/type checks and rejects
sessions open in another process. See [SESSIONS.md](SESSIONS.md#delete).

Cleanup is available only when the session is ready. In the TUI, a command
submitted during generation joins the existing FIFO queue. A stopped/failed
turn leaves unprocessed commands as paused drafts for review.
Before deleting, Jecode rejects links, junctions and special files and saves
a cleanup-intent event. Cleanup success or failure is also recorded. An
interrupted cleanup is never retried automatically. If cleanup fails or Jecode
stops during it, run `/tmp` and inspect the area before relying on old `tmp:`
references or retrying `/tmp clean`; some files may already be gone.

The next model request includes the latest cleanup status, even after
compaction. Cleanup events also appear in exports.
