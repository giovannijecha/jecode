use super::super::{
    App, Kind,
    jobs::{CatalogPurpose, Job},
    selector::Selector,
};
use crate::backend;

impl App {
    pub(in crate::tui) fn load_catalog(&mut self, purpose: CatalogPurpose) {
        let home = self
            .config
            .store
            .path()
            .parent()
            .expect("configuration directory");
        let facts = self.facts();
        let id = match &purpose {
            CatalogPurpose::Models { defaults: true, .. }
            | CatalogPurpose::Effort { defaults: true, .. } => {
                self.config.settings.backend_id.as_str()
            }
            _ => facts.backend_id,
        };
        let services = if id == facts.backend_id {
            Ok(facts.services.clone())
        } else {
            self.config.settings.services(home, id)
        };
        drop(facts);
        match services {
            Ok(services) => self.load_catalog_for(purpose, services),
            Err(error) => self.finish_setting(Kind::Error, &error, vec![]),
        }
    }

    pub(in crate::tui) fn load_catalog_for(
        &mut self,
        purpose: CatalogPurpose,
        services: backend::Services,
    ) {
        let (title, context, defaults) = match &purpose {
            CatalogPurpose::Models { defaults: true, .. } => ("Default model", "", true),
            CatalogPurpose::Models { .. } => ("Model", "this conversation", false),
            CatalogPurpose::Effort {
                defaults, model, ..
            } => (
                if *defaults {
                    "Default effort"
                } else {
                    "Effort"
                },
                model.as_str(),
                *defaults,
            ),
        };
        let mut menu = Selector::loading(title, "Loading model catalog…", defaults);
        if self.provider_parent.is_some() {
            menu.parent = Some("Provider");
        }
        context.clone_into(&mut menu.context);
        self.state.selector = Some(menu);
        self.state.clear_notice();
        self.catalog_services = Some(services.clone());
        self.job = Some(Job::catalog(services, purpose));
    }
}
