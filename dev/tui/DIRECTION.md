# TUI direction

This is the current design direction, maintained alongside working previews.
It explains choices that are useful today; it is not a permanent interface
specification or a history of every decision. Replace a choice when an explicit
TUI task produces a clearer, tested result.

Product guarantees and the pre-1.0 scope remain in
[`docs/COMPATIBILITY.md`](../../docs/COMPATIBILITY.md). Runtime composition and
state ownership remain in [`docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md#tui-composition).

## Aim

Make a long coding conversation easy to read, with visible tool evidence and
predictable control during work. The newest output and composer should stay
close together. Operational feedback should be easy to find without becoming
part of the conversation or export.

## Current choices

| Area | Direction and reason | Inspect in the lab |
| --- | --- | --- |
| Conversation | A subtle full-width user surface distinguishes literal requests without a leading marker. Its two-cell text inset and half-cell coloured edges stay unchanged. Assistant prose, reasoning, and tools use the full terminal width, aligned with the composer's left edge. Short histories grow upward from the composer. | `golden`, `conversation`, `scroll` |
| Tools | The accepted Branch design places the target above the tool name and outcome, with connectors linking each record to its evidence. A blank row separates calls and messages; reasoning stays close to the following call. Running, waiting, failed, denied, and settled states remain distinguishable. | `tools-live`, `tools-trace`, `tools-lifecycle`, `tools-workflow`, `approve-denied` |
| Output and diffs | Collapsed output keeps four rows, prioritizing a failure diagnostic and the tail. Large diffs show the first and last three changed rows, with change totals, region counts, and an explicit omission count. Ctrl+O reveals all retained output and diff context. | `tools-stream`, `tools-output`, `tools-diff`, `tools-workflow`, `approve-edit` |
| Reasoning | An unlabeled, muted italic three-row tail keeps ongoing reasoning visible without dominating the answer. Full source remains available through expansion after settlement and through export. | `reasoning`, `reasoning-stream` |
| Composer and menus | The full-width dock shell holds editing, completion, selectors, fields, and help. Ribbon menus use a `●` selection marker, a subtle focus band, and aligned values without selected-choice descriptions. Clipped labels or values may use up to two stable overflow rows; timeline previews stay on one row per turn. Shared row and prompt renderers keep progress and cursor placement consistent. | `menu-workflow`, `menu-commands`, `menu-search`, `menu-settings`, `menu-timeline`, `field`, `help` |
| Feedback and guidance | One footer carries identity and replaceable operational state. A blocked send retains its draft. Guidance submitted during work joins the current turn's queue. | `feedback`, `steering` |
| Colour | The current Slate identity separates structure, technical content, prose, muted text, and outcomes through semantic palette roles. Focus remains legible without colour. | `golden`, `menu-permissions`, `markdown` |
| Motion | Only running tool connectors animate. Tool names and code remain still; waiting and settled records have no row flash or spinner. Reduced motion and `NO_COLOR` use static connectors while real output continues to arrive. | `tools-lifecycle`, `tools-stream`, `tools-workflow` |

The opening frame stays on the transcript and empty composer. Additional
banners or repeated control legends consume the space needed for conversation.
Markdown uses unframed prose, quiet fences, and a small code-body indent.
User messages display literal input: Markdown punctuation, indentation, and
explicit line breaks remain visible, with grapheme-safe cell wrapping. Terminal
controls are neutralized before layout, and tabs use the shared two-cell width.
The user surface paints only the inner half of its top and bottom padding rows.
The surrounding text keeps its position; in `NO_COLOR`, those rows stay blank
and the text inset remains visible.
Selectors retain the `●` marker without colour. Labels and values carry the
choice without explanatory rows below it. If narrow widths clip a label or
value, an overflow area reveals the selected content; its bounded height stays
stable during navigation. Remembered permission counts remain beside the policy.
Timeline labels are turn previews: they truncate in place without reserving an
overflow area or repeating the selected preview below the list. Timestamps,
branch marks, and settlement state remain on each choice row.
Model pickers open directly on search and choices; catalogue failures use footer
feedback. Menus stay available during generation; an approval temporarily takes
focus and restores the open menu after the answer. Model and context settings
apply to the next turn, while the footer retains the running turn's identity.
Approvals keep the question, target, scope-bearing choices, and real
shortcuts visible; their Esc action refuses, while other pickers use Esc to
close. Changing selection has no ambient animation. Search and masked fields retain their writable arrow
prompt. These are current solutions to readability and focus, open to revision
within the product's compatibility and accessibility obligations.

## Evaluate the next change

Start with a concrete weakness in a real interaction. Reproduce it with shared
fixtures before comparing alternatives. Check a normal conversation, a long
transcript, tool evidence, an open menu or approval, and the affected failure or
interruption path. Include narrow widths, long content, `NO_COLOR`, reduced
motion, scrolling, and resize where relevant.

Prefer a change that improves the same interaction across those states. A
winning isolated frame is insufficient if it hides evidence, moves the user's
reading position, obscures focus, or makes recovery harder.

Keep active alternatives in [`experiments/`](experiments/README.md). Once a
direction is chosen, update production components, the relevant scenarios and
tests, this document, and user documentation when behavior changes. Remove the
superseded alternative and rationale instead of accumulating competing rules.
Upcoming work is defined by concrete tasks; this file does not invent a feature
roadmap or make visual choices part of the 1.0 compatibility promise.
