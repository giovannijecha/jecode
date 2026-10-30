//! The `/permissions` menus (B12.1, B12.2, B12.5): one row per level with
//! the ` current ` chip, and the grants this conversation remembers.

use super::{Badge, Choice, OptionRow, Purpose, Selector, row};
use crate::permissions::{Grant, Level};

/// The three levels, the focus opening on `current`.
fn levels(current: Level) -> Vec<OptionRow> {
    Level::ALL
        .into_iter()
        .map(|level| OptionRow {
            badge: (level == current).then_some(Badge::Current),
            ..row(level.name(), level.fact(), Choice::Level(level))
        })
        .collect()
}

fn focused(mut menu: Selector, current: Level) -> Selector {
    menu.selected = Level::ALL
        .iter()
        .position(|level| *level == current)
        .unwrap_or(0);
    menu
}

/// `Reads` and the file, or `Runs` and the command with its folder.
pub fn grant_fact(grant: &Grant) -> String {
    match &grant.place {
        Some(place) => format!("{} in {place}", grant.target),
        None => grant.target.clone(),
    }
}

impl Selector {
    pub fn permissions(level: Level, default: Level, grants: usize) -> Self {
        let mut options = levels(level);
        let remembered = match (grants, level) {
            (0, _) => "none".into(),
            (count, Level::Workspace) => format!("{count} for this conversation"),
            (count, level) => format!("{count} · paused at {}", level.name()),
        };
        options.push(row("Remembered", remembered, Choice::Remembered));
        let mut menu = Self::new("Permissions", Purpose::Permissions { default }, options);
        "this conversation".clone_into(&mut menu.context);
        focused(menu, level)
    }

    pub fn default_permissions(current: Level) -> Self {
        let menu = Self::new(
            "Default permissions",
            Purpose::DefaultPermissions,
            levels(current),
        )
        .under_settings(true);
        focused(menu, current)
    }

    pub fn remembered(grants: Vec<Grant>, level: Level) -> Self {
        let options = grants
            .iter()
            .enumerate()
            .map(|(index, grant)| row(grant.kind.name(), grant_fact(grant), Choice::Grant(index)))
            .collect();
        let paused = (level != Level::Workspace).then_some(level);
        let mut menu = Self::new(
            "Remembered",
            Purpose::Remembered {
                display: grants.clone(),
                grants,
                paused,
            },
            options,
        );
        menu.parent = Some("Permissions");
        menu
    }
}
