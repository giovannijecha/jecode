//! Generates the native Workspace policy for Linux and macOS. The process
//! guardian applies it unchanged; see `src/native/process_confine.h`.
//!
//! Linux can be exercised locally, so Landlock uses an allow list of system
//! locations. Seatbelt starts from `allow default` and denies filesystem access
//! and escape routes: a missing entry in a `deny default` profile would break
//! every command on a Mac, where a mistake here is hard to observe.

use std::path::{Path, PathBuf};

/// System locations a confined Linux command can read and execute.
#[cfg(any(target_os = "linux", test))]
const LINUX_READ: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/etc",
    "/opt",
    "/proc",
    "/sys",
    "/dev",
    "/nix",
    "/snap",
    "/gnu",
    "/home/linuxbrew/.linuxbrew",
];

/// Devices and shared memory a confined Linux command can write.
#[cfg(any(target_os = "linux", test))]
const LINUX_WRITE: &[&str] = &[
    "/dev/null",
    "/dev/zero",
    "/dev/full",
    "/dev/random",
    "/dev/urandom",
    "/dev/tty",
    "/dev/ptmx",
    "/dev/pts",
    "/dev/shm",
];

/// One line per rule: `r` for read and execute, `w` for full access. A path can
/// hold any byte but NUL, so `%` and line breaks are escaped.
#[cfg(any(target_os = "linux", test))]
pub(super) fn landlock_rules(
    read: &[PathBuf],
    write: &[PathBuf],
    extra: &[PathBuf],
    protected_home: Option<&Path>,
) -> Result<Vec<u8>, String> {
    // Landlock grants are additive: a broad system grant cannot subtract a
    // custom JECODE_HOME within it. Refuse rather than expose saved accounts.
    if let Some(home) = protected_home {
        for path in LINUX_READ {
            let root = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
            if home.starts_with(&root) || root.starts_with(home) {
                return Err("Jecode's configuration overlaps a system runtime folder. Workspace commands cannot be confined here; move JECODE_HOME outside system runtime folders or /permissions full.".into());
            }
        }
    }
    let mut rules = Vec::new();
    let mut rule = |kind: u8, path: &[u8]| {
        rules.push(kind);
        for &byte in path {
            match byte {
                b'%' => rules.extend_from_slice(b"%25"),
                b'\n' => rules.extend_from_slice(b"%0A"),
                other => rules.push(other),
            }
        }
        rules.push(b'\n');
    };
    let bytes = |path: &Path| path.as_os_str().as_encoded_bytes().to_vec();
    for path in LINUX_READ {
        rule(b'r', path.as_bytes());
    }
    for path in read.iter().chain(extra) {
        rule(b'r', &bytes(path));
    }
    for path in LINUX_WRITE {
        rule(b'w', path.as_bytes());
    }
    for path in write {
        rule(b'w', &bytes(path));
    }
    Ok(rules)
}

/// Default-allow keeps ordinary command behavior; the denials below are what a
/// Workspace command must not reach. Later rules win over earlier ones.
#[cfg(any(target_os = "macos", test))]
const SEATBELT_BASE: &str = r##"(version 1)
(allow default)
(deny appleevent-send)
(deny user-preference-write)
(deny process-exec
  (literal "/bin/launchctl")
  (literal "/usr/bin/open")
  (literal "/usr/bin/osascript"))
(deny signal)
(allow signal (target same-sandbox))
(deny sysctl-read
  (sysctl-name "kern.procargs")
  (sysctl-name "kern.procargs2"))
(deny mach-lookup)
(allow mach-lookup
  (global-name "com.apple.SecurityServer")
  (global-name "com.apple.SystemConfiguration.DNSConfiguration")
  (global-name "com.apple.SystemConfiguration.configd")
  (global-name "com.apple.analyticsd")
  (global-name "com.apple.bsd.dirhelper")
  (global-name "com.apple.cfprefsd.agent")
  (global-name "com.apple.cfprefsd.daemon")
  (global-name "com.apple.diagnosticd")
  (global-name "com.apple.dnssd.service")
  (global-name "com.apple.logd")
  (global-name "com.apple.logd.events")
  (global-name "com.apple.networkd")
  (global-name "com.apple.ocspd")
  (global-name "com.apple.system.DirectoryService.libinfo_v1")
  (global-name "com.apple.system.logger")
  (global-name "com.apple.system.notification_center")
  (global-name "com.apple.system.opendirectoryd.libinfo")
  (global-name "com.apple.system.opendirectoryd.membership")
  (global-name "com.apple.trustd")
  (global-name "com.apple.trustd.agent")
  (local-name "com.apple.cfprefsd.agent"))
