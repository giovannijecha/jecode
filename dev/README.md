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
| [`benchmarks/`](benchmarks/README.md) | Manual probes for context, redaction, workspace search, sessions, and transcript rendering. |
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

## Source availability and optional private work

The npm artifact excludes `dev/`, `scripts/`, and `test/`. The source repository
retains everything required by its documented commands, tests, CI, and release
build. Moving exploratory notes to private storage must not break those paths.

Private research can live outside the tracked tree or in ignored `sandbox/`.
It is optional input to an investigation, never a dependency of a public test,
build, fixture, or direction document. Keep public conclusions self-contained.
This structure does not change repository visibility or create a private store.
