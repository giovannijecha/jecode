//! Whole-screen composition from state. Pure: no I/O; time arrives as a
//! parameter and only drives motion.
use super::activity_view::{self, Activity};
use super::block;
use super::caps::Caps;
use super::composer::{self, Footer};
use super::glyph::{Glyphs, glyphs};
use super::model::{Block, Notice, Status};
use super::motion;
use super::picker::{self, Picker};
use super::style::{Row, Tone};
use super::text;

/// Everything one frame shows.
#[derive(Clone, Copy)]
pub struct Screen<'a> {
    pub blocks: &'a [Block],
    /// Assistant text still streaming; rendered after the finished blocks.
    pub live: Option<&'a str>,
    pub activity: Option<Activity>,
    /// Standing warning or failure, shown above the activity row.
    pub notice: Option<&'a Notice>,
    pub draft: &'a str,
    /// Byte offset of the cursor in `draft`.
    pub cursor: usize,
    /// Highlighted command suggestion while the draft is `/name`.
    pub suggestion: usize,
    pub footer: &'a Footer,
    /// An open filterable list takes the composer's place.
    pub picker: Option<&'a Picker>,
    /// Ctrl+O: tool output and diffs in full across the transcript.
    pub expanded: bool,
    /// Messages sent mid-turn, oldest first; each goes out when a turn ends.
    pub queued: &'a [String],
}

/// A resize drag only needs the viewport tail. Settling still lays out and
/// replays every block at the final width, so this cannot omit saved history.
pub fn preview(screen: &Screen, width: usize, height: usize, caps: &Caps, now_ms: u64) -> Vec<Row> {
    let mut tail = *screen;
    let start = tail.blocks.len().saturating_sub(height.saturating_mul(2));
    tail.blocks = &screen.blocks[start..];
    Layout::default().frame_bounded(&tail, width, height, caps, now_ms)
}

/// Transcript, then the transient chrome: one blank row, as between blocks,
/// then the activity row while a turn runs, sitting on the composer.
/// Lays out from scratch; the live loop keeps a `Layout` instead.
#[cfg(test)]
pub fn frame(screen: &Screen, width: usize, caps: &Caps, now_ms: u64) -> Vec<Row> {
    Layout::default().frame(screen, width, caps, now_ms)
}

/// Rows of transcript blocks, kept between frames: a block is laid out again
/// only when it, the width, the glyph set or the expansion changes. Output is
/// identical to `frame`; a paint no longer re-renders the whole transcript.
#[derive(Default)]
pub struct Layout {
    key: (usize, bool, bool),
    /// Per block: the block, the spinner it was drawn with, its rows.
    entries: Vec<(Block, String, Vec<Row>)>,
}

impl Layout {
    #[cfg(test)]
    pub fn frame(&mut self, screen: &Screen, width: usize, caps: &Caps, now_ms: u64) -> Vec<Row> {
        self.frame_bounded(screen, width, usize::MAX, caps, now_ms)
    }

    /// The live terminal has a finite viewport. Keep all transcript rows for
    /// scrollback, but budget transient rows so the editor remains reachable.
    pub fn frame_bounded(
        &mut self,
        screen: &Screen,
        width: usize,
        height: usize,
        caps: &Caps,
        now_ms: u64,
    ) -> Vec<Row> {
        let glyph = glyphs(caps.ascii);
        let spinner = motion::spinner(now_ms, caps);
        let key = (width, caps.ascii, screen.expanded);
        if key != self.key {
            self.key = key;
            self.entries.clear();
        }
        self.entries.truncate(screen.blocks.len());
        let mut out = Vec::new();
        for (index, item) in screen.blocks.iter().enumerate() {
            // Only running tools animate; nothing else depends on the spinner.
            let spin = if running(item) { spinner.as_str() } else { "" };
            let fresh = self
                .entries
                .get(index)
                .is_some_and(|(block, drawn, _)| block == item && drawn == spin);
            if !fresh {
                let rows = block::rows(item, width, glyph, spin, screen.expanded);
                let entry = (item.clone(), spin.to_owned(), rows);
                match self.entries.get_mut(index) {
                    Some(slot) => *slot = entry,
                    None => self.entries.push(entry),
                }
            }
            if index > 0 {
                out.push(Row::blank());
            }
            out.extend_from_slice(&self.entries[index].2);
        }
        live(screen, width, height, caps, now_ms, &mut out);
        out
    }
}

fn running(block: &Block) -> bool {
    match block {
        Block::Tools(tools) => tools.iter().any(|tool| tool.status == Status::Running),
        _ => false,
    }
}

/// The streaming reply, its caret, then the chrome.
fn live(
    screen: &Screen,
    width: usize,
    height: usize,
    caps: &Caps,
    now_ms: u64,
    out: &mut Vec<Row>,
) {
    let glyph = glyphs(caps.ascii);
    if let Some(live) = screen.live {
        if !out.is_empty() {
            out.push(Row::blank());
        }
        let start = out.len();
        out.extend(block::rows(
            &Block::Assistant(live.into()),
            width,
            glyph,
            "",
            false,
        ));
        if out.len() == start {
            out.push(Row::new(block::MARGIN, Tone::Text));
        }
        caret(out.last_mut(), width, caps, now_ms);
    }
    out.extend(chrome(screen, width, height, caps, now_ms));
}

