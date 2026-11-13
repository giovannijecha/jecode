//! Which installation is running, and who owns it. The running executable's
//! own location decides, never PATH order: npm and Volta packages are
//! recognized by their package directory, standalone installs by the marker
//! `INSTALL.ps1` and `INSTALL.sh` write.

use crate::json::Value;
use std::path::{Path, PathBuf};

pub const MARKER: &str = ".jecode-install";
pub const MARKER_TEXT: &str = "Jecode local Rust installation";

#[derive(Clone, Debug, PartialEq)]
pub enum Owner {
    /// `INSTALL.ps1`/`INSTALL.sh`: `<root>/bin/jecode` beside `<root>/.jecode-install`.
    Standalone {
        root: PathBuf,
        binary: PathBuf,
    },
    /// A global npm installation under `prefix`.
    Npm {
        prefix: PathBuf,
        package: PathBuf,
    },
    /// A package Volta installed into its own image directory.
    Volta {
        home: PathBuf,
        package: PathBuf,
    },
    /// A dependency of the npm project at `project`.
    NpmProject {
        project: PathBuf,
        package: PathBuf,
    },
    Unknown {
        binary: PathBuf,
    },
}

/// npm's key for this platform's native executable directory.
pub fn target() -> (&'static str, &'static str) {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => ("win32-x64", "jecode.exe"),
        ("macos", "aarch64") => ("darwin-arm64", "jecode"),
        ("macos", _) => ("darwin-x64", "jecode"),
        _ => ("linux-x64", "jecode"),
    }
}

/// Whether releases include an executable for this platform: `target`
/// names the closest published one, which may not run here.
pub fn published() -> bool {
    published_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn published_for(os: &str, arch: &str) -> bool {
    matches!(
        (os, arch),
        ("windows", "x86_64") | ("macos", "aarch64" | "x86_64") | ("linux", "x86_64")
    )
}

impl Owner {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Standalone { .. } => "standalone",
            Self::Npm { .. } => "npm",
            Self::Volta { .. } => "volta",
            Self::NpmProject { .. } => "npm-project",
            Self::Unknown { .. } => "unknown",
        }
    }

    pub fn location(&self) -> &Path {
        match self {
            Self::Standalone { root, .. } => root,
            Self::Npm { package, .. }
            | Self::Volta { package, .. }
            | Self::NpmProject { package, .. } => package,
            Self::Unknown { binary } => binary,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Standalone { root, .. } => {
                format!("standalone installation at {}", root.display())
            }
            Self::Npm { prefix, .. } => {
                format!("npm global installation (prefix {})", prefix.display())
            }
            Self::Volta { home, .. } => {
                format!("npm package managed by Volta ({})", home.display())
            }
            Self::NpmProject { project, .. } => {
                format!("dependency of the npm project at {}", project.display())
            }
            Self::Unknown { binary } => format!(
                "unrecognized installation at {} (for example a build from source)",
                binary.display()
            ),
        }
    }

    pub fn value(&self) -> Value {
        Value::object([
            ("managed_by", Value::string(self.label())),
            ("location", Value::string(self.location().to_string_lossy())),
            ("description", Value::string(self.describe())),
        ])
    }
}

fn named(path: &Path, name: &str) -> bool {
    path.file_name().is_some_and(|value| value == name)
}

fn standalone(binary: &Path) -> Option<Owner> {
    let bin = binary.parent().filter(|bin| named(bin, "bin"))?;
    let root = bin.parent()?;
    let marker = root.join(MARKER);
    let metadata = std::fs::symlink_metadata(&marker).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(&marker).ok()?;
    (text.trim() == MARKER_TEXT && named(binary, target().1)).then(|| Owner::Standalone {
        root: root.to_path_buf(),
        binary: binary.to_path_buf(),
    })
}

