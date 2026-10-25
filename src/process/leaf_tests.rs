use super::*;
use std::time::{Duration, Instant};

#[test]
fn leaf_commands_report_exit_and_stop_on_cancellation() {
    let mut version = Command::new("curl.exe");
    version.arg("--version");
    let output = run_leaf(
        &mut version,
        Some(b"ignored".to_vec()),
        None,
        4096,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("curl "));

    let cancellation = Cancellation::default();
    let signal = cancellation.clone();
    let started = Instant::now();
    let mut ping = Command::new("ping.exe");
    ping.args(["-n", "30", "127.0.0.1"]);
    let output = run_leaf(
        &mut ping,
        None,
        None,
        4096,
        &cancellation,
        None,
        &mut |_| {
            signal.cancel();
            Ok(())
        },
    )
    .unwrap();
    assert!(output.cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}
