use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => println!("jecode npm fixture {}", env!("JECODE_NPM_FIXTURE_VERSION")),
        Some("--echo") => {
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input).unwrap();
            print!("{input}");
        }
        Some("--args") => {
            for arg in &args[1..] {
                std::io::stdout().write_all(arg.as_bytes()).unwrap();
                std::io::stdout().write_all(&[0]).unwrap();
            }
        }
        Some("--exit") => return ExitCode::from(args[1].parse::<u8>().unwrap()),
        _ => return ExitCode::from(2),
    }
    ExitCode::SUCCESS
}
