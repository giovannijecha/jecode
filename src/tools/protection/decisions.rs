use super::*;

impl Protection {
    pub(super) fn prepare_scope(
        &mut self,
        root: &Path,
        action: &str,
        arguments: &Value,
    ) -> Result<(), String> {
        if action == "status"
            && let Some(reason) = arguments.get("reason")
        {
            let reason = reason
                .as_str()
                .ok_or("A status scope reason must be a string")?;
            if !reason.trim().is_empty() {
                self.scope_reason = reason.into();
            }
        }
        let Some(exclusions) = arguments.get("scope_exclusions") else {
            return Ok(());
        };
        let exclusions = exclusions
            .as_array()
            .filter(|paths| paths.len() <= 64)
            .ok_or("scope_exclusions must be an array of at most 64 project paths")?;
        // Empty defaults neither exclude paths nor approve preservation decisions.
        if exclusions.is_empty() {
            return Ok(());
        }
        if action != "status" {
            return Err("scope_exclusions are accepted only by protect status".into());
        }
        if self.scope_reason.is_empty() {
            return Err("Explain why the excluded paths are outside the current preservation requirements with reason".into());
        }
        let mut paths = BTreeSet::new();
        for path in exclusions {
            let absolute =
                files::existing_path(root, path.as_str().ok_or("Excluded paths must be strings")?)?;
            if !absolute.is_file() {
                return Err("Exclude individual related files, not directories".into());
            }
            let relative = absolute.strip_prefix(root).unwrap().to_path_buf();
            if self.covering_directory(&relative).is_some() {
                return Err("This file belongs to a recorded directory preservation scope from an earlier request. Record its current bytes before mutations. Exclusions cannot discard that declaration; release the directory scope only when a newer user request explicitly permits that scope change. Releasing a directory declaration leaves its individual file baselines active".into());
            }
            if self
                .files
                .get(&relative)
                .is_some_and(|entry| !entry.released)
            {
                return Err("A scope exclusion cannot waive a registered baseline; use an authorized release".into());
            }
            paths.insert(relative);
        }
        self.scope_exclusions.extend(paths);
        Ok(())
    }

    pub(super) fn finish_scope_review(&mut self, root: &Path, cancellation: &Cancellation) {
        let view = self.scope_view(root, cancellation);
        let earlier = self
            .files
            .values()
            .any(|entry| !entry.released && entry.request < self.request)
            || self.has_earlier_directory();
        let decided = view
            .get("unclassified_related_files")
            .and_then(Value::as_array)
            .is_some_and(|paths| paths.is_empty());
        let inventory_reviewed = view.get("related_inventory_complete") == Some(&Value::Bool(true))
            || !self.scope_reason.is_empty();
        if !earlier || (decided && inventory_reviewed && !cancellation.requested()) {
            self.reviewed_request = Some(self.request);
            self.scope_blocked = false;
        }
    }
}
