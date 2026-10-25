# Copy a response

Type `/copy` to choose text from the latest completed assistant message in the
current conversation, including while the agent works. Choose the whole
response, a fenced code block or an explicit blockquote. Use Up/Down and Enter;
small menus also accept number keys and larger menus support search.
Esc or Ctrl+C closes the menu without stopping active work.

The menu captures its source when opened, so a newer response does not change
an existing choice. Drafts are preserved. Copying makes no model request and
adds no new provider message. Delivery success or failure appears in TUI
feedback, not in the transcript.

The whole-response choice keeps the original Markdown. Code choices remove the
opening and closing fence; quote choices remove one outer `>` marker and its
optional following space. Code inside explicitly marked quotes is also offered.
Other spaces, Unicode and LF/CRLF line endings are preserved, including trailing
spaces and newlines. Previews shorten only the menu display. Current credentials
remain masked. Incomplete streamed text and recovered partial responses are
excluded; a completed assistant message accompanying tool calls is eligible.
`/new` has no copyable response until one completes; `/resume` uses the chosen
session's completed messages.

`jecode --plain` offers the same choices as a numbered menu. Enter chooses the
whole response; `/cancel`, `0` or end-of-input closes it. Clipboard delivery runs
after selection. A single prompt or piped task does not run chat slash commands.

## Native clipboard boundary

| Platform | Delivery | Required availability |
| --- | --- | --- |
| Windows | Fixed Windows PowerShell script using `Set-Clipboard` | Installed Windows PowerShell and an available clipboard |
| macOS | `/usr/bin/pbcopy` with a UTF-8 locale | The macOS system command and pasteboard |
| Linux | OSC 52 request serialized through the TUI's terminal output | A terminal that permits OSC 52 clipboard writes |

Native commands receive the selected UTF-8 text on stdin. Leaving the
conversation or exiting discards pending copy work. Native success is reported
as `Copied ... to clipboard`; failures are visible.

OSC 52 reports `Copy sent to terminal · confirmation unavailable`: terminals may
ignore it or require permission. It is never emitted into redirected output.
OSC 52 requests are limited to 100,000 UTF-8 bytes; larger choices fail
visibly instead of being truncated. A terminal multiplexer may need its own
clipboard setting.
Empty text and embedded NUL characters are rejected.

`pbcopy` automatically interprets EPS/RTF signatures as rich data. To keep those
selected bytes as plain text, such macOS choices use OSC 52 when an interactive
terminal is available, with the same size and confirmation limits.