(deny file-link)
(deny file-write*)
(allow file-write*
  (literal "/dev/null")
  (literal "/dev/zero")
  (literal "/dev/random")
  (literal "/dev/urandom")
  (literal "/dev/tty")
  (literal "/dev/ptmx")
  (literal "/dev/stdout")
  (literal "/dev/stderr")
  (literal "/dev/dtracehelper")
  (regex #"^/dev/(fd/[0-9]+|ttys[0-9]+)$"))
(deny file-read-data file-clone process-exec* file-map-executable
  (subpath "/Users")
  (subpath "/System/Volumes/Data/Users")
  (subpath "/Volumes")
  (subpath "/home")
  (subpath "/private/var/root")
  (subpath "/private/var/folders")
  (subpath "/private/tmp"))
"##;

/// Metadata stays readable everywhere, as on Linux: runtimes check each path
/// component from the root. Contents outside the grants do not.
#[cfg(any(target_os = "macos", test))]
pub(super) fn seatbelt_profile(
    read: &[PathBuf],
    write: &[PathBuf],
    protected_home: Option<&Path>,
) -> Result<String, String> {
    let mut profile = String::from(SEATBELT_BASE);
    if let Some(home) = protected_home {
        profile.push_str("(deny file-read-data file-clone process-exec* file-map-executable");
        profile.push_str(&subpath(home)?);
        profile.push_str(")\n");
    }
    let readable = read.iter().chain(write).collect::<Vec<_>>();
    if !readable.is_empty() {
        profile.push_str("(allow file-read-data file-clone process-exec* file-map-executable");
        for path in readable {
            profile.push_str(&subpath(path)?);
        }
        profile.push_str(")\n");
    }
    if !write.is_empty() {
        // A hard link gives its file another name: only one already writable may get one.
        profile.push_str("(allow file-write* file-link");
        for path in write {
            profile.push_str(&subpath(path)?);
        }
        profile.push_str(")\n");
    }
    Ok(profile)
}

#[cfg(any(target_os = "macos", test))]
fn subpath(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .filter(|text| !text.chars().any(char::is_control))
        .ok_or("Workspace commands cannot be confined to a path with control characters; choose a different working folder or /permissions full.")?;
    let quoted = text.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!(" (subpath \"{quoted}\")"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landlock_rules_list_system_locations_grants_and_escape_special_bytes() {
        let rules = landlock_rules(
            &[PathBuf::from("/home/me/.rustup")],
            &[PathBuf::from("/work/100%\nsure")],
            &[PathBuf::from("/run/resolv.conf")],
            None,
        )
        .unwrap();
        let text = String::from_utf8(rules).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        for expected in [
            "r/usr",
            "r/etc",
            "r/home/me/.rustup",
            "r/run/resolv.conf",
            "w/dev/null",
            "w/work/100%25%0Asure",
        ] {
            assert!(lines.contains(&expected), "{expected} missing from {text}");
        }
        // A home directory is never a blanket grant.
        assert!(!lines.iter().any(|line| *line == "r/home" || *line == "r/"));
        assert!(!lines.iter().any(|line| line.starts_with("r/tmp")));
        assert!(!lines.iter().any(|line| line.starts_with("r/run/user")));
    }

    #[test]
    fn seatbelt_profile_denies_before_granting_and_quotes_paths() {
        let profile = seatbelt_profile(
            &[PathBuf::from("/Users/me/.rustup")],
            &[PathBuf::from("/Users/me/my \"quoted\" \\ project")],
            Some(Path::new("/opt/private-jecode")),
        )
        .unwrap();
        let deny_read = profile.find("(deny file-read-data").unwrap();
        let allow_read = profile.find("(allow file-read-data").unwrap();
        let deny_write = profile.find("(deny file-write*)").unwrap();
        let allow_write = profile.rfind("(allow file-write*").unwrap();
        assert!(deny_read < allow_read && deny_write < allow_write);
        assert!(profile[..allow_read].contains("(subpath \"/opt/private-jecode\")"));
        let granted = &profile[allow_read..];
        assert!(granted.contains("(subpath \"/Users/me/.rustup\")"));
        assert!(granted.contains("(subpath \"/Users/me/my \\\"quoted\\\" \\\\ project\")"));
        // A read-only grant must not appear among the writable paths.
        assert!(!profile[allow_write..].contains(".rustup"));
        assert_eq!(profile.matches('(').count(), profile.matches(')').count());
    }

    #[test]
    fn seatbelt_profile_refuses_paths_it_cannot_express() {
        assert!(seatbelt_profile(&[], &[PathBuf::from("/Users/me/bad\nname")], None).is_err());
    }

    #[test]
    fn landlock_refuses_a_custom_session_home_inside_a_system_read_root() {
        let root = std::fs::canonicalize("/opt").unwrap_or_else(|_| PathBuf::from("/opt"));
        let home = root.join("private-jecode");
        let error = landlock_rules(&[], &[], &[], Some(&home)).unwrap_err();
        assert!(error.contains("configuration overlaps a system runtime folder"));
        assert!(landlock_rules(&[], &[], &[], Some(Path::new("/home/me/.jecode"))).is_ok());
    }
}
