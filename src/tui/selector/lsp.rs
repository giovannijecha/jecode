use super::{Choice, Purpose, Selector, row};
use crate::tui::lsp::Panel;

impl Selector {
    pub(in crate::tui) fn lsp(
        panel: Panel,
        title: &str,
        parent: Option<&'static str>,
        context: &str,
        rows: Vec<(String, String)>,
    ) -> Self {
        let options = rows
            .into_iter()
            .enumerate()
            .map(|(index, (name, fact))| row(name, fact, Choice::Lsp(index)))
            .collect();
        let mut menu = Self::new(title, Purpose::Lsp(panel), options);
        menu.parent = parent;
        menu.context = context.into();
        menu
    }
}
