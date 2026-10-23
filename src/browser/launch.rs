use super::{Browser, transport};
use crate::cancel::Cancellation;
use crate::json::Value;
use crate::scratch::Area;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_PROFILE: AtomicU64 = AtomicU64::new(1);

pub(super) fn start(
    area: &Area,
    profiles: Option<&Path>,
    name: Option<&str>,
    visible: bool,
    cancellation: &Cancellation,
) -> Result<Browser, String> {
    let executable = find_browser()?;
    let (profile, lease) = profile(area, profiles, name)?;
    let port_file = profile.join("DevToolsActivePort");
    if port_file.exists() {
        fs::remove_file(&port_file)
            .map_err(|e| format!("Could not clear stale browser endpoint: {e}"))?;
    }
    let mut command = Command::new(executable);
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "about:blank",
        ]);
    crate::auth::isolate_environment(&mut command);
    if !visible {
        command.arg("--headless=new");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if !visible {
            command.creation_flags(crate::process::CREATE_NO_WINDOW);
        }
    }
    let lifetime = Cancellation::default();
    let monitor = lifetime.clone();
    let supervisor = std::thread::spawn(move || {
        crate::process::run_observed(&mut command, None, None, 0, &monitor, None, &mut |_| Ok(()))
    });
    let mut browser = Browser {
        supervisor: Some(supervisor),
        lifetime,
        profile,
        lease,
        port: 0,
        tabs: std::collections::BTreeMap::new(),
        id: String::new(),
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let port = loop {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        if browser
            .supervisor
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            let result = browser
                .supervisor
                .take()
                .unwrap()
                .join()
                .map_err(|_| "Browser supervisor panicked".to_string())?;
            return Err(format!(
                "Browser exited before DevTools started: {:?}",
                result.map(|output| output.exit_code)
            ));
        }
        if let Ok(contents) = fs::read_to_string(browser.profile.join("DevToolsActivePort"))
            && let Some(port) = contents.lines().next().and_then(|s| s.parse::<u16>().ok())
        {
            break port;
        }
        if Instant::now() >= deadline {
            return Err("Browser DevTools startup timed out".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    browser.port = port;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if cancellation.requested() {
            return Err("Operation cancelled".into());
        }
        if let Ok(value) = transport::http_get(port, "/json/list")
            && value.as_array().is_some_and(|tabs| {
                tabs.iter()
                    .any(|t| t.get("type").and_then(Value::as_str) == Some("page"))
            })
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Browser page target did not appear".into());
        }
        cancellation.wait(Duration::from_millis(50))?;
    }
    Ok(browser)
}

pub(super) struct ProfileLease {
    file: File,
}
impl Drop for ProfileLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn profile(
    area: &Area,
    profiles: Option<&Path>,
    name: Option<&str>,
) -> Result<(PathBuf, Option<ProfileLease>), String> {
    let (path, persistent) = if let Some(name) = name {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || name == "."
            || name == ".."
        {
            return Err("Invalid browser profile name".into());
        }
        let root = profiles.ok_or("Persistent browser profiles are unavailable")?;
        crate::disk::private_directory(root).map_err(|e| e.to_string())?;
        if crate::disk::linked(&fs::symlink_metadata(root).map_err(|e| e.to_string())?) {
            return Err("Browser profile root may not be a link".into());
        }
        (root.join(name), true)
    } else {
        let id = NEXT_PROFILE.fetch_add(1, Ordering::Relaxed);
        let (_, marker) = area.file(
            &format!(
                "browser/profiles/run-{}-{id}/.jecode-browser-profile",
                std::process::id()
            ),
            true,
        )?;
        (
            marker
                .parent()
                .ok_or("Invalid temporary browser profile")?
                .to_path_buf(),
            false,
        )
    };
    let marker = path.join(".jecode-browser-profile");
    if path.exists() {
        if crate::disk::linked(&fs::symlink_metadata(&path).map_err(|e| e.to_string())?) {
            return Err("Browser profile may not be a link".into());
        }
        if !persistent && marker.exists() {
            return Err("Temporary browser profile already exists".into());
        }
        if persistent
            && (crate::disk::linked(&fs::symlink_metadata(&marker).map_err(|e| e.to_string())?)
                || fs::read(&marker).map_err(|e| e.to_string())?
                    != b"Jecode dedicated Chromium profile\n")
        {
            return Err("Browser profile has no Jecode ownership marker".into());
        }
        if !persistent {
            fs::write(&marker, b"Jecode dedicated Chromium profile\n")
                .map_err(|e| e.to_string())?;
        }
    } else {
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        fs::write(&marker, b"Jecode dedicated Chromium profile\n").map_err(|e| e.to_string())?;
    }
    let lease_path = path.join(".jecode-lease");
    if lease_path.exists()
        && crate::disk::linked(&fs::symlink_metadata(&lease_path).map_err(|e| e.to_string())?)
    {
        return Err("Browser profile lease may not be a link".into());
    }
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lease_path)
        .map_err(|e| e.to_string())?;
    lease
        .try_lock()
        .map_err(|e| format!("Browser profile is already leased: {e}"))?;
    Ok((path, Some(ProfileLease { file: lease })))
}

fn find_browser() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("JECODE_BROWSER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err("JECODE_BROWSER is not an installed executable file".into());
    }
    #[cfg(windows)]
    {
        for root in [
            std::env::var_os("PROGRAMFILES"),
            std::env::var_os("PROGRAMFILES(X86)"),
            std::env::var_os("LOCALAPPDATA"),
        ]
        .into_iter()
        .flatten()
        {
            for name in [
                "Google/Chrome/Application/chrome.exe",
                "Microsoft/Edge/Application/msedge.exe",
            ] {
                let path = PathBuf::from(&root).join(name);
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    for path in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ] {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    #[cfg(not(windows))]
    let names = [
        "google-chrome",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
    ];
    #[cfg(windows)]
    let names = ["chrome.exe", "msedge.exe"];
    if let Some(path) = std::env::var_os("PATH") {
        for root in std::env::split_paths(&path) {
            for name in names {
                let executable = root.join(name);
                if executable.is_file() {
                    return Ok(executable);
                }
            }
        }
    }
    Err("Chrome or Edge is not installed; set JECODE_BROWSER to its executable".into())
}
