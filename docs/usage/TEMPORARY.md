# Session temporary files

Jecode provides an owned working area for disposable scripts, experiments, dumps,
one-off verification files and browser profiles:

```text
~/.jecode/tmp/FOLDER_BUCKET/SESSION_ID/
```

`JECODE_HOME` moves it together with configuration and saved sessions. The folder
bucket uses the existing canonical project identity. An ownership record checks
both project and session before files are used or cleared. The area is created
on first use; its metadata is excluded from file and size counts.

## Choose where verification files belong

Keep source code, durable tests and reusable project verification in the project.
Follow its existing test conventions and use checks proportional to the change.
Put one-off verification scripts, tools or dependencies installed solely for a
check, their configuration, and check-only fixtures, logs, reports, screenshots
and browser traces in the active session area. Use `tmp:` with file tools or
quoted `$JECODE_TMP` paths in Bash. Add lasting test infrastructure only when
future project use or an explicit user request justifies it. Honor explicit
user or project output requirements.

Set generated-output paths explicitly: `TMPDIR`, `TEMP` and `TMP` only affect
programs that use them and do not redirect arbitrary output. Before finishing,
inspect files you created, move your disposable leftovers into the session area
and preserve preexisting user files and necessary project deliverables. This is
model guidance; Jecode does not classify or move files automatically.

## Use from file and shell tools

`read`, `write` and `edit` accept `tmp:relative/path` in the active session area:

```text
write path: tmp:probes/check.sh
read  path: tmp:probes/check.sh
edit  path: tmp:probes/check.sh
```

`write` creates parent directories for temporary paths. Project writes retain
their existing parent-directory requirement. Absolute paths inside the same
temporary area are accepted too, including Git Bash `/c/...` paths on Windows.
Other sessions and configuration directories are not extra file-tool roots.
Traversal, ownership metadata paths, links and Windows junctions are rejected
within temporary paths. The file tools continue to read/write UTF-8 text;
Bash can create other file types in the area.

Bash starts in the project and receives `JECODE_TMP`, `TMPDIR`, `TEMP` and `TMP`
pointing to this session area. Use quoted paths:

```bash
bash "$JECODE_TMP/probes/check.sh"
some-command > "$JECODE_TMP/report.txt"
created=$(mktemp "$TMPDIR/check.XXXXXXXXXX")
```

Pass an explicit template to `mktemp` to use the session area on every platform.
macOS can use its native user temporary directory instead of `TMPDIR` when no
template is supplied.

Git Bash can normalize temporary environment paths into its `/c/...` notation;
native Windows child programs receive paths usable by Windows APIs. Jecode does
not change the parent terminal's environment. Temporary scripts must reference
project paths explicitly when their imports resolve relative to the script.
Programs that generate files beside their inputs need explicit output paths.
Bash keeps normal system access; providing an area does not redirect arbitrary
writes or make it a sandbox.

The tool contracts and each model request describe the active area. This remains
available after context compaction and when older saved conversations resume;
the original provider history is not rewritten to update these instructions.

## Retention and explicit cleanup

Completed turns, interruption and application exit keep working files. Explicit
resume reuses the same area. `/new` selects a new area and preserves the previous
one for its saved session. There is no age limit or automatic eviction.

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
turn returns queued commands to the draft instead of executing them.
Before deleting anything, Jecode checks the entire area for links, junctions and
special files and saves a cleanup-intent event. Cleanup success or failure is
also recorded. An interrupted cleanup remains identifiable after resume and is
never retried automatically. A filesystem error during deletion can leave a
partially cleared area; errors remain visible.

The next model request includes the latest cleanup status alongside the current
area instructions, even after compaction. Temporary maintenance stays separate
from original provider messages and is included in session/export events.

The TUI shows requested `/tmp` information without a command echo. Successful
cleanup acknowledgements remain hidden, like other local command confirmations.
Plain chat prints both information and cleanup results.

## Verification

Executed Windows checks cover all four tools, Git Bash `mktemp`, native Windows
child temporaries, paths containing spaces, folder/session boundaries, explicit
resume and interruption, cleanup/save failures, retained complete output,
junction rejection, legacy context, queued TUI commands and plain chat. Linux
checks under WSL with native tmpfs fixtures exercise the shared tools, Bash temporary files, folder/session
boundaries, interruption, resume and cleanup. Native Windows child paths,
junctions and queued TUI commands have Windows-specific coverage. GitHub Actions
runs the shared test suite on native Windows, Linux and macOS; ignored clipboard
and interactive terminal checks still require a suitable native host.
