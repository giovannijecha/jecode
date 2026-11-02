use super::{App, AuthMethod, Field, Panel, Selector};

impl App {
    pub(super) fn show_mcp_methods(&mut self) {
        let rows = self
            .mcp
            .methods
            .iter()
            .map(|method| {
                let (name, fact) = match method {
                    AuthMethod::None => ("Not needed", "Public server"),
                    AuthMethod::Token => ("Token", "Paste an access token"),
                    AuthMethod::Browser | AuthMethod::GitHubDevice => ("OAuth", "Open browser"),
                };
                (name.into(), fact.into())
            })
            .collect();
        let mut menu = Selector::mcp(
            Panel::Methods,
            "Sign-in",
            Some("Connection"),
            "server options",
            rows,
        );
        if self.mcp.methods.is_empty() {
            menu.message = "No sign-in method offered by this server".into();
        }
        self.state.selector = Some(menu);
    }

    pub(super) fn choose_mcp_method(&mut self, index: usize) {
        let Some(method) = self.mcp.methods.get(index).copied() else {
            return;
        };
        if let Some(draft) = &mut self.mcp.draft {
            if draft.method != method {
                draft.token.clear();
                draft.oauth = None;
            }
            draft.method = method;
        }
        match method {
            AuthMethod::Token => self.show_mcp_field(Field::Token),
            AuthMethod::Browser | AuthMethod::GitHubDevice => self.start_mcp_sign_in(),
            AuthMethod::None => self.show_mcp_form(),
        }
    }
}
