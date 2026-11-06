//! Locate the installed macOS SDK before a confined Rust command starts.
//! Rust's linker otherwise asks xcrun to locate it, and xcrun writes outside
//! the Workspace temporary area on a cold cache.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "macos")]
pub(super) fn configure(command: &mut Command, sdk_root_explicit: bool) {
    set_sdk_root_if_absent(
        command,
        sdk_root_explicit,
        Path::new("/var/db/xcode_select_link"),
        &[
            Path::new("/Applications/Xcode.app/Contents/Developer"),
            Path::new("/Library/Developer/CommandLineTools"),
        ],
    );
}

pub(super) fn set_sdk_root_if_absent(
    command: &mut Command,
    sdk_root_explicit: bool,
    selected: &Path,
    fallbacks: &[&Path],
) {
    // env_clear can erase an env_remove entry. The caller keeps that explicit
    // choice separately, while get_envs still sees inherited or set values.
    if sdk_root_explicit || command.get_envs().any(|(key, _)| key == "SDKROOT") {
        return;
    }
    let developer_dir = command
        .get_envs()
        .find(|(key, _)| *key == "DEVELOPER_DIR")
        .and_then(|(_, value)| value);
    if let Some(sdk) = discover_sdk(developer_dir, selected, fallbacks) {
        command.env("SDKROOT", sdk);
    }
}

fn discover_sdk(
    developer_dir: Option<&OsStr>,
    selected: &Path,
    fallbacks: &[&Path],
) -> Option<PathBuf> {
    if let Some(developer_dir) = developer_dir {
        let path = Path::new(developer_dir);
        if !path.is_absolute() {
            return None;
        }
        let developer = if path.extension() == Some(OsStr::new("app")) {
            path.join("Contents/Developer")
        } else {
            path.to_path_buf()
        };
        return sdk_in(&developer);
    }
    std::iter::once(selected)
        .chain(fallbacks.iter().copied())
        .find_map(sdk_in)
}

#[cfg(all(test, target_os = "macos"))]
mod native_tests;

fn sdk_in(developer: &Path) -> Option<PathBuf> {
    let developer = std::fs::canonicalize(developer).ok()?;
    [
        developer.join("Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk"),
        developer.join("SDKs/MacOSX.sdk"),
    ]
    .into_iter()
    .find_map(|sdk| std::fs::canonicalize(sdk).ok().filter(|sdk| sdk.is_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;

    fn sdk(developer: &Path, xcode: bool) -> PathBuf {
        let sdk = if xcode {
            developer.join("Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk")
        } else {
            developer.join("SDKs/MacOSX.sdk")
        };
        std::fs::create_dir_all(&sdk).unwrap();
        std::fs::canonicalize(sdk).unwrap()
    }

    fn command_sdk(command: &Command) -> Option<PathBuf> {
        command
            .get_envs()
            .find(|(key, _)| *key == "SDKROOT")
            .and_then(|(_, value)| value)
            .map(PathBuf::from)
    }

    #[test]
    fn explicit_sdk_root_and_removal_override_discovery() {
        let root = Directory::new();
        let developer = root.path().join("Xcode.app/Contents/Developer");
        let discovered = sdk(&developer, true);
        let selected = root.path().join("selected");
        let mut command = Command::new("example");
        command.env("SDKROOT", "explicit-sdk");
        set_sdk_root_if_absent(&mut command, true, &selected, &[&developer]);
        assert_eq!(command_sdk(&command), Some(PathBuf::from("explicit-sdk")));

        let mut command = Command::new("example");
        command.env("SDKROOT", "inherited-sdk");
        set_sdk_root_if_absent(&mut command, false, &selected, &[&developer]);
        assert_eq!(command_sdk(&command), Some(PathBuf::from("inherited-sdk")));

        let mut command = Command::new("example");
        command.env_clear();
        command.env_remove("SDKROOT");
        set_sdk_root_if_absent(&mut command, true, &selected, &[&developer]);
        assert_eq!(command_sdk(&command), None);

        let mut command = Command::new("example");
        set_sdk_root_if_absent(&mut command, false, &selected, &[&developer]);
        assert_eq!(command_sdk(&command), Some(discovered));
    }

    #[test]
    fn developer_dir_selects_xcode_app_or_command_line_tools() {
        let root = Directory::new();
        let app = root.path().join("Xcode-beta.app");
        let developer = app.join("Contents/Developer");
        let xcode = sdk(&developer, true);
        let clt = root.path().join("CommandLineTools");
        let clt_sdk = sdk(&clt, false);
        let selected = root.path().join("selected");

        for (developer, expected) in [
            (app.as_path(), &xcode),
            (developer.as_path(), &xcode),
            (clt.as_path(), &clt_sdk),
        ] {
            let mut command = Command::new("example");
            command.env("DEVELOPER_DIR", developer);
            set_sdk_root_if_absent(&mut command, false, &selected, &[]);
            assert_eq!(command_sdk(&command).as_ref(), Some(expected));
        }

        let mut command = Command::new("example");
        command.env("DEVELOPER_DIR", "Xcode-beta.app");
        set_sdk_root_if_absent(&mut command, false, &selected, &[&clt]);
        assert_eq!(command_sdk(&command), None);
    }

    #[test]
    fn selected_developer_and_fallbacks_are_checked_in_order() {
        let root = Directory::new();
        let selected = root.path().join("selected");
        let chosen = sdk(&selected, true);
        let fallback = root.path().join("fallback");
        let alternate = sdk(&fallback, false);
        let mut command = Command::new("example");
        set_sdk_root_if_absent(&mut command, false, &selected, &[&fallback]);
        assert_eq!(command_sdk(&command), Some(chosen));

        let mut command = Command::new("example");
        set_sdk_root_if_absent(
            &mut command,
            false,
            &root.path().join("missing"),
            &[&fallback],
        );
        assert_eq!(command_sdk(&command), Some(alternate));

        let mut command = Command::new("example");
        set_sdk_root_if_absent(&mut command, false, &root.path().join("missing"), &[]);
        assert_eq!(command_sdk(&command), None);
    }
}
