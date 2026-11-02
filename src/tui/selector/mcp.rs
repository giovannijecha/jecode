use super::{Choice, OptionRow, Purpose, Selector, row};
use crate::tui::mcp::Panel;

impl Selector {
    pub(in crate::tui) fn mcp(
        panel: Panel,
        title: &str,
        parent: Option<&'static str>,
        context: &str,
        rows: Vec<(String, String)>,
    ) -> Self {
        let options: Vec<OptionRow> = rows
            .into_iter()
            .enumerate()
            .map(|(index, (name, fact))| row(name, fact, Choice::Mcp(index)))
            .collect();
        let mut menu = Self::new(title, Purpose::Mcp(panel), options);
        menu.parent = parent;
        menu.context = context.into();
        menu.searchable = matches!(panel, Panel::Tools);
        menu
    }
}
