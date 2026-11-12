# Attachments

You can attach files and clipboard images to a message. Jecode copies each one
into its own storage, so later changes to the original do not affect the
conversation.

To point the agent at a file in the working directory without copying it, type
`@` in the TUI, or write `@path` yourself. A
[file reference](INTERFACE.md#reference-skills-and-files) is text in the message:
it adds nothing to attachment storage, and the model reads the file's current
content through its file tools when it needs it.

## Attach files

| Where | How |
| --- | --- |
| TUI | Drop files onto the terminal window, or paste their paths. Press Alt+V, or Ctrl+V on Linux and macOS, to attach an image from the clipboard. |
| `jecode --plain` | Start with `jecode --plain --attach PATH`. Enter on an empty line sends only the staged attachments. |
| Single prompt or pipe | `jecode --attach PATH [--attach PATH]... "prompt"` |

Attachments appear in the draft as numbered elements, such as `[1# Image]` or
`[2# File: report.pdf]`, shown as chips. Backspace or Delete removes a whole
element; typing a label yourself attaches nothing. A message can contain only
attachments.

Wait for an import to finish before sending; pressing Enter early keeps the
draft and sends nothing. Any regular file up to 1 GiB can be attached, including
binaries and files outside the project. A single drop accepts up to 64 paths.

Your terminal's own paste, such as Cmd+V, Ctrl+Shift+V or Ctrl+V on Windows, stays
text paste. Jecode reads the clipboard only on these shortcuts and only for an
image; if there is none, it shows a warning and keeps the draft.

### How drops are recognized

Terminals deliver a dropped file as pasted text. Jecode treats a paste as files
only when every path in it names an existing regular file; otherwise the paste
stays text. Quotes and backslash escapes are removed without variable
expansion, globbing or command execution. Quoted Windows paths, WSL paths and
`file://` URIs work. Under WSL, Windows paths are translated through the live
mount table, and `\\wsl.localhost\<distro>\...` paths into the running
distribution map to Linux paths.

## What the model receives

Each request carries a manifest listing every attachment's name, its
`attachment:ID` reference, its stored path and how it was delivered. The
content is built from the stored copy just before the request:

| Kind | Delivery |
| --- | --- |
| PNG, JPEG, WebP, GIF | The image. A PNG or JPEG up to 16 MiB that is larger than 512 KiB or 2,048 pixels on its long side is sent as a smaller copy (scaled to at most 2,048 pixels, PNG or JPEG at quality 85) when that copy is smaller. The stored original never changes. |
| Other image formats | Converted to PNG or JPEG, at most 2,048 pixels on the long side. If conversion is unavailable, the original stays local and the manifest says so. |
| PDF | Sent as a document up to 32 MiB. Through OpenRouter, models with native file input read it directly; others use OpenRouter's parser. Larger PDFs stay local. |
| Text | The model reads it with `read attachment:ID`, a page at a time. |
| Other files | Kept locally; the manifest gives the path so the model can use a suitable tool. |

When an OpenRouter model's catalog says it cannot take images, it receives a
note instead. The OpenAI and Anthropic routes, with a plan or an API key, send
supported images without that check, and the provider may reject them. OpenRouter's PDF parsing results are
saved and reused, so a PDF is not parsed twice.

Images and PDFs you attach stay in the conversation's active context. Images a
tool returns, such as browser screenshots, stay in every later request on the
Anthropic routes, where signed thinking depends on them, while their tool
results remain in the active context. On
the OpenAI and OpenRouter routes they are sent with the next request only;
afterwards the model can read the `attachment:ID` again to see them, or read
several together to compare. References stay valid after compaction, resume and
temporary cleanup.

Collaborators share the project's attachments: an `attachment:ID` passed in an
assignment works in their workspace, and files they import are readable by the
main agent.

## Storage

Copies live in the project's shared `attachments/` directory; see
[Files Jecode keeps](../reference/FILES.md#stored-files). Drafts, staged and queued messages,
prompt history and saved messages keep their attachments across cancellation,
switching conversations, restarts and crash recovery.

An attachment is removed only when no saved conversation, collaborator history
or recovery record in the project refers to it. When you quit the TUI, unused
imports are kept for 10 minutes in case another Jecode instance is using them.
[Deleting a conversation](SESSIONS.md#delete-a-conversation) releases its
attachments immediately if nothing else needs them.

[`/export`](COPY_AND_EXPORT.md#export) keeps attachment references but does not
copy the files.

## Platform requirements

| Platform | Clipboard image | Image conversion |
| --- | --- | --- |
| Windows | Windows PowerShell 5.1 (`powershell.exe`) with System.Windows.Forms | System.Drawing through PowerShell |
| WSL | `powershell.exe` through Windows interop | The same, through interop |
| macOS | `osascript` | `sips` |
| Linux | `libX11` (X11) or the compositor's Wayland data-control protocol | The desktop GdkPixbuf library |

On Wayland, the compositor must offer `ext-data-control-v1` or
`wlr-data-control-unstable-v1`. If it does not, Jecode uses X11 through XWayland
when `DISPLAY` is set, and otherwise reports that clipboard access is
unavailable. Installing a clipboard utility does not change what the compositor
allows.
