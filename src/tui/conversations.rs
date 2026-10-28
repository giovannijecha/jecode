use super::{App, Feedback, Kind};
use crate::effort::Effort;

impl App {
    pub(super) fn new_conversation_selection(&self) -> Result<(String, String, Effort), Feedback> {
        let agent = self.agent.as_ref().expect("ready conversation");
        let settings = &self.config.settings;
        if settings.has_model() && agent.has_backend(&settings.backend_id) {
            return Ok((
                settings.backend_id.clone(),
                settings.model.clone(),
                settings.effort,
            ));
        }
        if agent.needs_configuration() && !agent.services().is_configured() {
            return Err(Feedback::result(Kind::Warning, "No configured provider")
                .action("/settings", "configure provider"));
        }
        if !agent.has_model() {
            return Err(
                Feedback::result(Kind::Warning, "Choose a model with /model")
                    .action("/model", "choose model"),
            );
        }
        Ok((
            agent.backend_id().into(),
            agent.model().into(),
            agent.effort(),
        ))
    }

    pub(super) fn new_conversation(&mut self) {
        let (backend, model, effort) = match self.new_conversation_selection() {
            Ok(selection) => selection,
            Err(notice) => {
                self.local_start("/new");
                self.local_finish_quiet(notice.kind, &notice.text, vec![]);
                self.state.notify(notice);
                return;
            }
        };
        let defaults = backend == self.config.settings.backend_id
            && model == self.config.settings.model
            && effort == self.config.settings.effort;
        self.cancel_copy();
        let result = self
            .agent
            .as_mut()
            .unwrap()
            .start_new_selection(&backend, model, effort);
        if let Err(error) = result {
            self.local_start("/new");
            self.local_finish(Kind::Error, &error, vec![]);
            return;
        }
        let agent = self.agent.as_ref().unwrap();
        self.archive = agent.archive();
        self.todo_message_len = usize::MAX;
        self.persistence = agent.sessions();
        self.state.effort = agent.effort().name().into();
        self.state.clear();
        self.question_observer = agent.questions();
        self.question_observer.set_available(true);
        self.refresh_question();
        self.refresh_todo();
        self.reset_agents_observer();
        self.refresh_context();
        self.local_start("/new");
        self.local_finish(
            Kind::Notice,
            if defaults {
                "New conversation · saved defaults applied"
            } else {
                "New conversation · current model kept"
            },
            vec![],
        );
    }
}
