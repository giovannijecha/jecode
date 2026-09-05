# Development environments

This directory contains reproducible environments for developing and measuring
Jecode. Interactive terminal work lives in [`tui/`](tui/README.md), and browser
sign-in pages live in [`web/`](web/README.md). Add another environment only when
an explicit product task needs it.

| Location | Responsibility |
| --- | --- |
| [`tui/`](tui/README.md) | Production TUI previews, inert scenarios, interaction checks, and fixture playback. |
| [`web/`](web/README.md) | Production browser sign-in pages with inert success and failure states. |
| [`tui/DIRECTION.md`](tui/DIRECTION.md) | Current TUI direction and the reasons behind choices still in use. |
| [`tui/experiments/`](tui/experiments/README.md) | Temporary investigations with a question, comparable evidence, and an exit condition. |
| [`benchmarks/`](benchmarks/README.md) | Manual probes for context, redaction, workspace search, sessions, transcript rendering, and integrated TUI responsiveness. |
| [`context/`](context/README.md) | Opt-in, bounded numeric diagnostics from real production TUI sessions. |
| [`validation/`](validation/README.md) | Release-candidate scenarios, evidence format, and manual checks that automated tests cannot establish. |
| [`test-support/`](test-support/) | Small shared factories and host harnesses for automated tests, excluded from the release runtime. |
| [`../scripts/`](../scripts/README.md) | Required repository checks, release builds, and package verification. |
| [`../test/`](../test/) | Automated regression tests, including tests of the development environments. |

Install development dependencies with `npm ci --ignore-scripts`, then run
`npm run tui:lab`. Use `npm run tui:lab -- --help` for controls and headless
options. The lab imports production components and input handling; fixtures
never contact providers, execute tools, or write product data.

## Keep information in one place

Public behavior and compatibility belong in [`docs/`](../docs/ARCHITECTURE.md).
Current design reasoning belongs beside its development environment. Temporary
alternatives belong in experiments and leave when the investigation closes.
Avoid separate decision logs that restate the current direction or completed
task lists that duplicate Git history. Markdown document basenames are
uppercase: `README.md`, `DIRECTION.md`, and `FINDINGS.md`.

Use a new module when it owns a distinct concern. A scenario joins the TUI
registry once; related scenes share fixtures and production renderers. Do not
create a framework, package, or parallel renderer merely to organize examples.

Test support follows the same rule: share only fixtures used by multiple suites,
create fresh mutable state per call, and leave setup and cleanup in each suite.
Helpers stay outside `test/` because Node's default test discovery includes
modules throughout that directory, including files without a `.test.ts` suffix.

## Temporary outputs

Store generated logs, captures, benchmark reports, package verification files,
and PR or release drafts in a task-specific directory under the system temporary
directory (`$env:TEMP` in PowerShell or `os.tmpdir()` in Node.js). Remove that
directory when the task closes.

The opt-in [context recorder](context/README.md) uses the development user-data
home's `diagnostics/` directory so a capture remains discoverable after restart.
Keep those bounded recordings local; remove them after retaining the sanitized
evidence needed for the investigation or candidate review.

Reusable tools and active experiments belong in the relevant development
environment. Retain only the reviewed fixtures or evidence needed to reproduce
a result; keep conclusions in the relevant documentation and remove superseded
scripts, comparisons, and drafts.

## Source availability and optional private work

The npm artifact excludes `dev/`, `scripts/`, and `test/`. The source repository
retains everything required by its documented commands, tests, CI, and release
build. Moving exploratory notes to private storage must not break those paths.

Keep private research outside the repository in a deliberate location. It is
optional input to an investigation, never a dependency of a public test, build,
fixture, or direction document. Keep public conclusions self-contained.
This structure does not change repository visibility or create a private store.
