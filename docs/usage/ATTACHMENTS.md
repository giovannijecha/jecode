# Attachments

Files and clipboard images appear as numbered draft elements, such as
`[1# Image]` or `[2# File: report.pdf]`. Each element points to a stored copy;
typing a label does not attach a file.

## Attaching

| Where | How |
| --- | --- |
| Fullscreen composer | Drop files onto the terminal or press Alt+V for a clipboard image. |
| `jecode --plain` | Start with `jecode --plain --attach PATH` to attach a file to the first message. Enter on an empty line sends only staged attachments. |
| Single prompts and pipes | `jecode --attach PATH [--attach PATH]... "prompt"` attaches files to the first message. |

Any regular file can be attached, including binaries and files outside the
working directory, up to 1 GiB each. A drop accepts at most 64 paths at once.
Elements appear at the caret after copying; Backspace or Delete
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

`--attach PATH` accepts a path for the first message; repeat the flag for more
files. Paths can be absolute or relative to the working directory. In plain
chat, staged files remain available after exit, `/new`, `/resume`, and deletion
of the source conversation. Open the session in the fullscreen composer to
review or discard staged elements and saved drafts with attachments.

## What the model receives

Jecode keeps the original bytes and sends a manifest with each attachment's
name, `attachment:ID` reference, stored path and delivery status. It builds
provider content from the copy just before a request.

| Kind | Delivery |
| --- | --- |
| PNG, JPEG, WebP, GIF | Original image bytes. A PNG or JPEG up to 16 MiB that exceeds 512 KiB or 2048 pixels on the long side can use a derived view: scaled to at most 2048 pixels, PNG or JPEG quality 85, whichever is smaller. The view is kept only when it is smaller than the original. Images above 16 MiB use their original bytes to avoid loading the whole file for conversion. The stored original never changes. |
| Other image formats | Converted to PNG or JPEG, at most 2048 pixels on the long side. Without a converter, the original stays local and the manifest says so. |
| PDF | Sent as a file part up to 32 MiB. OpenRouter uses its `file-parser`: `native` for a model with file input, otherwise `cloudflare-ai`. OpenAI sends an `input_file` with the PDF bytes. Larger PDFs stay local. |
| Text | Read through `read attachment:ID` with normal pagination. |
| Other binaries | Kept locally; the manifest gives the path for a suitable tool. |

When an OpenRouter model's catalog entry rules out image input, it receives a
note instead of the image. The OpenAI adapter sends a supported image view
without filtering it against the ChatGPT catalog's input modalities; the service
may reject an image for a model that cannot accept it. OpenRouter PDF parse
annotations are stored with the response and reused on later requests, avoiding
a repeat parse.

`read attachment:ID` can revisit copies in the session's project bucket.
Text returns pages; images and PDFs are shown to the model in the next
request; other binaries return a path and an uninterpreted-data note. The
result includes `attached_from`. References remain available after
compaction; see [LONG_WORK.md](LONG_WORK.md). File tools keep their normal
scope, with these references adding access to the stored copies.

User-provided attachments stay in the active conversation view. Media returned
by tools is sent for the next assistant response, including retries and user
messages arriving before that response. Later requests retain the original tool
results and attachment references without resending those media bytes. Read an
older `attachment:ID` again to show it; read several references together for a
visual comparison. This also applies to browser screenshots and tool-read PDFs.

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

`/export` writes only the conversation JSON. Messages and PDF annotations keep
their attachment references, but the referenced files are not copied. See
[EXPORT.md](EXPORT.md).

## Native requirements

| Platform | Clipboard image (Alt+V) | Image conversion |
| --- | --- | --- |
| Windows | Windows PowerShell 5.1 (`powershell.exe`) with System.Windows.Forms | System.Drawing through PowerShell |
| WSL | `powershell.exe` through Windows interop | Same, through interop |
| macOS | `osascript` | `sips` |
| Linux | Embedded adapter using X11 `libX11` or the compositor's Wayland data-control protocol | Desktop `gdk-pixbuf` library through the embedded adapter |

Wayland clipboard access requires the compositor to advertise `ext-data-control-v1`
or `wlr-data-control-unstable-v1`. If it does not, Jecode reports that desktop
clipboard access is unavailable; installing a clipboard utility does not change
the compositor's permission. Image bytes use standard input and output, not
command arguments.
