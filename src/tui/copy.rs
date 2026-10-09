use super::{
    App, Feedback, Kind,
    selector::{Purpose, Selector},
};
use crate::clipboard::{Delivery, Job};
use std::io::{self, Write};

impl App {
    pub(super) fn copy_command(&mut self, prompt: &str) -> bool {
        let value = prompt.trim();
        let (name, arguments) = value
            .split_once(char::is_whitespace)
            .map_or((value, ""), |(name, args)| (name, args.trim()));
        if !name.eq_ignore_ascii_case("/copy") {
            return false;
        }
        self.state.information = None;
        if arguments.is_empty() {
            self.open_copy();
        } else {
            self.copy_feedback(Kind::Error, "Usage: /copy".into());
        }
        true
    }

    pub(super) fn copy_selector_open(&self) -> bool {
        self.state
            .selector
            .as_ref()
            .is_some_and(|selector| matches!(selector.purpose, Purpose::Copy))
    }

    pub(super) fn open_copy(&mut self) {
        if self.copy_job.is_some() {
            self.copy_feedback(Kind::Warning, "Copy already in progress".into());
            return;
        }
        match crate::copy::response(&self.archive) {
            Ok(targets) => {
                self.state.selector = Some(Selector::copy(&targets));
                self.copy_targets = targets;
            }
            Err(error) => self.copy_feedback(Kind::Error, error),
        }
    }

    pub(super) fn choose_copy(&mut self, index: usize) {
        let Some(target) = self.copy_targets.get(index) else {
            return;
        };
        let label = super::text::clip(&target.name, 64);
        let text = target.text.clone();
        self.copy_targets.clear();
        self.state.notify_copy(Feedback::progress(
            Kind::Notice,
            format!("Copying {label}…"),
        ));
        #[cfg(not(test))]
        let job = Job::start(text, self.terminal.is_some());
        #[cfg(test)]
        let job = {
            self.copied_text = Some(text);
            Job::fixture(Ok(Delivery::Confirmed))
        };
        self.copy_job = Some((label, job));
    }

    pub(super) fn poll_copy(&mut self) -> bool {
        let mut dirty = false;
        if self
            .copy_job
            .as_ref()
            .is_some_and(|(_, job)| job.finished())
        {
            let (label, job) = self.copy_job.take().unwrap();
            let result = job.finish().and_then(|delivery| match delivery {
                Delivery::Confirmed => Ok((Kind::Notice, format!("Copied {label} to clipboard"))),
                Delivery::Terminal(packet) => {
                    let mut output = io::stdout().lock();
                    output
                        .write_all(packet.as_bytes())
                        .and_then(|()| output.flush())
                        .map_err(|error| format!("Could not send copy to terminal: {error}"))?;
                    Ok((
                        Kind::Warning,
                        "Copy sent to terminal · confirmation unavailable".into(),
                    ))
                }
            });
            match result {
                Ok((kind, text)) => self.copy_feedback(kind, text),
                Err(error) => self.copy_feedback(Kind::Error, format!("Copy failed: {error}")),
            }
            dirty = true;
        }
        dirty
    }

    pub(super) fn cancel_copy(&mut self) {
        drop(self.copy_job.take());
        self.copy_targets.clear();
        self.state.clear_copy_notice();
    }

    fn copy_feedback(&mut self, kind: Kind, text: String) {
        self.state
            .notify_copy(Feedback::result(kind, self.archive.redactor.text(&text)));
    }
}
