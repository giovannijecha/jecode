//! Local checkpoint commands run beside the terminal input loop. The archive
//! command has no cancellation contract, so completion is joined on exit.
use super::{App, Kind, information::Information};
use crate::{agent::checkpoints::CommandWork, checkpoints::render::Output};
use std::thread::{self, JoinHandle};

pub(super) struct Job {
    task: JoinHandle<Result<Output, String>>,
}

impl Job {
    fn start(work: CommandWork) -> Self {
        Self {
            task: thread::spawn(move || work.run()),
        }
    }

    fn finished(&self) -> bool {
        self.task.is_finished()
    }

    fn finish(self) -> Result<Output, String> {
        self.task
            .join()
            .map_err(|_| "Checkpoint worker failed".to_string())?
    }
}

impl App {
    pub(super) fn start_checkpoints(&mut self, arguments: &str) {
        match self.agent.as_ref().unwrap().checkpoints_work(arguments) {
            Ok(work) => {
                self.checkpoint_job = Some(Job::start(work));
                self.state.status = "Checking checkpoints...".into();
                self.state.changed();
            }
            Err(error) => self.local_finish(Kind::Error, &error, vec![]),
        }
    }

    pub(super) fn finish_checkpoints(&mut self, wait: bool) -> bool {
        if self
            .checkpoint_job
            .as_ref()
            .is_none_or(|job| !wait && !job.finished())
        {
            return false;
        }
        // A user may open another sheet while the checkpoint worker runs. Its
        // completion must not replace that sheet or wait behind it for later.
        let occupied = self.state.selector.is_some()
            || self.state.information.is_some()
            || self.state.processes.is_some()
            || self.state.agents_panel.is_some()
            || self.state.ask_panel.is_some()
            || self.state.todo_panel.is_some()
            || self.state.permission.is_some()
            || self.state.quitting.is_some();
        let previous_information = self.state.information.take();
        let result = self.checkpoint_job.take().unwrap().finish();
        match result {
            Ok(output) if output.raw.is_some() => {
                // Recorded content belongs in the panel, not the conversation.
                let redactor = &self.archive.redactor;
                let details = output
                    .details
                    .iter()
                    .map(|(line, text)| (line.clone(), redactor.text(text)))
                    .collect();
                self.state.information =
                    Some(Information::new(redactor.text(&output.title), details));
                self.local_finish_quiet(Kind::Notice, &output.title, vec![]);
            }
            Ok(output) => self.local_finish(Kind::Notice, &output.title, output.details),
            Err(error) => self.local_finish(Kind::Error, &error, vec![]),
        }
        if occupied {
            self.state.information = previous_information;
        }
        true
    }
}
