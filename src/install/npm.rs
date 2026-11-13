//! npm and Volta installations stay owned by their package manager: Jecode
//! runs that manager against the installation it was started from. On
//! Windows the running executable is first moved out of the package, so the
//! manager can replace or remove the folder; a failure moves it back.

use super::provenance::{self, Owner};
use super::registry;
use super::system::{self, Moved};
use crate::json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Outcome {
    pub text: String,
    pub value: Value,
}

/// npm beside the Node.js that started the launcher, so a version manager's
/// global interception cannot redirect the install; else npm from PATH.
fn npm() -> Result<Command, String> {
    if let Some(node) = std::env::var_os("JECODE_LAUNCHER_NODE").map(PathBuf::from)
        && node.is_file()
    {
        let directory = node.parent().unwrap_or(Path::new("."));
        for candidate in [
            directory.join("node_modules/npm/bin/npm-cli.js"),
            directory.join("../lib/node_modules/npm/bin/npm-cli.js"),
        ] {
            if candidate.is_file() {
                let mut command = Command::new(&node);
                command.arg(candidate);
                return Ok(command);
            }
        }
    }
    let name = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or(
            "npm was not found; run jecode update from the npm installation's own jecode command",
        )?;
    // Volta's home is `Volta` on Windows and `.volta` on Unix, or VOLTA_HOME.
    let volta_home = std::env::var_os("VOLTA_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from);
    if volta_home.is_some_and(|home| found.starts_with(home))
        || found.components().any(|part| {
            let part = part.as_os_str();
            part.eq_ignore_ascii_case("volta") || part.eq_ignore_ascii_case(".volta")
        })
    {
        return Err("Only Volta's npm shim was found, which would install into Volta instead of this npm prefix; run jecode update through this installation's jecode command".into());
    }
    Ok(Command::new(found))
}

fn volta(home: &Path) -> Result<Command, String> {
    let name = if cfg!(windows) { "volta.exe" } else { "volta" };
    let mut candidates = vec![home.join("bin").join(name)];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|directory| directory.join(name)));
    }
    let found = candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or("Volta was not found; run volta install @giovannijecha/jecode yourself")?;
    let mut command = Command::new(found);
    command.env("VOLTA_HOME", home);
    Ok(command)
}

/// The package manager's output goes to stderr: stdout stays for the result.
fn run(mut command: Command, what: &str) -> Result<(), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(std::io::stderr()))
        .stderr(Stdio::inherit())
        .env("npm_config_update_notifier", "false")
        .env_remove("JECODE_LAUNCHER_NODE");
    let status = crate::auth::isolate_environment(&mut command)
        .status()
        .map_err(|error| format!("Could not start {what}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed ({status})"))
    }
}

fn npm_install(mut command: Command, prefix: &Path, spec: &str, registry: &str) -> Command {
    command
        .args(["install", "--global", "--prefix"])
        .arg(prefix)
        .args(["--registry", registry, spec]);
    command
}

fn volta_install(mut command: Command, spec: &str, registry: &str) -> Command {
    command.args(["install", spec]);
    command.env("npm_config_registry", registry);
    command
}

/// On Windows, moves the running executable out of `package`.
fn release_running(package: &Path) -> Result<Option<Moved>, String> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let (key, name) = provenance::target();
    let binary = package.join("native").join(key).join(name);
    let modules = package
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    system::move_aside(&binary, &[std::env::temp_dir(), modules], ".jecode-running").map(Some)
}

fn finish(moved: Option<Moved>, result: Result<(), String>) -> Result<Vec<PathBuf>, String> {
    match (moved, result) {
        (Some(moved), Ok(())) => Ok(system::remove_later(&[moved.to], &[])),
        (None, Ok(())) => Ok(Vec::new()),
        (Some(moved), Err(error)) => {
            if moved.from.exists() {
                std::fs::remove_file(&moved.from).map_err(|remove| {
                    format!(
                        "{error}; previous executable kept at {}; could not remove the partial replacement {}: {remove}",
                        moved.to.display(),
                        moved.from.display()
                    )
                })?;
            }
            moved.restore().map_err(|restore| {
                format!(
                    "{error}; previous executable kept at {} but could not be restored: {restore}",
                    moved.to.display()
                )
            })?;
            Err(error)
        }
        (None, Err(error)) => Err(error),
    }
}

