//! Development commands use only the Rust standard library and installed tools.
mod package;

use std::{
    env, io,
    path::Path,
    process::{Command, ExitCode},
};

fn cargo(args: &[&str]) -> Command {
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(env!("CARGO_MANIFEST_DIR")).args(args);
    command
}

fn run(mut command: Command) -> Result<(), String> {
    let status = command.status().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("check exited with {status}"))
    }
}

fn ownership() -> Result<(), String> {
    let output = cargo(&[
        "tree",
        "--locked",
        "--offline",
        "--all-features",
        "--target",
        "all",
        "--edges",
        "normal,build,dev",
        "--prefix",
        "none",
    ])
    .output()
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("cannot inspect the complete Cargo dependency graph".into());
    }
    let graph = String::from_utf8(output.stdout).map_err(|_| "Cargo graph is not UTF-8")?;
    let rows: Vec<_> = graph.lines().filter(|line| !line.is_empty()).collect();
    let expected = format!("jecode v{} (", env!("CARGO_PKG_VERSION"));
    if rows.len() != 1 || !rows[0].starts_with(&expected) {
        return Err("ownership gate: expected one Jecode package and zero dependencies across all targets and features".into());
    }
    println!("ownership: one package, zero runtime/build/development dependencies");
    Ok(())
}

fn check() -> Result<(), String> {
    ownership()?;
    package_inventory()?;
    run(cargo(&["fmt", "--all", "--", "--check"]))?;
    run(cargo(&[
        "clippy",
        "--locked",
        "--offline",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))?;
    run(cargo(&["test", "--locked", "--offline"]))?;
    Ok(())
}

fn package_inventory() -> Result<(), String> {
    let output = cargo(&[
        "package",
        "--locked",
        "--offline",
        "--list",
        "--allow-dirty",
    ])
    .output()
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("cannot inspect the public source package".into());
    }
    let inventory = String::from_utf8(output.stdout).map_err(|_| "package list is not UTF-8")?;
    let roots = [
        ".cargo/config.toml",
        ".cargo_vcs_info.json",
        "Cargo.toml",
        "Cargo.toml.orig",
        "Cargo.lock",
        "rust-toolchain.toml",
        "README.md",
        "CHANGELOG.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "LICENSE",
    ];
    for path in inventory.lines() {
        let path = path.replace('\\', "/");
        if !roots.contains(&path.as_str())
            && !["src/", "tests/", "checks/", "docs/"]
                .iter()
                .any(|root| path.starts_with(root))
        {
            return Err(
                "unexpected file in public source package; keep local material excluded".into(),
            );
        }
        if path
            .split('/')
            .any(|part| part == "AGENTS.md" || part.starts_with(".jecode-"))
        {
            return Err("private development or recovery file in public source package".into());
        }
    }
    println!("source package: public code, tests and documentation only");
    Ok(())
}

fn release_check(tag: &str) -> Result<(), String> {
    if tag != format!("v{}", env!("CARGO_PKG_VERSION")) {
        return Err("release tag does not match Cargo.toml".into());
    }
    let status = Command::new("git")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .map_err(|error| error.to_string())?;
    if !status.status.success() || !status.stdout.is_empty() {
        return Err("release verification requires a clean, committed source tree".into());
    }
    check()?;
    run(cargo(&[
        "build",
        "--locked",
        "--offline",
        "--release",
        "--bin",
        "jecode",
    ]))?;
    println!("release source verified for {tag}; no artifact was published");
    Ok(())
}

fn entry() -> Result<(), String> {
    if !Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Cargo.toml")
        .is_file()
    {
        return Err("development commands require the source checkout".into());
    }
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "check" => check(),
        [command] if command == "ownership" => ownership(),
        [command, tag] if command == "release-check" => release_check(tag),
        [command, destination] if command == "package-windows" => package::windows(Path::new(destination)),
        _ => {
            Err("usage: cargo run --bin jecode-check -- <check|ownership|release-check TAG|package-windows target/DEST>".into())
        }
    }
}

fn main() -> ExitCode {
    match entry() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            use io::Write;
            let _ = writeln!(io::stderr().lock(), "jecode-check: {error}");
            ExitCode::FAILURE
        }
    }
}
