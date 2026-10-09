# Copy a response

Type `/copy` to choose text from the latest completed assistant message in the
current conversation. It opens immediately, including while the agent works.
The selector offers the whole response, fenced code blocks and explicitly
marked blockquotes, with previews. Use Up/Down and Enter; small menus also accept
their displayed numbers. Larger menus support search. Esc or Ctrl+C closes the
copy menu without stopping active work. Ctrl+Q still cancels work and exits.

The menu captures its source when opened. A newer response or a queued prompt
cannot change the text behind an existing choice. Drafts are preserved, and
delivery runs separately from the agent and settings jobs. No model request is
made and the copied payload is not added to the session journal again.
TUI results share the shaded feedback area: success or delivery warnings disappear
after five seconds or editing; errors stay until the next action. They never
become transcript rows. Closing the selector is silent.

The whole-response choice keeps the original Markdown. Code choices remove the
opening and closing fence; quote choices remove one outer `>` marker and its
optional following space. Code inside explicitly marked quotes is also offered.
Remaining spaces, Unicode and LF/CRLF line endings are preserved, including
trailing spaces and newlines. Previews shorten only the menu display. Blocks use
Jecode's supported fence and explicit-quote syntax, rather than a full CommonMark
parser. Current credentials remain masked. Incomplete streamed text and recovered
partial responses are excluded; a completed assistant message that accompanies
tool calls is eligible. `/new` starts with no copyable response, and `/resume`
uses the selected session's completed messages.

`jecode --plain` offers the same choices as a numbered menu. Enter chooses the
whole response; `/cancel`, `0` or end-of-input closes it. Clipboard delivery runs
after selection. A single prompt or piped task does not run chat slash commands.

## Native clipboard boundary

The implementation uses owned Rust code and operating-system facilities, with
no extra crates or third-party clipboard libraries.

| Platform | Delivery | Required availability |
| --- | --- | --- |
| Windows | Fixed Windows PowerShell script using `Set-Clipboard` | Installed Windows PowerShell and an available clipboard |
| macOS | `/usr/bin/pbcopy` with a UTF-8 locale | The macOS system command and pasteboard |
| Linux | OSC 52 request serialized through the TUI's terminal output | A terminal that permits OSC 52 clipboard writes |

Native commands receive the selected UTF-8 text on stdin, never as command code
or arguments. They are bounded and cancellable through the existing process
supervisor. Leaving the conversation or exiting discards pending copy work.
Native success is reported as `Copied ... to clipboard`; failures are visible.

OSC 52 reports `Copy sent to terminal · confirmation unavailable`: terminals may
ignore it or require permission. It is never emitted into redirected output.
Requests are limited to 100,000 UTF-8 bytes; larger choices fail visibly instead
of being truncated. A terminal multiplexer may need its own clipboard setting.
Empty text and embedded NUL characters are rejected.

`pbcopy` automatically interprets EPS/RTF signatures as rich data. To keep those
selected bytes as plain text, such macOS choices use OSC 52 when an interactive
terminal is available, with the same size and confirmation limits.

References: Microsoft's [Set-Clipboard](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/set-clipboard?view=powershell-5.1),
the Apple-authored [pbcopy manual](https://man.freebsd.org/cgi/man.cgi?manpath=macOS+13.6.5&query=pbcopy&sektion=1),
and [xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html).

Ordinary tests use synthetic content and clipboard fixtures; they do not read or
overwrite the user's clipboard. Executed platform coverage and the native Mac
procedure are recorded in [PLATFORMS.md](../development/PLATFORMS.md).
