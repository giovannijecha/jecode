//! Compile owned platform adapters during the build, never at runtime.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/native");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let target = env::var("CARGO_CFG_TARGET_OS").expect("Cargo target OS");
    let mut assets = Vec::new();
    shell(&out, &mut assets, &target);
    if target == "windows" {
        windows(&out, &mut assets);
    } else if target == "linux" || target == "macos" {
        unix(&out, &mut assets, &target);
    } else {
        panic!("Jecode supports Windows, Linux and macOS");
    }
    let mut table =
        String::from("fn asset(name: &str) -> Option<&'static [u8]> {\n    match name {\n");
    for (name, path) in assets {
        table.push_str(&format!(
            "        {name:?} => Some(include_bytes!({:?})),\n",
            path.to_string_lossy()
        ));
    }
    table.push_str("        _ => None,\n    }\n}\n");
    fs::write(out.join("native_assets.rs"), table).expect("Write embedded native assets");
}

fn shell(out: &Path, assets: &mut Vec<(String, PathBuf)>, target: &str) {
    println!("cargo:rerun-if-changed=src/shell.rs");
    println!("cargo:rerun-if-changed=src/shell");
    let binary = out.join(if target == "windows" {
        "jecode-shell.exe"
    } else {
        "jecode-shell"
    });
    let compiler = env::var_os("RUSTC").expect("Cargo RUSTC");
    let triple = env::var("TARGET").expect("Cargo TARGET");
    let mut command = Command::new(&compiler);
    command.args([
        "--edition=2024",
        "--crate-name=jecode_shell",
        "-Dunsafe_code",
        "-Copt-level=2",
    ]);
    command.arg("--target").arg(&triple);
    if let Some(linker) = env::var_os("RUSTC_LINKER") {
        command
            .arg("-C")
            .arg(format!("linker={}", linker.to_string_lossy()));
    } else if triple.ends_with("windows-gnullvm") {
        let output = Command::new(&compiler)
            .args(["--print", "sysroot"])
            .output()
            .expect("Rust sysroot");
        assert!(output.status.success(), "Could not locate the Rust linker");
        let sysroot = String::from_utf8(output.stdout).expect("UTF-8 sysroot");
        let linker =
            PathBuf::from(sysroot.trim()).join(format!("lib/rustlib/{triple}/bin/rust-lld.exe"));
        command
            .arg("-C")
            .arg(format!("linker={}", linker.display()));
    }
    if target == "windows" {
        command.arg("-Ctarget-feature=+crt-static");
    }
    command.arg("src/shell/entry.rs").arg("-o").arg(&binary);
    checked(&mut command, "shell");
    assets.push(("shell".into(), binary));
}

fn windows(out: &Path, assets: &mut Vec<(String, PathBuf)>) {
    let root = env::var_os("SystemRoot").expect("Windows builds need SystemRoot");
    let framework = PathBuf::from(root).join("Microsoft.NET/Framework64/v4.0.30319");
    let compiler = framework.join("csc.exe");
    let mut compile = |name: &str, sources: &[&str], references: &[&str]| {
        let binary = out.join(format!("jecode-{name}.exe"));
        let mut command = Command::new(&compiler);
        command.args(["/nologo", "/optimize+", "/target:exe", "/platform:anycpu"]);
        command.arg(format!("/out:{}", binary.display()));
        for reference in references {
            command.arg(format!("/reference:{reference}"));
        }
        for source in sources {
            println!("cargo:rerun-if-changed={source}");
            let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join(source);
            command.arg(source.to_string_lossy().replace('/', "\\"));
        }
        checked(&mut command, name);
        assets.push((name.into(), binary));
    };
    compile(
        "supervisor",
        &[
            "src/process/windows.cs",
            "src/process/windows_command.cs",
            "src/process/windows_sandbox.cs",
            "src/process/windows_sandbox_spec.cs",
        ],
        &["System.Web.Extensions.dll"],
    );
    compile(
        "pty",
        &["src/pty/windows.cs", "src/process/windows_command.cs"],
        &[],
    );
    compile(
        "terminal",
        &["src/tui/terminal.cs", "src/native/terminal_entry.cs"],
        &[],
    );
    compile("checkpoint_space", &["src/checkpoints/space.cs"], &[]);
    compile("openai_crypto", &["src/auth/openai/native.cs"], &[]);
    compile("http", &["src/native/winhttp.cs"], &[]);
}