pub fn update(owner: &Owner, release: &registry::Release) -> Result<Outcome, String> {
    let spec = format!("{}@{}", registry::PACKAGE, release.version);
    let (command, what, package) = match owner {
        Owner::Npm { prefix, package } => (
            npm_install(npm()?, prefix, &spec, &registry::registry()),
            "npm install",
            package,
        ),
        Owner::Volta { home, package } => (
            volta_install(volta(home)?, &spec, &registry::registry()),
            "volta install",
            package,
        ),
        _ => unreachable!("only package-manager installations"),
    };
    let moved = release_running(package)?;
    let (key, name) = provenance::target();
    let result = run(command, what).and_then(|()| {
        super::standalone::validate(
            &package.join("native").join(key).join(name),
            &release.version,
        )
    });
    let deferred = finish(moved, result)?;
    let mut text = format!(
        "Updated {} to Jecode {} with {what}",
        owner.describe(),
        release.version
    );
    if !deferred.is_empty() {
        text.push_str(
            "\nThe previous executable is still running; it is removed after Jecode exits.",
        );
    }
    Ok(Outcome {
        text,
        value: Value::object([
            ("updated", Value::Bool(true)),
            ("version", Value::string(&release.version)),
            ("managed_by", Value::string(owner.label())),
            ("location", Value::string(package.to_string_lossy())),
        ]),
    })
}

pub fn uninstall(owner: &Owner) -> Result<Outcome, String> {
    let (command, what, package) = match owner {
        Owner::Npm { prefix, package } => {
            let mut command = npm()?;
            command
                .args(["uninstall", "--global", "--prefix"])
                .arg(prefix)
                .arg(registry::PACKAGE);
            (command, "npm uninstall", package)
        }
        Owner::Volta { home, package } => {
            let mut command = volta(home)?;
            command.args(["uninstall", registry::PACKAGE]);
            (command, "volta uninstall", package)
        }
        _ => unreachable!("only package-manager installations"),
    };
    let moved = release_running(package)?;
    let deferred = finish(moved, run(command, what))?;
    let mut text = format!("Removed the {} with {what}", owner.describe());
    if !deferred.is_empty() {
        text.push_str("\nThe running executable is removed after Jecode exits.");
    }
    Ok(Outcome {
        text,
        value: Value::object([
            ("removed", Value::Bool(true)),
            ("managed_by", Value::string(owner.label())),
            ("location", Value::string(package.to_string_lossy())),
        ]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_commands_use_the_registry_that_supplied_the_release() {
        let directory = crate::test_support::Directory::new();
        let prefix = directory.path().join("npm prefix with spaces");
        let mirror = "http://127.0.0.1:34567";
        let spec = "@giovannijecha/jecode@9.9.9";
        let npm = npm_install(Command::new("npm"), &prefix, spec, mirror);
        let args: Vec<_> = npm
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "install",
                "--global",
                "--prefix",
                prefix.to_str().unwrap(),
                "--registry",
                mirror,
                spec
            ]
        );
        let volta = volta_install(Command::new("volta"), spec, mirror);
        assert_eq!(volta.get_args().collect::<Vec<_>>(), ["install", spec]);
        assert!(volta.get_envs().any(|(name, value)| {
            name == "npm_config_registry" && value == Some(std::ffi::OsStr::new(mirror))
        }));
    }

    #[test]
    fn failed_manager_replacement_restores_previous_binary() {
        let directory = crate::test_support::Directory::new();
        let binary = directory.path().join("jecode.exe");
        std::fs::write(&binary, b"previous").unwrap();
        let moved = system::move_aside(
            &binary,
            &[directory.path().to_path_buf()],
            ".jecode-running",
        )
        .unwrap();
        std::fs::write(&binary, b"partial replacement").unwrap();
        assert!(finish(Some(moved), Err("manager failed".into())).is_err());
        assert_eq!(std::fs::read(&binary).unwrap(), b"previous");
    }
}
