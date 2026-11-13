use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().skip(1).next().as_deref() == Some("--version") {
        println!(
            "jecode {} (native-lifecycle-fixture)",
            env!("JECODE_NATIVE_FIXTURE_VERSION")
        );
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}
