# TUI experiments

Use this space only while a concrete TUI question needs comparison. The
production baseline is always the lab's real renderers and shared fixtures.

There are no active experiments. The accepted Branch transcript and Ribbon
menus use the production renderer. Their reusable samples remain in
`tools-workflow` and `menu-workflow`, as described in the [lab guide](../README.md).

## Keep an experiment small

Create a descriptive folder with one `README.md` covering:

- The interaction problem, affected scenes, and observable success criteria.
- The baseline and candidate, with the same content, dimensions, motion, and
  fixture time so comparisons isolate the proposed change.
- How to run the comparison and the constraints it must preserve.
- Current findings and the condition that will end the experiment.

Keep candidate-only code inside that folder. Reuse production components and
lab helpers; avoid copying the entire renderer or creating a second fixture
catalogue. An experiment must not become an import of `src/`, a release-build
requirement, or an undocumented prerequisite for the normal lab and tests.
Screenshots and generated captures follow the shared
[temporary-output lifecycle](../../README.md#temporary-outputs), unless a small,
reviewed asset is necessary to explain a reproducible result.

## Close it deliberately

If accepted, move the implementation into production, retain useful scenarios
and regression tests, and update [`DIRECTION.md`](../DIRECTION.md). Delete the
temporary alternative and duplicate notes. If rejected, remove its code; keep a
short rationale in the reviewed change only when it explains an ongoing tradeoff.
Git history already preserves the investigation. Do not create an archive of
obsolete designs or an append-only decision log.

Optional private research stays outside the repository. Public scenarios and
conclusions must work without access to it.
This folder itself belongs to the reproducible source repository; npm excludes
the entire development environment from the installed product.