/// The transient rows under the transcript: one blank row, the notice, the
/// activity row while a turn runs, then the picker or the composer.
fn chrome(screen: &Screen, width: usize, height: usize, caps: &Caps, now_ms: u64) -> Vec<Row> {
    let glyph = glyphs(caps.ascii);
    let mut composer = match screen.picker {
        Some(open) => picker::rows(open, width, glyph),
        None => composer::rows(
            screen.draft,
            screen.cursor,
            screen.suggestion,
            screen.activity.is_some(),
            width,
            glyph,
            screen.footer,
        ),
    };
    let mut notice_rows: Vec<Row> = screen.notice.map_or_else(Vec::new, |notice| {
        notice
            .text
            .lines()
            .map(|line| {
                activity_view::notice(
                    &Notice {
                        status: notice.status,
                        text: line.into(),
                    },
                    width,
                    glyph,
                )
            })
            .collect()
    });
    let mut notices = notice_rows.len() + usize::from(screen.activity.is_some());
    let budget = height.saturating_sub(1);
    // Keep the two rules, prompt/query and selected row; remove decorative
    // rows before they could push the editor below the viewport.
    while composer.len() + notices + 1 > budget && composer.len() > 3 {
        let keep = composer
            .iter()
            .position(|row| {
                row.tone == Tone::User || row.spans.iter().any(|span| span.1 == Tone::Cursor)
            })
            .unwrap_or(1);
        let Some(remove) = (2..composer.len()).find(|index| *index != keep) else {
            break;
        };
        composer.remove(remove);
    }
    while composer.len() + notices > budget && notice_rows.len() > 1 {
        // Authentication's verification code precedes the trailing wait hint.
        notice_rows.pop();
        notices -= 1;
    }
    let mut chrome = if composer.len() + notices < budget {
        vec![Row::blank()]
    } else {
        Vec::new()
    };
    let mut queue_room = budget.saturating_sub(chrome.len() + notices + composer.len());
    if !screen.queued.is_empty() && queue_room == 0 {
        chrome.clear();
        queue_room = budget.saturating_sub(notices + composer.len());
    }
    if !screen.queued.is_empty() && queue_room > 0 {
        let visible = if screen.queued.len() > queue_room {
            queue_room.saturating_sub(1)
        } else {
            screen.queued.len()
        };
        for message in screen.queued.iter().take(visible) {
            chrome.push(queued(message, width, glyph));
        }
        if visible < screen.queued.len() {
            let hidden = screen.queued.len() - visible;
            let next = &screen.queued[visible];
            let label = if visible == 0 {
                format!(
                    "{} queued · next: {}",
                    screen.queued.len(),
                    next.lines().next().unwrap_or("")
                )
            } else {
                format!(
                    "{} more queued · next: {}",
                    hidden,
                    next.lines().next().unwrap_or("")
                )
            };
            chrome.push(Row::new(
                format!(
                    "{}{}",
                    block::MARGIN,
                    text::clip(&label, width.saturating_sub(2), glyph.ellipsis)
                ),
                Tone::Muted,
            ));
        }
    }
    chrome.extend(notice_rows);
    if let Some(activity) = &screen.activity {
        chrome.push(activity_view::row(activity, width, caps, glyph, now_ms));
    }
    chrome.extend(composer);
    for row in &mut chrome {
        row.transient = true;
    }
    chrome
}

/// A waiting message: muted `› text · queued`, one row, cut to fit (a
/// multi-row message shows its first row).
fn queued(message: &str, width: usize, glyph: &Glyphs) -> Row {
    let label = format!(" {} queued", glyph.dot);
    let lead = block::MARGIN.len() + text::width(glyph.prompt) + 1;
    let room = width.saturating_sub(lead + text::width(&label) + block::MARGIN.len());
    let first = message.lines().next().unwrap_or("");
    let body = match message.trim_end().contains('\n') {
        true => format!("{first} {}", glyph.ellipsis),
        false => first.into(),
    };
    let mut row = Row::new(block::MARGIN, Tone::Text);
    row.push(glyph.prompt, Tone::Muted)
        .push(" ", Tone::Text)
        .push(&text::clip(&body, room, glyph.ellipsis), Tone::Muted)
        .push(&label, Tone::Muted);
    row
}

/// Blinking caret after streamed prose; code panels already fill the row.
fn caret(row: Option<&mut Row>, width: usize, caps: &Caps, now_ms: u64) {
    let Some(row) = row.filter(|row| row.tone == Tone::Text) else {
        return;
    };
    if let Some(caret) = motion::caret(now_ms, caps)
        && text::width(&row.text) + text::width(caret) < width
    {
        row.push(caret, Tone::Accent);
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
