//! The `/terminal` panel: active background processes, saved output and manual
//! stop requests. The registry stays in the core; this
//! module keeps only what the panel shows and its own read positions.

use super::{App, terminal::Key};
use crate::tools::processes::{Observer, Page, Process, Window};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub(super) const REFRESH: Duration = Duration::from_millis(250);
pub(super) const STREAMS: [&str; 2] = ["Output", "Errors"];

pub(super) struct Panel {
    pub processes: Vec<Process>,
    pub selected: usize,
    pub logs: Option<Logs>,
    pub confirm_stop: Option<Process>,
    refreshed: Instant,
}

pub(super) struct Logs {
    pub id: String,
    pub process: Option<Process>,
    pub stream: usize,
    pub views: [Stream; 2],
    pub interactive: bool,
    pub input_mode: bool,
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

impl Panel {
    pub fn new(processes: Vec<Process>) -> Self {
        Self {
            processes: processes.into_iter().filter(Process::active).collect(),
            selected: 0,
            logs: None,
            confirm_stop: None,
            refreshed: Instant::now(),
        }
    }

    /// Replaces the list, keeping the selected process by identity.
    fn update(&mut self, mut processes: Vec<Process>) -> bool {
        let mut changed = false;
        if let Some(logs) = &mut self.logs {
            let current = processes
                .iter()
                .find(|process| process.id == logs.id)
                .cloned();
            changed |= logs.process != current;
            logs.process = current;
            if !logs.process.as_ref().is_some_and(Process::active) && logs.input_mode {
                logs.input_mode = false;
                changed = true;
            }
        }
        if let Some(confirm) = &mut self.confirm_stop
            && let Some(current) = processes.iter().find(|process| process.id == confirm.id)
        {
            changed |= confirm != current;
            *confirm = current.clone();
        }
        processes.retain(Process::active);
        let selected = self.processes.get(self.selected).map(|p| p.id.clone());
        self.selected = selected
            .and_then(|id| processes.iter().position(|process| process.id == id))
            .unwrap_or(self.selected)
            .min(processes.len().saturating_sub(1));
        changed |= self.processes != processes;
        self.processes = processes;
        changed
    }

    pub fn selected_process(&self) -> Option<&Process> {
        match &self.logs {
            Some(logs) => logs.process.as_ref(),
            None => self.processes.get(self.selected),
        }
    }
}

impl App {
    /// Opens the panel; `id` opens that process's output directly.
    pub(super) fn open_processes(&mut self, id: &str) {
        self.sync_process_observer();
        self.state.information = None;
        self.state.suggestions.dismiss();
        self.state.processes = Some(Panel::new(vec![]));
        self.refresh_processes(true);
        if self.state.processes.is_none() {
            return;
        }
        if let Some(id) = crate::session::commands::terminal_stop_id(id) {
            self.stop_process(id);
            return;
        }
        if id.is_empty() {
            return;
        }
        let process = self
            .process_observer
            .processes()
            .and_then(|processes| processes.into_iter().find(|process| process.id == id));
        match process {
            Some(process) => {
                let panel = self.state.processes.as_mut().unwrap();
                if let Some(index) = panel.processes.iter().position(|process| process.id == id) {
                    panel.selected = index;
                }
                self.open_process_logs(process);
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
            let terminal = self.process_observer.terminal_size(&logs.id).is_some();
            if terminal && !logs.interactive {
                logs.interactive = true;
                dirty = true;
            }
            if !logs.process.as_ref().is_some_and(Process::active) && logs.input_mode {
                logs.input_mode = false;
                dirty = true;
            }
            dirty |= follow(&self.process_observer, logs);
        }
        if let Some((id, size)) = self.direct_resize_target()
            && self.process_observer.terminal_size(&id) != Some(size)
        {
            if let Err(error) = self.process_observer.resize_direct(&id, size) {
                self.leave_direct_input(&error);
                dirty = true;
            } else {
                dirty = true;
            }
        }
        dirty
    }

    fn open_logs(&mut self) {
        if let Some(process) = self
            .state
            .processes
            .as_ref()
            .and_then(|panel| panel.processes.get(panel.selected))
            .cloned()
        {
            self.open_process_logs(process);
        }
    }

    /// Opens retained output without changing the model's read cursors.
    fn open_process_logs(&mut self, process: Process) {
        let mut logs = Logs {
            id: process.id.clone(),
            process: Some(process),
            stream: 0,
            views: Default::default(),
            interactive: false,
            input_mode: false,
        };
        logs.interactive = self.process_observer.terminal_size(&logs.id).is_some();
        follow(&self.process_observer, &mut logs);
        self.state.processes.as_mut().unwrap().logs = Some(logs);
    }

    pub(super) fn process_key(&mut self, key: Key) -> Result<bool, String> {
        if self.direct_input_active() {
            self.direct_key(key);
            return Ok(false);
        }
        if self.process_control(key) {
            return Ok(false);
        }
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
        if panel.confirm_stop.is_some() {
            return;
        }
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
        let terminal = logs.interactive && index == 0;
        let stream = &mut logs.views[index];
        let mut starts = super::view::processes::row_starts(&stream.display_text(terminal), width);
        let bottom = starts.len().saturating_sub(capacity);
        let offset = if stream.follow {
            bottom
        } else {
            super::view::processes::row_at(&starts, stream.anchor).min(bottom)
        };
        let mut anchor = starts.get(offset).copied().unwrap_or(0);
        if delta < 0 {
            let up = delta.unsigned_abs();
            stream.follow = false;
            if offset < up && stream.start() > 0 {
                match observer.read(&id, index, Window::Before(stream.start())) {
                    Ok(page) => {
                        let added = Stream::display_chars(&page.text, terminal);
                        if page.end == stream.start() {
                            stream.prepend(page);
                            anchor += added;
                            starts = super::view::processes::row_starts(
                                &stream.display_text(terminal),
                                width,
                            );
                        }
                    }
                    Err(error) => stream.error = Some(error),
                }
            }
            let row = super::view::processes::row_at(&starts, anchor).saturating_sub(up);
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
                        .map(|page| Stream::display_chars(&page.text, terminal))
                        .sum::<usize>();
                    anchor = anchor.saturating_sub(removed);
                } else {
                    anchor = 0;
                }
                starts = super::view::processes::row_starts(&stream.display_text(terminal), width);
                let row = super::view::processes::row_at(&starts, anchor)
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

mod control;
mod input;
mod stream;
#[cfg(test)]
use stream::RETAINED;
#[cfg(test)]
mod navigation_tests;
#[cfg(test)]
mod tests;
