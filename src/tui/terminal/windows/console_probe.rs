//! Compile the owned screen probe once per fixture, then launch it directly.
//! Repeated snapshots must not repeatedly start PowerShell and its C# compiler.

use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::OnceLock,
};

pub(super) fn snapshot() -> String {
    let result = Command::new(program())
        .stdin(Stdio::null())
        .output()
        .expect("Could not start console probe");
    assert!(
        result.status.success(),
        "Console probe failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).expect("Console probe did not return UTF-8")
}

/// Sets the console title through the native API, as a shell or user would.
pub(super) fn set_title(title: &str) {
    let status = Command::new(program())
        .args(["title", title])
        .stdin(Stdio::null())
        .status()
        .expect("Could not start console probe");
    assert!(status.success(), "Console probe could not set the title");
}

fn program() -> &'static PathBuf {
    static PROGRAM: OnceLock<PathBuf> = OnceLock::new();
    PROGRAM.get_or_init(compile)
}

fn compile() -> PathBuf {
    // Native fixtures set TEMP/TMP to their isolated Directory. Its owner
    // removes the executable after the fixture and all its children exit.
    let program =
        std::env::temp_dir().join(format!("jecode-console-probe-{}.exe", std::process::id()));
    let script = format!(
        "Add-Type -TypeDefinition @'\n{}\n'@ -OutputAssembly '{}' -OutputType ConsoleApplication",
        include_str!("native_probe.cs"),
        program.to_string_lossy().replace('\'', "''")
    );
    let result = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .output()
        .expect("Could not compile console probe");
    assert!(
        result.status.success() && program.is_file(),
        "Could not compile console probe: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    program
}
