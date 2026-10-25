use super::{App, Kind};
use crate::{backend, effort::Effort};

impl App {
    pub(super) fn backend_command(&mut self, arguments: &str) {
        if arguments.is_empty() {
            let details = backend::available()
                .iter()
                .map(|info| {
                    (
                        info.id.into(),
                        format!(
                            "{}{}",
                            info.name,
                            if info.id == self.agent.as_ref().unwrap().backend_id() {
                                " · current"
                            } else {
                                ""
                            }
                        ),
                    )
                })
                .collect();
            self.local_finish(
                Kind::Notice,
                "Available backends · use /backend ID MODEL to switch",
                details,
            );
        } else {
            let mut parts = arguments.split_whitespace();
            let (Some(id), Some(model), None) = (parts.next(), parts.next(), parts.next()) else {
                self.local_finish(Kind::Error, "Usage: /backend ID MODEL", vec![]);
                return;
            };
            if !backend::available().iter().any(|info| info.id == id) {
                self.local_finish(
                    Kind::Error,
                    &format!("Backend is not available: {id}"),
                    vec![],
                );
            } else if let Err(error) = crate::config::validate_model(model) {
                self.local_finish(Kind::Error, &error, vec![]);
            } else if let Ok(services) = self.config.settings.services(
                self.config
                    .store
                    .path()
                    .parent()
                    .expect("configuration directory"),
                id,
            ) {
                let previous = {
                    let agent = self.agent.as_ref().unwrap();
                    format!("{} / {}", agent.backend_id(), agent.model())
                };
                let current = self.agent.as_ref().unwrap().services();
                let result = if current.id() == id && current.account() == services.account() {
                    let agent = self.agent.as_mut().unwrap();
                    agent
                        .set_model(model.into())
                        .map(|()| agent.set_effort(Effort::Default))
                } else {
                    services.create(model.into()).and_then(|mut client| {
                        client.set_effort(Effort::Default);
                        self.agent.as_mut().unwrap().replace_backend(client)
                    })
                };
                match result {
                    Ok(()) => {
                        self.archive = self.agent.as_ref().unwrap().archive();
                        self.state.effort = Effort::Default.name().into();
                        self.refresh_context();
                        self.local_finish_change(
                            Kind::Notice,
                            &format!("Backend: {previous} → {id} / {model} · this conversation"),
                            vec![],
                        );
                    }
                    Err(error) => self.local_finish(Kind::Error, &error, vec![]),
                }
            } else {
                let error = self
                    .config
                    .settings
                    .services(
                        self.config
                            .store
                            .path()
                            .parent()
                            .expect("configuration directory"),
                        id,
                    )
                    .err()
                    .unwrap_or_else(|| format!("{id} is unavailable"));
                self.local_finish(Kind::Error, &error, vec![]);
            }
        }
    }
}
