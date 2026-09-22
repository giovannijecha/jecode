//! Cache a block at its original display width, retaining only one render.
//! Complete source lines are stable in our deliberately line-local Markdown
//! subset. The unterminated line is disposable; it never advances parser state.
use super::{
    markdown,
    model::Block,
    style::{Row, Tone, lines, pad},
    text,
};

pub struct BlockLayout {
    width: usize,
    speaker: Option<&'static str>,
    source: String,
    rows: Vec<Row>,
    committed: usize,
    stable_rows: usize,
    state: markdown::State,
    #[cfg(test)]
    pub parsed_bytes: usize,
}
impl BlockLayout {
    pub fn new(width: usize) -> Self {
        Self {
            width,
            speaker: None,
            source: String::new(),
            rows: Vec::new(),
            committed: 0,
            stable_rows: 0,
            state: markdown::State::default(),
            #[cfg(test)]
            parsed_bytes: 0,
        }
    }

    pub fn rows(&mut self, block: &Block) -> &[Row] {
        let same_role = self.speaker == Some(block.speaker);
        if same_role && self.source == block.text {
            return &self.rows;
        }
        // Do not assume append-only callers: edits, truncation and role changes
        // invalidate both the rendered prefix and the fenced-code state.
        if !same_role || !block.text.starts_with(&self.source) {
            self.rows.clear();
            self.committed = 0;
            self.stable_rows = 0;
            self.state = markdown::State::default();
        }
        self.speaker = Some(block.speaker);
        if block.speaker == "You" {
            self.rows.clear();
            self.rows.push(pad(Row::new("", Tone::User), self.width));
            for line in text::wrap(&block.text, self.width.saturating_sub(2).max(1)) {
                self.rows
                    .push(pad(Row::new(format!(" {line}"), Tone::User), self.width));
            }
            self.rows.push(pad(Row::new("", Tone::User), self.width));
        } else if matches!(
            block.speaker,
            "EditPreview" | "CommandPreview" | "Edit" | "Command"
        ) {
            self.rows = super::action_view::receipt(block, self.width);
        } else if matches!(block.speaker, "ToolSummary" | "ToolWarning" | "Tool") {
            self.rows.clear();
            for (index, line) in block.text.split('\n').enumerate() {
                let tone = if block.speaker == "ToolWarning"
                    || line.trim_start().starts_with("failed / ")
                {
                    Tone::Error
                } else if index == 0 {
                    Tone::Heading
                } else {
                    Tone::Muted
                };
                let mut rendered = lines(line, self.width, tone);
                if index == 0
                    && block.speaker == "ToolSummary"
                    && let Some(row) = rendered.first_mut()
                    && row.text.starts_with('✓')
                {
                    row.spans.push((0..'✓'.len_utf8(), Tone::Success));
                }
                self.rows.extend(rendered);
            }
        } else if matches!(block.speaker, "Error" | "Status" | "Workspace") {
            // Local outcomes are plain text, not assistant Markdown. Keep them
            // visible in scrollback after the transient composer status clears.
            let tone = if block.speaker == "Error" {
                Tone::Error
            } else {
                Tone::Muted
            };
            self.rows = lines(&block.text, self.width, tone);
        } else {
            self.rows.truncate(self.stable_rows);
            let remaining = &block.text[self.committed..];
            let mut consumed = 0;
            for (index, byte) in remaining.bytes().enumerate() {
                if matches!(byte, b'\n' | b'\r') {
                    let line = &remaining[consumed..index];
                    self.state
                        .line(&text::safe(line), self.width, &mut self.rows);
                    #[cfg(test)]
                    {
                        self.parsed_bytes += line.len();
                    }
                    consumed = index + 1;
                }
            }
            self.committed += consumed;
            self.stable_rows = self.rows.len();
            let tail = &block.text[self.committed..];
            if !tail.is_empty() {
                let mut preview = self.state;
                preview.line(&text::safe(tail), self.width, &mut self.rows);
                #[cfg(test)]
                {
                    self.parsed_bytes += tail.len();
                }
            }
        }
        self.source.clone_from(&block.text);
        &self.rows
    }
}
