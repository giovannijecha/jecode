use crate::{
    file_change::{Entry, Kind},
    tui::line::{Code, Fold, Line, Row},
};
use std::ops::Range;

/// How each line of a hunk folds. Collapsed, context keeps one row and changes
/// two; a changed pair whose fragment would fall past them slides both rows to
/// one shared start, so the partners stay aligned.
pub(super) fn folds(
    hunk: &[Entry],
    partners: &[Option<usize>],
    lines: &[(Line, Option<Range<usize>>)],
    width: usize,
    expanded: bool,
    markup: bool,
) -> Vec<Fold> {
    let mut folds: Vec<_> = hunk
        .iter()
        .map(|entry| Fold {
            rows: (!expanded).then_some(if entry.kind == Kind::Context { 1 } else { 2 }),
            from: 0,
            markup,
        })
        .collect();
    if expanded {
        return folds;
    }
    for (index, partner) in partners.iter().enumerate() {
        let Some(partner) = partner.filter(|&partner| partner > index) else {
            continue;
        };
        let pair = [index, partner];
        let codes = pair.map(|line| Code::new(&lines[line].0));
        let fragments = [0, 1].map(|side| {
            let range = lines[pair[side]].1.clone().unwrap_or_default();
            codes[side].glyph(range.start)..codes[side].glyph(range.end)
        });
        if (0..2).all(|side| {
            shows(
                &codes[side],
                &codes[side].layout(width, folds[pair[side]]),
                fragments[side].clone(),
                6,
            )
        }) {
            continue;
        }
        // Both lines share the text before the fragment, so one start aligns them.
        let longer = usize::from(codes[1].len() > codes[0].len());
        let mut from = start(&codes[longer], fragments[0].start, width);
        // The usual lead preserves context at normal widths. At narrow widths
        // it can consume both rows before the fragment, so move the shared
        // window forward until both sides actually show their changed glyphs.
        let visible = |from, count| {
            (0..2).all(|side| {
                let fold = Fold {
                    from,
                    ..folds[pair[side]]
                };
                shows(
                    &codes[side],
                    &codes[side].layout(width, fold),
                    fragments[side].clone(),
                    count,
                )
            })
        };
        if !visible(from, 6)
            && let Some(next) = (from + 1..=fragments[0].start)
                .find(|&at| visible(at, 6))
                .or_else(|| (from + 1..=fragments[0].start).find(|&at| visible(at, 1)))
        {
            from = next;
        }
        for line in pair {
            folds[line].from = from;
        }
    }
    folds
}

/// Whether the rows show the requested visible glyphs of a nonempty fragment.
/// Spaces dropped at a break count as shown.
fn shows(code: &Code, rows: &[Row], fragment: Range<usize>, count: usize) -> bool {
    if fragment.is_empty() {
        return true;
    }
    let mut visible: Vec<_> = fragment
        .clone()
        .filter(|&at| code.at(at) != ' ')
        .take(count)
        .collect();
    if visible.is_empty() {
        visible.push(fragment.start.min(code.len()).saturating_sub(1));
    }
    visible
        .iter()
        .all(|&at| rows.iter().any(|row| (row.start..row.end).contains(&at)))
}

/// About 12 cells before the fragment at a token boundary, or earlier when the
/// line's tail then fills the first row.
fn start(code: &Code, fragment: usize, width: usize) -> usize {
    let near = near(code, fragment);
    let total = code.cells(0, code.len());
    let mut fill = (1..=code.len())
        .find(|&at| total - code.cells(0, at) <= width.saturating_sub(1))
        .unwrap_or(0);
    while fill < near && !code.boundary(fill) {
        fill += 1;
    }
    near.min(fill)
}

fn near(code: &Code, fragment: usize) -> usize {
    let at = code.cells(0, fragment);
    if at <= 12 {
        return 0;
    }
    let ideal = at - 12;
    (1..fragment)
        .filter(|&start| {
            let cells = code.cells(0, start);
            cells + 16 >= at && cells + 8 <= at && code.boundary(start)
        })
        .min_by_key(|&start| code.cells(0, start).abs_diff(ideal))
        .unwrap_or_else(|| {
            (0..fragment)
                .rev()
                .find(|&start| code.cells(0, start) <= ideal)
                .unwrap_or(0)
        })
}
