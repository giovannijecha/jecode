//! The `/terminal` panel: a read-only view of the conversation's persistent
//! processes and their saved output. The registry stays in the core; this
//! module keeps only what the panel shows and its own read positions.

use super::{App, terminal::Key, text};
use crate::tools::processes::{Observer, Page, Process, Window};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(super) const REFRESH: Duration = Duration::from_millis(250);
/// Loaded log text stays within this many saved bytes.
const RETAINED: u64 = 128 * 1024;
pub(super) const STREAMS: [&str; 2] = ["Output", "Errors"];

pub(super) struct Panel {
    pub processes: Vec<Process>,
    pub selected: usize,
    pub logs: Option<Logs>,
    refreshed: Instant,
}

pub(super) struct Logs {
    pub id: String,
    pub stream: usize,
    pub views: [Stream; 2],
}

/// A contiguous, bounded window of one saved stream.
pub(super) struct Stream {
    pub pages: VecDeque<Page>,
    /// First visible row at the last navigation width.
    pub offset: usize,
    /// Character position in the displayed text of the first visible row.
    pub anchor: usize,
    pub follow: bool,
    pub error: Option<String>,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            pages: VecDeque::new(),
            offset: 0,
            anchor: 0,
            follow: true,
            error: None,
        }
    }
}

impl Stream {
    pub fn text(&self) -> String {
        self.pages.iter().map(|page| page.text.as_str()).collect()
    }
    fn start(&self) -> u64 {
        self.pages.front().map_or(0, |page| page.start)
    }
    fn end(&self) -> u64 {
        self.pages.back().map_or(0, |page| page.end)
    }
    pub fn length(&self) -> u64 {
        self.pages.back().map_or(0, |page| page.length)
    }

    fn display_chars(text: &str) -> usize {
        text::clean(text).chars().count()
    }

    fn select_row(&mut self, starts: &[usize], row: usize) {
        self.offset = row;
        self.anchor = starts.get(row).copied().unwrap_or(0);
    }

    /// Appends output after the window; returns the pages trimmed from its start.
    fn append(&mut self, page: Page) -> Vec<Page> {
        if page.start != self.end() {
            self.pages.clear();
        }
        if page.end > page.start || self.pages.is_empty() {
            self.pages.push_back(page);
        } else if let Some(last) = self.pages.back_mut() {
            last.length = page.length;
        }
        let mut trimmed = Vec::new();
        while self.end() - self.start() > RETAINED && self.pages.len() > 1 {
            trimmed.extend(self.pages.pop_front());
        }
        trimmed
    }

    /// Prepends output before the window and trims its end to stay bounded.
    fn prepend(&mut self, page: Page) {
        if page.end != self.start() {
            return;
        }
        self.pages.push_front(page);
        while self.end() - self.start() > RETAINED && self.pages.len() > 1 {
            self.pages.pop_back();
        }
    }
}

impl Panel {
    pub fn new(processes: Vec<Process>) -> Self {
        Self {
            processes,
            selected: 0,
            logs: None,
            refreshed: Instant::now(),
        }
    }

    /// Replaces the list, keeping the selected process by identity.
    fn update(&mut self, processes: Vec<Process>) -> bool {
        let selected = self.processes.get(self.selected).map(|p| p.id.clone());
        self.selected = selected
            .and_then(|id| processes.iter().position(|process| process.id == id))
            .unwrap_or(self.selected)
            .min(processes.len().saturating_sub(1));
        let changed = self.processes != processes;
        self.processes = processes;
        changed
    }

    pub fn selected_process(&self) -> Option<&Process> {
        let id = self.logs.as_ref().map(|logs| logs.id.as_str());
        match id {
            Some(id) => self.processes.iter().find(|process| process.id == id),
            None => self.processes.get(self.selected),
        }
    }
}

impl App {
    /// Opens the panel; `id` opens that process's output directly.
    pub(super) fn open_processes(&mut self, id: &str) {
        self.sync_process_observer();
        if self.state.tool_focus.is_some() {
            self.state.select_tool(None);
        }
        self.state.information = None;
        self.state.suggestions.dismiss();
        self.state.processes = Some(Panel::new(vec![]));
        self.refresh_processes(true);
        if id.is_empty() {
            return;
        }
        let panel = self.state.processes.as_mut().unwrap();
        match panel.processes.iter().position(|process| process.id == id) {
            Some(index) => {
                panel.selected = index;
                self.open_logs();
            }
            None => self.warn(&format!("No process {id} in this conversation")),
        }
    }

