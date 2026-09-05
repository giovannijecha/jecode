# TUI development lab

The lab makes production TUI states repeatable without running a model turn.
Use it to inspect a change, exercise input, and compare narrow terminals,
colour, reduced motion, and streaming. The current visual direction lives in
[`DIRECTION.md`](DIRECTION.md); temporary alternatives belong in
[`experiments/`](experiments/README.md).

The accepted Branch presentation is part of the production renderer. Open
`npm run tui:lab -- --scene tools-workflow` for a 34-second conversation with
reads, a failing check, a large edit, a new file, and successful verification.
The fixture is synthetic; `n` opens its next paused evidence sample.

The accepted Ribbon menus also use the production renderer. Run
`npm run tui:lab -- --scene menu-workflow --time 4000 --paused` to inspect an
edit approval; `n` cycles eight interaction samples. The selected choice uses
a `●` marker and a subtle focus band in colour. Descriptions and clipped
content get a stable detail area sized to the longest detail in the current
menu, up to two rows. One-line details reserve one row; simple lists need none.

## Run and capture

Run from the repository root with the same Node.js prerequisites as Jecode:

```powershell
npm run tui:lab
npm run tui:lab -- --scene tools-workflow --paused
npm run tui:lab -- --scene menu-workflow --time 4000 --paused
npm run tui:lab -- --scene tools-stream --paused
npm run tui:lab -- --scene menu-providers --size 38x14 --color off
npm run tui:lab -- --list
npm run tui:lab -- --help
```

Interactive mode needs a TTY. It adds three development-control rows above the
production frame. The preview uses the remaining terminal space, capped by
`--size` when supplied; the host never pretends a physical terminal is larger.
The preview stays at the bottom and left edge, with any unused height between
the controls and preview. Its footer remains on the terminal's bottom row;
the requested width is preserved through resizing.

Headless mode writes only the selected preview frame, with no terminal lifecycle or
lab controls. Use npm's silent flag when redirecting a frame:

```powershell
npm run --silent tui:lab -- --render --scene tools-lifecycle --time 3200 --size 100x30 --color off
npm run --silent tui:lab -- --render --scene tools-workflow --time 20400 --size 100x30 --color off
npm run --silent tui:lab -- --render --scene reasoning-stream --time 4800 --size 38x14 --reduced-motion
```

`--time` is an integer in milliseconds, from 0 through 3,600,000. `--size` uses
`columns x rows` without spaces, from `1x1` through `300x200`; small dimensions
exercise the production recovery frame. `--color auto` respects terminal
capabilities and `NO_COLOR`; `--color off` disables colour. A redirected frame
uses plain text. Reduced motion suppresses decorative tool transitions while
fixture output still arrives; pause stops fixture time itself.

## Browse and interact

The catalogue starts with focus. Its controls are separate from preview input:

| Catalogue key | Action |
| --- | --- |
| Arrows or Page Up/Down | Previous or next scene. |
| Home / End | First or last scene. |
| `n` where samples are available | Open the next paused sample, resetting preview edits and scroll. |
| Enter / Tab | Focus the production preview. |
| Space / `p` | Pause or resume fixture playback. |
| `.` | Advance one 80 ms fixture step, including while paused. |
| `r` | Restart the current scene. |
| `m` | Toggle reduced motion. |
| `c` | Toggle colour within terminal capabilities. |
| `[` / `]` | Cycle automatic, 38×14, 100×30, and 160×40 preview sizes. |
| `q`, Ctrl+C, Ctrl+D | Exit. |

Ctrl+G switches focus between catalogue and preview. With preview focus, ordinary
characters reach the production editor or picker, including the letters used
by the catalogue. Production keys exercise completion, grapheme editing,
selection, fields, Page Up/Down, mouse-wheel scrolling, Ctrl+O, and interruption.

The scene registry also maps represented slash commands to their corresponding
previews. `/providers` can navigate its Account and API groups. Permissions
adjust in memory. Other settings, approval, credential, resume, and timeline
selections close their preview without performing the real workflow. Unmapped
commands have no effect beyond normal input acceptance. Text submission appends
a fixed preview response; it never invokes a controller. Fixture completion or
interruption returns unconsumed guidance to the composer.

Scene selection and restart discard preview edits and reset time, selection,
and scrolling. Colour, reduced motion, and playback preference remain. Frames
use the fixture clock for tool duration and footer time; advancing time retains
block identity and user detail expansion. Captures start directly at the
requested fixture time; tool start times and the shared clock keep motion
consistent with playback. Opening a named sample creates its preview at that
time, including the initial menu or approval. Use reduced motion to compare
layout without decorative transitions.

The workflow samples stop at read summaries (8,400 ms), the failing command
(13,600 ms), the running edit (16,800 ms), the running write (20,400 ms), and
the verified answer (33,200 ms). Ctrl+O reveals retained output and full diff
context; Page Up/Down inspects evidence beyond the visible frame.

The menu samples open commands, models, settings, permissions, edit approval,
command approval, a masked field, and help at 0 through 7,000 ms in 1,000 ms
steps. `/models`, `/settings`, `/permissions`, and `/help` stay within this
workflow. Descriptions, approval scope, and `y` / `a` / `n` shortcuts come from
the production factories; changing a sample resets its local edits.

## Change or add a scenario

| Module | Owns |
| --- | --- |
| `main.ts`, `options.ts` | Development entry point, argument validation, help, list, and headless output. |
| `host.ts` | Terminal lifecycle, focus, development keys, playback timer, and preview size. |
| `controller.ts` | Scene selection and fixture time. |
| `preview.ts` | Production input and rendering over an inert memory session. |
| `registry.ts` | Scene identifiers, groups, factories, animation timing, samples, and navigation. |
| `scenarios/` | Conversation, tool, approval, input, session, and configuration states. |
| `fixtures.ts`, `fixtures/` | Shared synthetic content and full workflow source evidence. |
| `scenarios/shared.ts` | Small scene-building helpers that reuse production types. |
| `view.ts` | Headless composition through the same controller and preview. |

Add a factory to the relevant scenario family and register it once. The CLI,
navigation, and frame matrix derive their catalogue from that registration;
picker and completion navigation derive option counts from production data.
Use `state.tick * TICK_MS` for timing and explicit transitions for arriving or
settling evidence. A factory must describe the complete state at the requested
time without requiring earlier frames. Keep a fixture deterministic and free of
network, tool, credential-store, and persistence access. Reuse production picker factories
where available; fixed provider-access examples remain synthetic UI data.

Shipped rendering changes belong in `src/tui/` or `src/ui/`. The catalogue
calls production composition directly. Temporary alternatives belong under
[`experiments/`](experiments/README.md); remove their integration code when the
experiment closes. Optional `moments` expose named fixture times through the
`n` key; optional `routes` map commands to those same inert samples.

Add the scenario that makes the behavior inspectable, then cover the production
behavior and the relevant interactive path in tests. The lab is not a second
TUI implementation or a replacement for tests of real provider, permission,
or session workflows.

## Validate

```powershell
node --test test/tui-lab*.test.ts test/tui-branch.test.ts test/tui-evidence.test.ts test/tui-ribbon.test.ts
npm run typecheck
npm run check
```

The lab tests cover the complete scene registry at multiple terminal sizes,
colour and `NO_COLOR`, reduced motion, fixture progression, interactive input,
headless commands, and host cleanup. Development files are typechecked but are
excluded from release compilation and npm artifacts. Keep generated captures
and machine-specific notes in ignored `sandbox/`.
