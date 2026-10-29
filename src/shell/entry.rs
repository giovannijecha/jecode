// Standalone build-time shell adapter. It shares the parser and interpreter
// with the main binary, and is embedded so unit tests need no installed shell.
#[path = "../shell.rs"]
mod shell;

fn shell_executable() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|error| error.to_string())
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--internal-shell")) {
        std::process::exit(shell::run_entry(args));
    }
    std::process::exit(shell::run_entry(std::env::args_os().skip(1)));
}