    /// Takes the current conversation's observer while this thread holds the
    /// agent; a different conversation closes the panel.
    fn sync_process_observer(&mut self) {
        let Some(agent) = &self.agent else {
            return;
        };
        let current = agent.processes();
        if !current.same(&self.process_observer) {
            self.state.processes = None;
        }
        self.process_observer = current;
    }

    /// Re-reads statuses and followed output at most every `REFRESH`.
    pub(super) fn refresh_processes(&mut self, force: bool) -> bool {
        if self
            .state
            .processes
            .as_ref()
            .is_none_or(|panel| !force && panel.refreshed.elapsed() < REFRESH)
        {
            return false;
        }
        self.sync_process_observer();
        let Some(panel) = &mut self.state.processes else {
            return true;
        };
        panel.refreshed = Instant::now();
        let Some(processes) = self.process_observer.processes() else {
            self.state.processes = None;
            self.warn("The conversation changed · process viewer closed");
            return true;
        };
        let mut dirty = panel.update(processes);
        if let Some(logs) = &mut panel.logs {
            dirty |= follow(&self.process_observer, logs);
        }
        dirty
    }

    fn open_logs(&mut self) {
        let Some(panel) = &mut self.state.processes else {
            return;
        };
        let Some(process) = panel.processes.get(panel.selected) else {
            return;
        };
        let mut logs = Logs {
            id: process.id.clone(),
            stream: 0,
            views: Default::default(),
        };
        follow(&self.process_observer, &mut logs);
        panel.logs = Some(logs);
    }