fn unix(out: &Path, assets: &mut Vec<(String, PathBuf)>, target: &str) {
    for (name, source) in [
        ("pty_unix", "src/pty/unix.c"),
        ("process_unix", "src/native/process.c"),
        ("terminal_unix", "src/native/terminal.c"),
    ] {
        println!("cargo:rerun-if-changed={source}");
        let binary = out.join(format!("jecode-{name}"));
        let mut command = c_compiler();
        command.arg("-std=c11");
        command.arg(source).arg("-o").arg(&binary);
        if target == "macos" {
            command.arg("-D_DARWIN_C_SOURCE");
        }
        checked(&mut command, name);
        assets.push((name.into(), binary));
    }
    println!("cargo:rerun-if-changed=src/pty/unix_session.h");
    let crypto = format!("src/native/{target}_crypto.c");
    {
        println!("cargo:rerun-if-changed={crypto}");
        let binary = out.join("jecode-openai_crypto");
        let mut command = c_compiler();
        command.arg("-std=c11");
        command.arg(&crypto).arg("-o").arg(&binary);
        if target == "linux" {
            command.arg("-ldl");
        } else {
            command.args(["-framework", "Security", "-framework", "CoreFoundation"]);
        }
        checked(&mut command, "openai_crypto");
        assets.push(("openai_crypto".into(), binary));
    }
    let http = if target == "linux" {
        "src/native/linux_http.c"
    } else {
        "src/native/macos_http.m"
    };
    {
        println!("cargo:rerun-if-changed={http}");
        let binary = out.join("jecode-http");
        let mut command = c_compiler();
        if target == "linux" {
            command.arg("-std=c11");
        } else {
            command.arg("-fobjc-arc");
        }
        command.arg(http).arg("-o").arg(&binary);
        if target == "linux" {
            command.arg("-ldl");
        } else {
            command.args(["-framework", "Foundation", "-framework", "CFNetwork"]);
        }
        checked(&mut command, "http");
        assets.push(("http".into(), binary));
    }
    if target == "linux" {
        let binary = out.join("jecode-clipboard_linux");
        let mut command = c_compiler();
        command.arg("-std=c11");
        for source in [
            "src/native/linux_clipboard.c",
            "src/native/linux_clipboard_x11.c",
            "src/native/linux_clipboard_wayland.c",
            "src/native/linux_image.c",
        ] {
            println!("cargo:rerun-if-changed={source}");
            command.arg(source);
        }
        command.arg("-o").arg(&binary).arg("-ldl");
        checked(&mut command, "clipboard_linux");
        assets.push(("clipboard_linux".into(), binary));
    }
}

/// Selects the C compiler for the Cargo target. Cross builds use the usual
/// `CC_<target>`/`TARGET_CC`/`CC` variables; between macOS architectures the
/// system `cc` receives `-arch`. Development builds deny warnings; release
/// builds do not, so a newer system compiler cannot break user builds.
fn c_compiler() -> Command {
    let triple = env::var("TARGET").expect("Cargo TARGET");
    let host = env::var("HOST").expect("Cargo HOST");
    let names = [
        format!("CC_{triple}"),
        format!("CC_{}", triple.replace('-', "_")),
        "TARGET_CC".into(),
        "CC".into(),
    ];
    for name in &names {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let configured = names.iter().find_map(env::var_os);
    let mut command = if let Some(compiler) = configured {
        Command::new(compiler)
    } else if triple == host {
        Command::new("cc")
    } else if triple.ends_with("-apple-darwin") && host.ends_with("-apple-darwin") {
        let mut command = Command::new("cc");
        let architecture = match triple.split('-').next().unwrap_or_default() {
            "aarch64" => "arm64",
            other => other,
        };
        command.args(["-arch", architecture]);
        command
    } else {
        panic!(
            "Cross-compiling Jecode's C adapters for {triple} on {host} needs a target C \
             compiler: set CC_{} or TARGET_CC",
            triple.replace('-', "_")
        );
    };
    command.args(["-O2", "-Wall", "-Wextra"]);
    if env::var("PROFILE").as_deref() == Ok("debug") {
        command.arg("-Werror");
    }
    command
}

fn checked(command: &mut Command, name: &str) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("Could not compile owned {name} adapter: {error}"));
    assert!(
        output.status.success(),
        "Could not compile owned {name} adapter:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
