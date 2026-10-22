//! The identity of a measurement run: source, compiler, system and storage.
use crate::json::Value;
use std::env::consts::{ARCH, OS};
use std::fs;
use std::path::Path;
use std::process::Command;

const WARMUPS: &str = "untimed alternating pairs before each paired editing and Bash \
    series; every other series times all of its samples, so first uses stay in its minimum \
    and maximum.";
const PAUSES: &str = "File-tool pairs follow an untimed pause of 100 ms plus 1 ms per KiB. \
    The pause separates calls; it does not prove background work has finished. The \
    following-point series measures waiting for verification; adjacent edits have no pause.";
const CACHE: &str = "Fixtures are written just before they are measured, so the OS file \
    cache is warm for every sample; no figure is a cold-cache figure. Cold refers only to \
    process and notification-helper start.";

pub fn identity(output: &Path, quick: bool, elapsed: f64) -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = manifest.join("target/test-fixtures");
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let fields = vec![
        ("revision", revision(manifest)),
        ("dirty paths", dirty_paths(manifest)),
        ("source digest", Some(source_digest(manifest))),
        (
            "compiler",
            run("rustc", &["-vV"], manifest).map(|text| text.trim().replace('\n', "; ")),
        ),
        ("profile", Some(profile.into())),
        ("target", Some(format!("{ARCH}-{OS}"))),
        ("operating system", system()),
        ("processor", processor()),
        (
            "logical processors",
            std::thread::available_parallelism()
                .ok()
                .map(|count| count.to_string()),
        ),
        ("memory bytes", memory()),
        ("fixture directory", Some(fixtures.display().to_string())),
        ("fixture filesystem", filesystem(&fixtures)),
        (
            "scan workers",
            Some(super::super::scan::WORKERS.to_string()),
        ),
        ("output directory", Some(output.display().to_string())),
        ("quick run", Some(quick.to_string())),
        (
            "selected workflows",
            Some(std::env::var(super::ONLY).unwrap_or_else(|_| "all".into())),
        ),
        (
            "warm-ups",
            Some(format!("{} {WARMUPS}", super::editing::WARMUPS)),
        ),
        ("pauses", Some(PAUSES.into())),
        ("total seconds", Some(format!("{elapsed:.1}"))),
        ("cache assumptions", Some(CACHE.into())),
    ];
    #[cfg(windows)]
    let fields = {
        let mut fields = fields;
        fields.push((
            "real-time antivirus",
            powershell("(Get-MpComputerStatus).RealTimeProtectionEnabled"),
        ));
        fields
    };
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| {
                let value = value.unwrap_or_else(|| "unknown".into());
                (key.to_string(), Value::string(value))
            })
            .collect(),
    )
    .pretty()
}

fn dirty_paths(manifest: &Path) -> Option<String> {
    run("git", &["status", "--porcelain"], manifest)
        .map(|status| status.lines().count().to_string())
}

fn run(program: &str, arguments: &[&str], directory: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The Git revision, or the one named by the run when the copy has no history.
fn revision(manifest: &Path) -> Option<String> {
    run("git", &["rev-parse", "HEAD"], manifest)
        .or_else(|| std::env::var("JECODE_CHECKPOINT_MEASURE_REVISION").ok())
}

/// Identifies the measured source across platforms and copies.
fn source_digest(manifest: &Path) -> String {
    let mut files = Vec::new();
    let mut pending = vec![manifest.join("src")];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.extend(
        ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"].map(|name| manifest.join(name)),
    );
    let mut named: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(manifest).unwrap_or(&path);
            let name = relative.to_string_lossy().replace('\\', "/");
            (name, fs::read(&path).unwrap_or_default())
        })
        .collect();
    named.sort();
    let mut bytes = Vec::new();
    for (name, content) in &named {
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&(content.len() as u64).to_le_bytes());
        bytes.extend_from_slice(content);
    }
    let digest = super::super::hash::digest(&bytes).0;
    format!(
        "{:016x}{:016x} over {} files",
        digest[0],
        digest[1],
        named.len()
    )
}

#[cfg(windows)]
fn powershell(command: &str) -> Option<String> {
    run(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", command],
        Path::new("."),
    )
}

#[cfg(windows)]
fn system() -> Option<String> {
    powershell(
        "(Get-CimInstance Win32_OperatingSystem | ForEach-Object { \"$($_.Caption) $($_.Version) build $($_.BuildNumber)\" })",
    )
}

#[cfg(windows)]
fn processor() -> Option<String> {
    powershell("(Get-CimInstance Win32_Processor | Select-Object -First 1).Name")
}

#[cfg(windows)]
fn memory() -> Option<String> {
    powershell("(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory")
}

#[cfg(windows)]
fn filesystem(path: &Path) -> Option<String> {
    let text = path.to_string_lossy();
    let letter = text.trim_start_matches(r"\\?\").chars().next()?;
    powershell(&format!(
        "Get-Volume -DriveLetter {letter} | ForEach-Object {{ \"$($_.FileSystemType) on $($_.DriveType) volume {letter}:\" }}"
    ))
}

#[cfg(not(windows))]
fn system() -> Option<String> {
    let release = fs::read_to_string("/etc/os-release").ok()?;
    let name = release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?
        .trim_matches('"');
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    Some(format!("{name}, kernel {}", kernel.trim()))
}

#[cfg(not(windows))]
fn processor() -> Option<String> {
    let info = fs::read_to_string("/proc/cpuinfo").ok()?;
    info.lines()
        .find_map(|line| line.strip_prefix("model name"))
        .map(|line| line.trim_start_matches([' ', '\t', ':']).to_string())
}

#[cfg(not(windows))]
fn memory() -> Option<String> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    let line = info
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    let kib: u64 = line.trim().trim_end_matches("kB").trim().parse().ok()?;
    Some((kib * 1024).to_string())
}

/// The mount holding `path`, from the longest matching mount point.
#[cfg(not(windows))]
fn filesystem(path: &Path) -> Option<String> {
    let mounts = fs::read_to_string("/proc/self/mountinfo").ok()?;
    let path = path.to_string_lossy();
    mounts
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let mount = *fields.get(4)?;
            let separator = fields.iter().position(|field| *field == "-")?;
            let kind = fields.get(separator + 1)?;
            let source = fields.get(separator + 2)?;
            let inside =
                path == mount || path.starts_with(&format!("{}/", mount.trim_end_matches('/')));
            inside.then(|| (mount.len(), format!("{kind} ({source}) mounted at {mount}")))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, description)| description)
}
