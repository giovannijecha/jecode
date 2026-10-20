# Attachments

Files and clipboard images appear as numbered draft elements, such as
`[1# Image]` or `[2# File: report.pdf]`. Each element points to a stored copy;
typing a label does not attach a file.

## Attaching

| Where | How |
| --- | --- |
| Fullscreen composer | Drop files onto the terminal, press Alt+V for a clipboard image, or type `/attach PATH...`. |
| `jecode --plain` | `/attach PATH...` stages files for the next message; Enter on an empty line sends only the attachments. |
| Single prompts and pipes | `jecode --attach PATH [--attach PATH]... "prompt"` attaches files to the first message. |

Any regular file can be attached, including binaries and files outside the
working directory, up to 1 GiB each. Drops and `/attach` accept at most 64
paths at once. Elements appear at the caret after copying; Backspace or Delete
removes a whole element. A message can contain only attachments. Wait for the
import to finish before sending; an early Enter keeps the draft and sends
nothing.

Ctrl+V remains the terminal's text paste. Jecode reads the clipboard only on
Alt+V and only for an image. A missing image or failed import shows a warning
and keeps the draft.

### File drops and paths

Terminals deliver a drop as pasted file paths. Jecode imports them only when
every path names an existing regular file; otherwise the paste remains text.
Quotes and backslash escapes are removed without variable expansion, globbing
or command execution. Quoted Windows and WSL paths and `file://` URIs work.
Under WSL, Windows paths are converted with `wslpath` using the live mount
table; paths through `\\wsl.localhost\<distro>\...` into the running
distribution also map to Linux paths.

`/attach` accepts quoted or bare paths, absolute or relative to the working
directory. Backslashes in Windows paths need no escaping. In plain chat it
also accepts saved references such as `/attach attachment:att-123-456-7`.
A resumed draft prints these references so the stored copies can be restaged
without the source files. Plain-chat staging survives exit, `/new`, `/resume`
and deletion of the source conversation. Use `/attach --clear` to discard it;
opening the session in the fullscreen composer shows the staged elements.

## What the model receives

Jecode keeps the original bytes and sends a manifest with each attachment's
name, `attachment:ID` reference, stored path and delivery status. It builds
provider content from the copy just before a request.

| Kind | Delivery |
| --- | --- |
| PNG, JPEG, WebP, GIF | Image part unchanged if at most 3.75 MB and 8000 pixels per dimension. |
| Other or oversized images | Converted to PNG or JPEG, at most 2048 pixels on the long side. Without a converter, the original stays local and the manifest says so. |
| PDF | File part via OpenRouter's `file-parser`: `native` for a model with file input, otherwise `cloudflare-ai`. PDFs over 32 MiB stay local. |
| Text | Read through `read attachment:ID` with normal pagination. |
| Other binaries | Kept locally; the manifest gives the path for a suitable tool. |

If the model's catalog entry lacks image input, it receives a note instead of
the image. OpenRouter PDF parse annotations are stored with the response and
reused on later requests, avoiding a repeat parse.

`read attachment:ID` can revisit copies in the session's project bucket.
Text returns pages; images and PDFs are shown to the model in the next
request; other binaries return a path and an uninterpreted-data note. The
result includes `attached_from`. References remain available after
compaction; see [LONG_WORK.md](LONG_WORK.md). File tools keep their normal
scope, with these references adding access to the stored copies.

## Storage and retention

The model can also use `read` on an image or PDF in the project or the current
`tmp:` area. Jecode recognizes the file's contents, imports a copy into this
same pool and returns its `attachment:ID` reference. It then sends the visual
content on the next request, just as for `read attachment:ID`. No composer
attachment is needed. The normal file-tool path boundaries still apply.

Copies live in the project's session bucket, under
`~/.jecode/sessions/<project>/attachments/`. Each asset has metadata and an
exact-byte copy; converted images can also have a provider view. Changes to
the original file do not change its copy. Drafts, plain-chat staging, queued
and paused messages, prompt history and saved messages retain their elements
across cancellation, session switches, restarts and crash recovery. Tool results
that reference imported images or PDFs also retain their copies. Cleaning the
temporary area removes its originals, not these saved copies. PDF annotation
images are stored here too.

An asset is removed only when no saved session in the bucket refers to it.
On TUI exit, collection gives unreferenced imports a 10-minute grace period
to protect other running instances. [Session deletion](SESSIONS.md#delete)
immediately releases attachments of its messages when no other session needs
them; inherited drafts retain their copies.

## Export

`/export` copies attachments referenced by the conversation, including PDF
annotation images, into a `.attachments` folder beside the JSON. If a copy
fails, the export is not written. See [EXPORT.md](EXPORT.md).

## Native requirements

| Platform | Clipboard image (Alt+V) | Image conversion |
| --- | --- | --- |
| Windows | Windows PowerShell 5.1 (`powershell.exe`) with System.Windows.Forms | System.Drawing through PowerShell |
| WSL | `powershell.exe` through Windows interop | Same, through interop |
| macOS | `osascript` | `sips` |
| Linux | `wl-paste` (Wayland) or `xclip` (X11) | None; unsupported formats stay local |

If a native command is unavailable, Jecode reports the import or conversion
failure. Image bytes use standard input and output, not command arguments.
