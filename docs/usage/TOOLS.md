# Tools

The model works in your project by calling tools. Jecode runs each call, checks
it against the applicable [permissions](PERMISSIONS.md) and returns
the result to the model. This page explains what the tools do and what you see
of them. Exact arguments and limits are in the
[Tool reference](TOOL_REFERENCE.md).

## How tool calls run

- A call runs only after the model's complete response has arrived; a partial or
  interrupted response never executes anything. Text that describes a change is
  not a change: only a tool call acts.
- An error, such as a missing file or a failed command, returns to the model as
  that call's result, so it can correct itself.
- Up to four adjacent read-only calls, such as reads, listings and searches, run
  at the same time. The model can also mark other adjacent calls as independent
  to run them in parallel, at most four at once. Results always enter the
  conversation in the original call order.
- The model can send several tools as one [`steps`](TOOL_REFERENCE.md#steps)
  call when later ones do not need earlier results, such as an edit followed by
  the tests. You still see one card and one permission check per step, and it
  saves the request that would otherwise resend the conversation between them.
- Jecode sets no limit on turn length, model requests or tool calls. A turn ends
  when the model answers without calling a tool, when you stop it, or on an
  error that needs you. See [Long-running work](LONG_WORK.md).

## Available tools

| Tool | What it does |
| --- | --- |
| `read` | Read a text file a page at a time, up to 12 files in one call, or show an image or PDF to the model. Also reads saved command output, earlier conversation history, skills, attachments and checkpoints. |
| `write` | Create or replace a whole text file. |
| `edit` | Apply exact text replacements to one file, or to up to 12 files in one call. Each file changes all or nothing; with several files, one that fails does not stop or undo the others. |
| `list` | List files and directories, optionally recursively and filtered by a pattern. |
| `search` | Find literal text in project files, with line numbers and context. |
| `map` | Summarize the project's structure and keep source-backed notes about it. |
| `bash` | Run commands in Jecode's own [command language](SHELL.md). |
| `exec` | Run a program directly with exact arguments, optionally in an interactive terminal. |
| `process` | Check, control or stop commands left running in the background. |
| `checkpoint` | List, inspect and restore files saved before a modifying call. |
| `web` | Fetch, download or crawl web pages over HTTP(S). |
| `browser` | Control an installed Chrome or Edge browser. |
| `mcp` | Discover and call remote tools from globally configured [MCP connections](MCP.md). |
| `lsp` | Query definitions, references and document diagnostics through enabled [language servers](LSP.md). |
| `agent` | Delegate work to [collaborators](COLLABORATION.md). Main agent only. |
| `ask_user` | Ask you up to four [questions](QUESTIONS_AND_PLANS.md) and wait for answers. Main agent only. |
| `todo_list` | Keep a [plan](QUESTIONS_AND_PLANS.md#plans) for the conversation. |
| `steps` | Run several of these tools in order with [one call](TOOL_REFERENCE.md#steps). |

Collaborators get every tool except `agent` and `ask_user`.

MCP connections have separate remote permissions. Enable and manage them in
Settings → **MCP** in the TUI, or in `~/.jecode/mcp.json`; `/mcp` inspects the
configured connections. Read, Workspace and Full do not govern MCP calls: an
enabled connection follows the permissions granted at its service, including
in Read. Collaborators use the same connections. Local checkpoints cannot undo
remote changes.

LSP likewise follows the user's server enablement at all local levels. Its
selected executable runs as the user, and a root is analysis context rather
than confinement. Each conversation owns and reuses its own server instances.

## Files

`read`, `write` and `edit` work inside the working folder; paths outside it are
refused in Read and need approval in Workspace. `write` creates a new file's
missing parent directories. Writes and edits replace the file through a temporary
copy, so a failed edit leaves the original intact. Text is UTF-8.

`edit` applies its replacements in order to the result of the previous ones.
Each one must match exactly once, including whitespace and line endings; a
missing or ambiguous match leaves that file unchanged, and in a multi-file call the
other files are still written. For files inside the project,
`write` and `edit` attempt to save the previous state in a
[checkpoint](CHECKPOINTS.md). The receipt reports what was captured; a save
failure does not block the file change.

When `read` opens an image or PDF in the project or the temporary area, Jecode
stores a copy and shows it to the model on the next request, as it does for
[attachments](ATTACHMENTS.md#what-the-model-receives).

## Finding things

`list` and `search` are Jecode's own implementation; they need no installed
search program. By default they skip hidden entries (names starting with `.`)
and anything ignored by the project's `.gitignore` files or the repository's
`info/exclude`. Your global Git excludes file is not read. Both can be asked to
include hidden or ignored entries. `search` matches literal text; for regular
expressions the model can run an installed program through `bash` or `exec`.

### Project map

`map` gives the model a quick overview of the project when it asks for one:
directory areas, likely entry points and, for Rust and Markdown files, a lexical
outline of declarations and headings. It is built on demand; Jecode never scans
your project at startup or puts a map into every request.

The model can also save notes about the project with `map`, each citing source
files. Notes are kept per project in the Jecode home and shared by all
conversations in that project. When a cited file changes, the next overview
marks the note `changed`; a missing file marks it `missing`. Notes are the
model's interpretation, not verified facts.

## Commands

`bash` runs commands in Jecode's own command language, the same on every
platform. It is not GNU Bash and never starts an installed shell; see
[Command language](SHELL.md) for what it supports. `exec` runs one program with
exact arguments and no shell parsing. Both start in the working folder and
receive `JECODE_TMP` and `TMPDIR` pointing to the conversation's
[temporary area](TEMPORARY.md), and `TEMP` and `TMP` too, except on Windows
when its path is 260 characters or longer. Provider credentials are removed from their
environment.

Commands have no timeout unless the model sets one. A command that ends within
its initial wait returns the exit code and a preview of stdout and stderr (16 KiB
per stream by default); the complete output is saved and the model can read all
of it.

Commands do not create checkpoints automatically, because Jecode cannot know
what a command will change. The model can list the files or directories a
command may modify in its `protect` argument, and Jecode saves them before the
command starts.

In Workspace, commands run inside an operating-system confinement; see
[Command confinement](PERMISSIONS.md#command-confinement).

### Processes

Every command is a process that you and the model can see and control while it
runs. If it is still running after its initial wait, 10 seconds unless the model
chooses another, the call returns a process ID and the command keeps running, so
the model can get on with independent work. Reaching the wait never stops a
command: only a timeout the model set, a stop, or the end of the conversation
does. The model uses `process` to read output, send input to an interactive
program, interrupt or stop it.

When a process ends, the model is told once at its next request, with the outcome
and references to the saved output, so it does not have to keep checking. An idle
conversation continues on its own when a process ends by itself. If you stopped
the turn, the outcome waits for your next message, and a process you stopped
yourself never starts a turn. In the plain chat the conversation continues at the
next line you enter, an empty one included, because reading a line blocks. A single
prompt (`jecode "..."` or piped input) ends with its answer, and that stops its
processes.

The model decides what depends on what. Builds, tests, generation and exports
need their outcome before dependent work or a claim of success; servers and
watchers need a real readiness check, and a live process or empty output does not
establish one. Processes last until they finish, are stopped, or the conversation
is closed, switched or deleted, and are never restarted.

To see, stop or background processes yourself, use `/process`: it lists the
conversation's running and recent processes and shows their output without adding
anything to the conversation. See
[Terminal interface](../tui/INTERFACE.md#processes).

Saved completion rows remain available after reopening a conversation, including
in plain chat. The model can read `history:processes` to recover a recorded outcome
when its original process ID is no longer live. This never restarts the command.

## Web and browser

`web` makes HTTP(S) requests through Jecode's native HTTP support. `fetch`
returns the status, a text preview and links; `download` saves a response;
`crawl` follows links from starting pages, 20 pages per call by default and
staying on the same site unless asked otherwise. Responses are saved in the
temporary area. Jecode provides no search engine; the model can browse to a
search site with `browser`.

`browser` drives an installed browser through its local DevTools interface: it
can open pages, read their text and interactive elements, click, type, wait,
scroll, take screenshots, run JavaScript and read console and network logs.
Screenshots are shown to the model automatically. The browser starts headless
by default; the model can open a visible window, for example for a sign-in you
complete yourself.

For keyboard behavior, `type` sends a sequence of key events, including Enter
for newlines and Tab for tabs. `press` supports individual keys and shortcuts
such as `Ctrl+z`, `Meta+z` and `Shift+Tab`, with an optional modifier bitmask.
`fill` replaces an input value without exercising per-character key handlers.

- **Which browser.** Jecode looks for Google Chrome and Microsoft Edge in their
  standard locations and on `PATH` (on Linux: `google-chrome`, `chromium`,
  `chromium-browser` or `microsoft-edge` on `PATH`). Set `JECODE_BROWSER` to the
  executable's path to use another one. Only Chrome and Edge have been tested.
- **Profiles.** Without a profile name, each browser uses a temporary profile
  that is deleted with the conversation's temporary files. A named profile is
  kept under `~/.jecode/browser/profiles/` and retains cookies and signed-in
  sessions between conversations.
- **Permission.** Browser control requires Full.

Pages, screenshots and logs the agent reads become part of the conversation and
are sent to your model provider.

## What you see

In the TUI, each call appears as a compact card: a badge with the tool name,
its target and a one-line result. Related routine calls group into a Work
chapter, file changes show a diff, and Ctrl+O expands everything from the latest
turn. See [Tool activity](../tui/TOOL_ACTIVITY.md).
