//! The explicit Token or OAuth choice among the methods a server offers.
use super::{App, AuthMethod, Field, menus};

impl App {
    pub(super) fn show_mcp_methods(&mut self) {
        let Some(draft) = &self.mcp.draft else {
            self.show_mcp_detail();
            return;
        };
        self.state.selector = Some(menus::methods(&draft.methods));
    }

    /// Offer checked methods; a public server needs no choice.
    pub(super) fn present_mcp_methods(&mut self) {
        let Some(draft) = &mut self.mcp.draft else {
            return;
        };
        if draft.methods == [AuthMethod::None] {
            if draft.sign_in {
                self.mcp.draft = None;
                self.show_mcp_detail();
            } else {
                draft.connection.method = AuthMethod::None;
                draft.connection.token.clear();
                draft.connection.oauth = None;
                self.show_mcp_form();
            }
            self.mcp_notice("This server does not require sign-in");
            return;
        }
        draft.methods.retain(|method| *method != AuthMethod::None);
        self.show_mcp_methods();
    }

    pub(super) fn choose_mcp_method(&mut self, index: usize) {
        let Some(draft) = &mut self.mcp.draft else {
            return;
        };
        let Some(method) = draft.methods.get(index).copied() else {
            return;
        };
        let connection = &mut draft.connection;
        if connection.method != method {
            connection.token.clear();
            connection.oauth = None;
            draft.sign_in_guard = None;
        }
        connection.method = method;
        match method {
            AuthMethod::Token => self.show_mcp_field(Field::Token),
            AuthMethod::Browser | AuthMethod::GitHubDevice => self.start_mcp_sign_in(),
            AuthMethod::None => self.show_mcp_form(),
        }
    }
}
