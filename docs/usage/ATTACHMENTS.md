# Attachments

Files and clipboard images become numbered elements in the draft, such as
`[1# Image]` or `[2# File: report.pdf]`. Each element owns a stored copy of the
data; the label is only how the composer shows it. Typing or pasting text that
looks like a label never creates an attachment.

## Attaching

| Where | How |
| --- | --- |
| Fullscreen composer | Drop files onto the terminal, press **Alt+V** for a clipboard image, or type `/attach PATH...`. |
| `jecode --plain` | `/attach PATH...` stages files for the next message; Enter on an empty line sends only the attachments. |
| Single prompts and pipes | `jecode --attach PATH [--attach PATH]... "prompt"`; the files join the first message. |

Any regular file can be attached, including binaries and files outside the
working directory, up to 1 GiB each and 64 per drop. Elements appear at the
caret once their copy is stored; the composer stays usable meanwhile. Backspace
or Delete removes a whole element. Messages may consist of attachments only.
Wait for importing to finish before pressing Enter; an early Enter keeps the
draft in the composer and sends nothing.

Ctrl+V remains the host's text paste. Jecode reads the clipboard only when you
press Alt+V, and only for an image. A missing image or a failed import shows a
warning and keeps the draft.

### File drops

Terminals deliver a drop as a paste of the file paths. Jecode treats a paste as a
drop only when every path names an existing regular file; anything else stays
pasted text. Quotes and backslash escapes are removed but never evaluated: no
variable expansion, globbing or command execution. Windows Terminal's
double-quoted paths, PowerShell literals with doubled apostrophes,
single-quoted WSL paths and `file://` URIs are recognized.
Under WSL, Windows paths are converted with `wslpath`, which follows the live
mount table, and `\\wsl.localhost\<distro>\...` paths into the running
distribution map to their Linux path.

Webterminal stages files dropped in the browser on the local machine and
pastes their paths the same way, so Jecode needs nothing specific to it. Its
Alt+V reaches Jecode like any other key.

`/attach` accepts quoted or bare paths, absolute or relative to the working
directory; backslashes stay literal so Windows paths need no escaping.
In plain chat it also accepts saved references, for example
`/attach attachment:att-123-456-7`. A resumed draft shows this command beside
its labels so its stored copies can be staged again without the source files.
Plain-chat staging is saved immediately. It survives exit, `/new`, `/resume`
and deletion of the source conversation; an empty line sends the staged items.
Use `/attach --clear` in plain chat to discard the staged list without sending.
Opening that session in the fullscreen composer turns the staging list into
visible draft elements once, while keeping any existing draft text.

## What the model receives

Stored history keeps the message text and attachment metadata. Right before
each request, Jecode builds the provider content from the stored copies, with a
manifest naming every element, its reference (`attachment:ID`), local copy and
how it was sent.

| Kind | Sent as |
| --- | --- |
| PNG, JPEG, WebP, GIF | An image part, unchanged when within 3.75 MB and 8000 pixels. |
| Other images (BMP, TIFF, oversized) | An image part converted to PNG, or JPEG for large photos, at most 2048 pixels on its long side. Without a converter the original stays local and the manifest says so. |
| PDF | A file part with OpenRouter's `file-parser` plugin, chosen explicitly: `native` when the model lists file input, otherwise the free `cloudflare-ai` engine, never the paid OCR default. PDFs over 32 MiB stay local. |
| Text | Not inlined; the model reads it with `read` on `attachment:ID`, with the usual pagination. |
| Other binaries | Not interpreted. The manifest gives the local path for tools suited to the format. |

Models whose catalog entry lacks image input receive a note instead of the
image. OpenRouter returns PDF parse results as message annotations; Jecode
stores them with the assistant message and sends them back so the same file is
not parsed again.

Context accounting weighs images by their pixels (about one token per 750
pixels, between 85 and 1600 per image) and PDFs by page count, not by the size
of their encoded payload.

## Reading attachments again

`read` accepts `attachment:ID` for any attachment of the session's project
bucket. Text attachments return pages; an image or PDF is shown to the model
again in the next request; binaries return their local path and a note that
they were not interpreted. Results include the original location in
`attached_from`. Attachment references stay valid after compaction, because the
request list kept by the summary includes each message's manifest. File tools
keep their usual scope; references only add access to attached copies.

## Storage and retention

Copies live in the project's session bucket, beside the saved sessions:

```text
~/.jecode/sessions/<project>/attachments/<id>.json      metadata
~/.jecode/sessions/<project>/attachments/<id>/<name>    exact bytes
```

The copy is taken once, so later changes to the source file do not affect it.
Images that need conversion also store the converted view. Drafts, plain-chat
staging, queued and paused messages, prompt history and saved messages keep their elements
across cancellation, session switches, restarts and crash recovery.
Images returned inside OpenRouter PDF parser annotations are also stored in
this bucket. The conversation keeps references alongside the parser hash and
text; Jecode restores image data only when sending a later request.

An asset is removed only when no saved session in the bucket refers to it:
- on exit, after a 10-minute grace period that protects imports of other running
  instances;
- when a session is deleted, immediately for the attachments of its messages.

Text-only sessions saved by earlier versions load unchanged.

## Export

`/export` copies every attachment the conversation refers to, including PDF
annotation images, into a folder
beside the JSON, `JECODE-SESSION-....attachments/`, and lists them in a
top-level `attachments` array with paths relative to the export. If a copy
fails, the export is not written.

## Native requirements

| Platform | Clipboard image (Alt+V) | Image conversion |
| --- | --- | --- |
| Windows | Windows PowerShell 5.1 (`powershell.exe`) with System.Windows.Forms | System.Drawing through the same PowerShell |
| WSL | `powershell.exe` through Windows interop; fails visibly when interop is off | Same, through interop |
| macOS | `osascript` | `sips` |
| Linux | `wl-paste` (Wayland) or `xclip` (X11) | None; unsupported formats stay local |

Image bytes travel through standard input and output, never through command
arguments. Path conversion under WSL uses `wslpath`.