fn package_name(package: &Path) -> Option<String> {
    let text = std::fs::read_to_string(package.join("package.json")).ok()?;
    crate::json::parse(&text)
        .ok()?
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn npm(binary: &Path) -> Option<Owner> {
    let (key, name) = target();
    if !named(binary, name) {
        return None;
    }
    let platform = binary.parent().filter(|dir| named(dir, key))?;
    let native = platform.parent().filter(|dir| named(dir, "native"))?;
    let package = native.parent().filter(|dir| named(dir, "jecode"))?;
    let scope = package
        .parent()
        .filter(|dir| named(dir, "@giovannijecha"))?;
    let modules = scope.parent().filter(|dir| named(dir, "node_modules"))?;
    if package_name(package).as_deref() != Some(super::registry::PACKAGE) {
        return None;
    }
    let package = package.to_path_buf();
    let parent = modules.parent()?;
    // Volta keeps each package in <home>/tools/image/packages/<name>/.
    let components: Vec<_> = package.components().collect();
    if let Some(index) = components.windows(3).position(|window| {
        window[0].as_os_str() == "tools"
            && window[1].as_os_str() == "image"
            && window[2].as_os_str() == "packages"
    }) {
        let home: PathBuf = components[..index].iter().collect();
        return Some(Owner::Volta { home, package });
    }
    let (prefix, shim) = if cfg!(windows) {
        (parent.to_path_buf(), parent.join("jecode.cmd"))
    } else if named(parent, "lib") {
        let prefix = parent.parent()?.to_path_buf();
        let shim = prefix.join("bin").join("jecode");
        (prefix, shim)
    } else {
        (parent.to_path_buf(), parent.join("bin").join("jecode"))
    };
    if std::fs::symlink_metadata(&shim).is_ok() {
        Some(Owner::Npm { prefix, package })
    } else {
        Some(Owner::NpmProject {
            project: parent.to_path_buf(),
            package,
        })
    }
}

pub fn detect(binary: &Path) -> Owner {
    let binary = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    let binary = strip_verbatim(binary);
    standalone(&binary)
        .or_else(|| npm(&binary))
        .unwrap_or(Owner::Unknown { binary })
}

pub fn running() -> Result<Owner, String> {
    std::env::current_exe()
        .map(|binary| detect(&binary))
        .map_err(|error| format!("Could not locate the running executable: {error}"))
}

/// Windows canonical paths carry a `\\?\` prefix that users never typed.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Directory;
    use std::fs;

    #[test]
    fn published_targets_match_the_release_artifacts() {
        assert!(published_for("windows", "x86_64"));
        assert!(!published_for("windows", "aarch64"));
        assert!(published_for("linux", "x86_64"));
        assert!(!published_for("linux", "aarch64"));
        assert!(published_for("macos", "aarch64"));
        assert!(published_for("macos", "x86_64"));
    }

    fn file(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn package(modules: &Path) -> PathBuf {
        let (key, name) = target();
        let package = modules.join("node_modules/@giovannijecha/jecode");
        file(
            &package.join("package.json"),
            r#"{"name":"@giovannijecha/jecode","version":"0.0.1"}"#,
        );
        let binary = package.join("native").join(key).join(name);
        file(&binary, "binary");
        binary
    }

    #[test]
    fn owners_come_from_the_running_executable_location_and_markers() {
        let directory = Directory::new();
        let root = directory.path().join("standalone");
        let binary = root.join("bin").join(target().1);
        file(&binary, "binary");
        assert!(matches!(detect(&binary), Owner::Unknown { .. }));
        file(&root.join(MARKER), "Someone else's installation\n");
        assert!(matches!(detect(&binary), Owner::Unknown { .. }));
        file(&root.join(MARKER), "Jecode local Rust installation\n");
        assert!(matches!(detect(&binary), Owner::Standalone { .. }));

        let prefix = directory.path().join("npm prefix");
        let modules = if cfg!(windows) {
            prefix.clone()
        } else {
            prefix.join("lib")
        };
        let binary = package(&modules);
        assert!(matches!(detect(&binary), Owner::NpmProject { .. }));
        let shim = if cfg!(windows) {
            prefix.join("jecode.cmd")
        } else {
            prefix.join("bin/jecode")
        };
        file(&shim, "shim");
        match detect(&binary) {
            Owner::Npm { prefix: found, .. } => {
                assert_eq!(
                    fs::canonicalize(found).unwrap(),
                    fs::canonicalize(&prefix).unwrap()
                )
            }
            other => panic!("expected npm, got {other:?}"),
        }

        let volta = directory.path().join("volta");
        let image = volta.join("tools/image/packages/@giovannijecha/jecode");
        let binary = package(&if cfg!(windows) {
            image.clone()
        } else {
            image.join("lib")
        });
        assert!(matches!(detect(&binary), Owner::Volta { .. }));

        let impostor = directory.path().join("other");
        let binary = package(&impostor);
        file(
            &impostor.join("node_modules/@giovannijecha/jecode/package.json"),
            r#"{"name":"another-package"}"#,
        );
        assert!(matches!(detect(&binary), Owner::Unknown { .. }));
    }
}