    pub(super) fn process_key(&mut self, key: Key) -> Result<bool, String> {
        let Some(panel) = &mut self.state.processes else {
            return Ok(false);
        };
        if key.code == 27 || key.ctrl() && key.code == 67 {
            if panel.logs.take().is_none() {
                self.state.processes = None;
            }
            return Ok(false);
        }
        if key.ctrl() || key.alt() {
            return Ok(false);
        }
        let page = if panel.logs.is_some() {
            super::view::process_log_rows(&self.state).1
        } else {
            super::view::process_capacity(&self.state)
        }
        .max(1);
        let panel = self.state.processes.as_mut().unwrap();
        if panel.logs.is_none() {
            let last = panel.processes.len().saturating_sub(1);
            panel.selected = match key.code {
                38 => panel.selected.saturating_sub(1),
                40 => (panel.selected + 1).min(last),
                33 => panel.selected.saturating_sub(page),
                34 => (panel.selected + page).min(last),
                36 => 0,
                35 => last,
                _ => panel.selected,
            };
            if key.code == 13 && !key.shift() {
                self.open_logs();
            }
            return Ok(false);
        }
        match key.code {
            38 => self.scroll_logs(-1),
            40 => self.scroll_logs(1),
            33 => self.scroll_logs(-(page as isize)),
            34 => self.scroll_logs(page as isize),
            36 => self.logs_start(),
            35 => {
                let logs = panel.logs.as_mut().unwrap();
                logs.views[logs.stream].follow = true;
                follow(&self.process_observer, logs);
            }
            9 => {
                let logs = panel.logs.as_mut().unwrap();
                logs.stream = 1 - logs.stream;
                follow(&self.process_observer, logs);
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn process_scroll(&mut self, amount: i16) {
        let amount = amount.clamp(-30, 30);
        let Some(panel) = &mut self.state.processes else {
            return;
        };
        if panel.logs.is_some() {
            self.scroll_logs(amount.into());
        } else if amount < 0 {
            panel.selected = panel.selected.saturating_sub(amount.unsigned_abs().into());
        } else {
            let last = panel.processes.len().saturating_sub(1);
            panel.selected = (panel.selected + amount as usize).min(last);
        }
    }

    /// Scrolls by `delta` rows, loading older or newer saved output at the
    /// window edges. Reaching the latest output resumes following.
    fn scroll_logs(&mut self, delta: isize) {
        let observer = &self.process_observer;
        let (_, capacity) = super::view::process_log_rows(&self.state);
        let width = super::view::process_width(&self.state);
        let Some(logs) = self.state.processes.as_mut().and_then(|p| p.logs.as_mut()) else {
            return;
        };
        let (id, index) = (logs.id.clone(), logs.stream);
        let stream = &mut logs.views[index];
        let mut starts = super::view::log_row_starts(&stream.text(), width);
        let bottom = starts.len().saturating_sub(capacity);
        let offset = if stream.follow {
            bottom
        } else {
            super::view::log_row_at(&starts, stream.anchor).min(bottom)
        };
        let mut anchor = starts.get(offset).copied().unwrap_or(0);
        if delta < 0 {
            let up = delta.unsigned_abs();
            stream.follow = false;
            if offset < up && stream.start() > 0 {
                match observer.read(&id, index, Window::Before(stream.start())) {
                    Ok(page) => {
                        let added = Stream::display_chars(&page.text);
                        if page.end == stream.start() {
                            stream.prepend(page);
                            anchor += added;
                            starts = super::view::log_row_starts(&stream.text(), width);
                        }
                    }
                    Err(error) => stream.error = Some(error),
                }
            }
            let row = super::view::log_row_at(&starts, anchor).saturating_sub(up);
            stream.select_row(&starts, row);
            return;
        }
        if offset.saturating_add(delta as usize) < bottom {
            stream.select_row(&starts, offset + delta as usize);
            return;
        }
        let old_end = stream.end();
        match observer.read(&id, index, Window::After(stream.end())) {
            Ok(page) if page.end > page.start => {
                let continuous = page.start == old_end;
                let trimmed = stream.append(page);
                if continuous {
                    let removed = trimmed
                        .iter()
                        .map(|page| Stream::display_chars(&page.text))
                        .sum::<usize>();
                    anchor = anchor.saturating_sub(removed);
                } else {
                    anchor = 0;
                }
                starts = super::view::log_row_starts(&stream.text(), width);
                let row = super::view::log_row_at(&starts, anchor)
                    .saturating_add(delta as usize)
                    .min(starts.len().saturating_sub(capacity));
                stream.select_row(&starts, row);
            }
            Ok(_) => {
                stream.follow = true;
                stream.offset = bottom;
            }
            Err(error) => stream.error = Some(error),
        }
    }

    fn logs_start(&mut self) {
        let observer = &self.process_observer;
        let Some(logs) = self.state.processes.as_mut().and_then(|p| p.logs.as_mut()) else {
            return;
        };
        let stream = &mut logs.views[logs.stream];
        stream.follow = false;
        stream.offset = 0;
        stream.anchor = 0;
        if stream.start() > 0 || stream.pages.is_empty() {
            match observer.read(&logs.id, logs.stream, Window::After(0)) {
                Ok(page) => {
                    stream.pages = VecDeque::from([page]);
                    stream.error = None;
                }
                Err(error) => stream.error = Some(error),
            }
        }
    }
}

/// Reads output saved since the window when the active stream follows.
fn follow(observer: &Observer, logs: &mut Logs) -> bool {
    let stream = &mut logs.views[logs.stream];
    if !stream.follow {
        return false;
    }
    let window = if stream.pages.is_empty() {
        Window::Tail
    } else {
        Window::After(stream.end())
    };
    let result = observer
        .read(&logs.id, logs.stream, window)
        .and_then(|page| {
            // Far behind: show the newest output instead of catching up page by page.
            if page.end < page.length {
                observer.read(&logs.id, logs.stream, Window::Tail)
            } else {
                Ok(page)
            }
        });
    match result {
        Ok(page) => {
            let before = (stream.start(), stream.end(), stream.length());
            let had_error = stream.error.is_some();
            stream.error = None;
            stream.append(page);
            had_error || before != (stream.start(), stream.end(), stream.length())
        }
        Err(error) => {
            let changed = stream.error.as_ref() != Some(&error);
            stream.error = Some(error);
            changed
        }
    }
}

#[cfg(test)]
mod navigation_tests;
#[cfg(test)]
mod tests;
