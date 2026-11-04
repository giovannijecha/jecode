use super::super::{
    App, Kind,
    selector::{ProviderGroup, Selector},
};

impl App {
    pub(in crate::tui) fn settings_menu(&self) -> Selector {
        let settings = &self.config.settings;
        Selector::settings(
            &settings.model,
            settings.effort,
            settings.auto_compact,
            settings.cache_retention,
            &settings.routing_for(&settings.model),
            settings.default_permissions,
            self.config.store.path(),
        )
    }

    pub(in crate::tui) fn context_menu(&self) -> Selector {
        let settings = &self.config.settings;
        Selector::context_settings(settings.auto_compact, settings.cache_retention)
    }

    pub(in crate::tui) fn models_menu(&self, defaults: bool) -> Selector {
        let facts = self.facts();
        let current = if defaults {
            &self.config.settings.model
        } else {
            &facts.model
        };
        let mut menu = Selector::models(&self.catalog, defaults, current);
        // The account the catalog came from rides the rule title.
        if let Some(detail) = self
            .catalog_services
            .as_ref()
            .and_then(|services| self.provider_account_detail(services))
        {
            menu.context = format!("{} · {detail}", menu.context);
        }
        if let Some((_, group)) = self.provider_parent {
            menu.parent = Some(group.label());
        }
        menu
    }

    pub(in crate::tui) fn open_settings(&mut self) {
        self.state.selector = Some(self.settings_menu());
    }

    pub(in crate::tui) fn open_model_providers(&mut self, defaults: bool) {
        self.provider_parent = None;
        let mut services = self.config.settings.configured_services(&self.home());
        let facts = self.facts().into_owned();
        let active = facts.services;
        match services.len() {
            0 => self.finish_setting(
                Kind::Error,
                "No configured provider. Open Settings › Providers.",
                vec![],
            ),
            1 => self.load_catalog_for(
                crate::tui::jobs::CatalogPurpose::Models { defaults, id: None },
                if !facts.needs_configuration
                    && services[0].id() == active.id()
                    && services[0].account() == active.account()
                {
                    active
                } else {
                    services.remove(0)
                },
            ),
            _ => {
                self.state.selector = Some(Selector::provider_groups(
                    &services,
                    defaults,
                    facts.backend_id,
                    &self.config.settings.backend_id,
                    None,
                ))
            }
        }
    }

    pub(in crate::tui) fn open_model_provider_group(
        &mut self,
        defaults: bool,
        group: ProviderGroup,
    ) {
        let services = self
            .config
            .settings
            .configured_services(&self.home())
            .into_iter()
            .filter(|service| ProviderGroup::for_id(service.id()) == group)
            .collect::<Vec<_>>();
        if services.is_empty() {
            self.open_model_providers(defaults);
            return;
        }
        let facts = self.facts().into_owned();
        let active = facts.services;
        let mut menu = Selector::providers(
            &services,
            defaults,
            group,
            facts.backend_id,
            &self.config.settings.backend_id,
        );
        for (option, service) in menu.options.iter_mut().zip(&services) {
            if let Some(detail) = self.provider_account_detail(service) {
                option.fact = detail;
                if active.id() == service.id()
                    && active.account() != service.account()
                    && let Some(current) = self.provider_account_detail(&active)
                {
                    option
                        .fact
                        .push_str(&format!(" · this conversation: {current}"));
                }
            }
        }
        self.state.selector = Some(menu);
    }

    pub(in crate::tui) fn return_to_provider_groups(
        &mut self,
        defaults: bool,
        group: ProviderGroup,
    ) {
        let services = self.config.settings.configured_services(&self.home());
        let facts = self.facts();
        self.state.selector = Some(Selector::provider_groups(
            &services,
            defaults,
            facts.backend_id,
            &self.config.settings.backend_id,
            Some(group),
        ));
    }
}
