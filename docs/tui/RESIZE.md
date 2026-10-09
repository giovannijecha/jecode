# Fullscreen scrolling and resize

The interactive TUI uses the terminal's alternate screen by default. Jecode owns
the conversation viewport and keeps the composer at the bottom. The previous
shell screen and native scrollback stay outside that buffer. `--plain`, single
prompts and pipes retain line-oriented output. All rendering and adapters are
original owned code using Rust's standard library and system APIs.

## Reading and following

The viewport initially follows the latest output. Page Up/Down scroll by a page
with one row of overlap; the wheel moves three rows per notch. Alt+Home goes to
the conversation start and Alt+End follows the tail. Up/Down remain editor and
prompt-history keys; Ctrl+P/N navigate history directly.

Scrolling upward pauses following. A reading hint reports rows below the visible
page. New messages, streamed updates and tool results do not pull that page back
to the bottom. Scrolling down to the tail resumes following. Wheel events go to
an open selector or information panel first. Mouse clicks and selection are not
implemented; `/copy` copies original completed text.

A reading anchor identifies a conversation block, source line and character in
its displayed text. Rewrapping maps it to the visual row containing that point.
Generated tool previews fall back to their row within the card. An increased
height can leave blank space below an anchored page rather than changing its
reading point. Menus and temporary information share the lower area; on taller
terminals their budget leaves at least one third of the screen for conversation.

Only the active conversation appears. `/new`, resume and current-session deletion
reset its viewport and caches. Previously saved conversations remain available
through `/resume`; the renderer does not retain unrelated conversations for
replay. Provider context, saved journals and JSON exports remain separate from
display layout.

## Resize lifecycle

The Windows adapter sends dimensions from a native console snapshot. The Unix
adapter measures `stty size` every 200 ms. Startup needs no cursor-position query.
The main loop processes bounded batches of input and geometry before drawing.

Each observed dimension change restarts a 75 ms quiet period, including a return
to the previously rendered dimensions. Repeated unchanged samples do not restart
it. During dragging/reflow the current page is temporarily shortened to fit;
the composer and menu use the latest geometry. A reflow hint occupies the normal
gap above the lower area. The reading anchor and draft are retained.

After dimensions settle, width changes rebuild settled source layout across
event-loop passes: up to 64 blocks and an additional 256-row Markdown allowance
per pass. A single non-table source line is parsed/wrapped as one unit and can
exceed that allowance. Tables measure one source row per step and emit up to 32
wrapped rows per step; one batch can cross the remaining allowance. Partial
layouts are discarded on another width change. Height-only changes reuse the
completed width layout and recalculate the visible row budget.

Input, worker events and cancellation continue between passes. The page remains
visible until the new source layout is ready. Mutable streaming/tool blocks are
cached separately and refreshed on conversation changes. Finalization settles
their existing source blocks; rendering never reruns a tool or adds a second copy
of a response. UI feedback and local command records add no transcript rows.

The renderer writes only changed visible rows by absolute position and uses
every terminal column. Explicit cursor moves cancel delayed wrapping after
each row, including the bottom-right cell. The Windows adapter enables
`DISABLE_NEWLINE_AUTO_RETURN` alongside VT output; Unix VT terminals already
use delayed wrapping. Windows restores the caller's exact output mode on exit.
Panel content retains equal two-cell side margins within the full-width surface.
Scrollbar visibility and window padding belong to the terminal host and are
not changed by Jecode. It emits no history line feeds and never uses saved-line
erasure (`CSI 3 J`). Unchanged frames emit
nothing. Output synchronization covers each write and never spans event-loop
passes. The draft/caret, queue, selector query/selection and turn timer survive
resize. With fewer than two rows or columns, painting pauses while input and
worker events continue; usable dimensions restore the latest state.

## Entering and leaving

The native guardian enables the alternate screen, bracketed paste, wheel reports
and hidden hardware cursor before reporting readiness. On Windows it consumes
native wheel records; Unix uses SGR mouse reports. Its private owner pipe handles
normal shutdown and owner death. Cleanup disables mouse/paste reporting, leaves
the alternate screen, restores cursor visibility and restores console modes.
Windows temporarily uses the UTF-8 output code page and restores the prior value.
The Rust owner has a fallback for a failed or killed guardian: it restores Unix
terminal settings or starts the bounded Windows console recovery adapter.

Normal exit cancels and joins active work, saves input, drops the terminal guard,
and then prints a one-line prompt/tool count. A successfully saved conversation
adds a second line with its resume command. Empty/new unsaved conversations do
not get a nonexistent resume command. Autosave errors are printed after shell
restoration. The fullscreen transcript is retained in saved sessions and exports,
not printed in full into the caller's scrollback.
