use super::{App, selector::Purpose};
use crate::cli::Selection;

impl App {
    pub(super) fn prepare_resume(
        &mut self,
        resume: Option<Option<String>>,
        selection: &Selection,
    ) -> Result<(), String> {
        match resume {
            Some(Some(id)) => self.resume_with_selection(&id, selection)?,
            Some(None) => {
                self.open_sessions();
                if let Some(menu) = self.state.selector.as_mut()
                    && let Purpose::Sessions { launch, .. } = &mut menu.purpose
                {
                    *launch = (!selection.is_empty()).then(|| selection.clone());
                }
            }
            None => {}
        }
        Ok(())
    }
}
