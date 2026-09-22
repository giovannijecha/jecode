//! npm is distribution infrastructure; the installed command is the native executable.
use std::{fs, path::Path, process::Command};

pub fn windows(destination: &Path) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let parent = destination
        .parent()
        .ok_or("package destination needs a parent")?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    let target = root
        .join("target")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !parent.starts_with(&target) {
        return Err("package output must be a new directory under target/".into());
    }
    let executable = root.join("target/release/jecode.exe");
    let binary = fs::read(&executable).map_err(|_| "build the Windows release first")?;
    let pe = binary
        .get(0x3c..0x40)
        .map(|p| u32::from_le_bytes(p.try_into().unwrap()) as usize)
        .ok_or("invalid PE header")?;
    if !binary.starts_with(b"MZ")
        || binary.get(pe..pe.saturating_add(6)) != Some(&b"PE\0\0\x64\x86"[..])
    {
        return Err("npm candidate requires a Windows x64 native executable".into());
    }
    let version = format!("jecode {}", env!("CARGO_PKG_VERSION"));
    let output = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != version {
        return Err("native executable version does not match this source".into());
    }
    fs::create_dir(destination).map_err(|_| "package destination must not already exist")?;
    fs::create_dir(destination.join("bin")).map_err(|e| e.to_string())?;
    fs::write(destination.join("bin/jecode.exe"), binary).map_err(|e| e.to_string())?;
    fs::copy(root.join("LICENSE"), destination.join("LICENSE")).map_err(|e| e.to_string())?;
    fs::copy(root.join("README.md"), destination.join("README.md")).map_err(|e| e.to_string())?;
    let manifest = format!(
        r#"{{
  "name": "@giovannijecha/jecode",
  "version": "{}",
  "description": "An owned terminal coding harness in Rust (Windows x64 alpha)",
  "license": "MIT",
  "repository": {{"type": "git", "url": "git+https://github.com/giovannijecha/jecode.git"}},
  "homepage": "https://github.com/giovannijecha/jecode#readme",
  "bugs": {{"url": "https://github.com/giovannijecha/jecode/issues"}},
  "os": ["win32"],
  "cpu": ["x64"],
  "bin": {{"jecode": "bin/jecode.exe"}},
  "files": ["bin/jecode.exe", "README.md", "LICENSE"],
  "publishConfig": {{"access": "public", "tag": "next"}}
}}
"#,
        env!("CARGO_PKG_VERSION")
    );
    fs::write(destination.join("package.json"), manifest).map_err(|e| e.to_string())?;
    println!(
        "Windows npm candidate prepared at {}",
        destination.display()
    );
    println!("No package was published. Inspect npm pack inventory and test an isolated install.");
    Ok(())
}
