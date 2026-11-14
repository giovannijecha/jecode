//! Windows launch handshake: a native helper checks its own and the suspended
//! application's Job Object membership before any application code can run.

use super::{Cancellation, Command, Duration, Instant, Value, report};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc;

const BREAKAWAY: u32 = 0x0100_0000;
const NO_WINDOW: u32 = 0x0800_0000;
const HANDSHAKE: Duration = Duration::from_secs(5);

enum Event {
    Refused(String),
    Launched(u32),
    Uncertain(u32, String),
    Exited(i32),
    StillRunning,
    ObservationError(String),
}

pub(super) fn launch(
    command: &Command,
    observation: Duration,
    cancellation: &Cancellation,
    started: Instant,
) -> Result<Value, String> {
    let name = command.get_program().to_string_lossy();
    let helper = crate::native::helper("launch").map_err(|error| {
        format!("Could not prepare the Windows launch adapter: {error}; nothing was launched")
    })?;
    let mut adapter = Command::new(helper);
    adapter
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(BREAKAWAY | NO_WINDOW);
    if let Some(directory) = command.get_current_dir() {
        adapter.current_dir(crate::process::ordinary_path(directory));
    }
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => adapter.env(key, value),
            None => adapter.env_remove(key),
        };
    }
    let mut child = adapter.spawn().map_err(|error| {
        format!(
            "Could not launch {name} independently of Jecode's host: {error}; Nothing was launched"
        )
    })?;
    let (sender, receiver) = mpsc::channel();
    let output = child.stdout.take().unwrap();
    std::thread::spawn(move || receive(output, sender));
    let data = encode(command, observation);
    // A helper that correctly refuses before reading the command can close
    // stdin immediately. Its reported refusal still decides this result.
    let write = child.stdin.take().unwrap().write_all(&data);
    let first = receiver.recv_timeout(HANDSHAKE);
    let pid = match first {
        Ok(Event::Refused(reason)) => {
            let _ = child.wait();
            return Err(format!(
                "Could not launch {name} independently of Jecode's host: {reason}; Nothing was launched"
            ));
        }
        Ok(Event::Launched(pid)) => pid,
        Ok(Event::Uncertain(pid, reason)) => {
            let _ = child.wait();
            return Err(format!(
                "Launch outcome for {name} is unknown (PID {pid}): {reason}"
            ));
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            let detail = write
                .err()
                .map_or_else(String::new, |error| format!("; input: {error}"));
            return Err(format!(
                "Launch outcome for {name} is unknown: no native confirmation{detail}"
            ));
        }
    };
    let mut cancelled = false;
    let outcome = loop {
        if cancellation.requested() {
            cancelled = true;
            break None;
        }
        let remaining = observation.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break None;
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(20))) {
            Ok(Event::Exited(code)) => break Some(Some(code)),
            Ok(Event::StillRunning) => break None,
            Ok(Event::ObservationError(error)) => {
                let _ = child.wait();
                return Ok(unknown(pid, started.elapsed(), error));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = child.wait();
                return Ok(unknown(
                    pid,
                    started.elapsed(),
                    "The launch adapter ended before reporting the application's state".into(),
                ));
            }
            _ => {}
        }
    };
    if outcome.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    Ok(report(
        pid,
        outcome,
        started.elapsed(),
        cancelled,
        Vec::new(),
    ))
}

fn unknown(pid: u32, elapsed: Duration, error: String) -> Value {
    Value::object([
        ("launch", Value::string("persistent")),
        ("pid", Value::number(pid)),
        ("application", Value::string("unknown")),
        ("observed_ms", Value::number(elapsed.as_millis())),
        (
            "error",
            Value::string(format!(
                "The application launched, but its state could not be observed: {error}"
            )),
        ),
    ])
}

fn encode(command: &Command, observation: Duration) -> Vec<u8> {
    let mut data = Vec::new();
    write_string(&mut data, command.get_program());
    data.extend_from_slice(&(command.get_args().len() as u32).to_le_bytes());
    for argument in command.get_args() {
        write_string(&mut data, argument);
    }
    data.extend_from_slice(&(observation.as_millis() as i32).to_le_bytes());
    data
}

fn write_string(data: &mut Vec<u8>, value: &OsStr) {
    let units: Vec<_> = value.encode_wide().collect();
    data.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        data.extend_from_slice(&unit.to_le_bytes());
    }
}

fn receive(mut output: impl Read, sender: mpsc::Sender<Event>) {
    let first = match byte(&mut output) {
        Ok(0) => text(&mut output).map(Event::Refused),
        Ok(1) => number(&mut output).map(Event::Launched),
        Ok(5) => number(&mut output)
            .and_then(|pid| text(&mut output).map(|detail| Event::Uncertain(pid, detail))),
        _ => Err("The launch adapter sent no valid handshake".into()),
    };
    let launched = matches!(&first, Ok(Event::Launched(_)));
    if sender
        .send(first.unwrap_or_else(Event::ObservationError))
        .is_err()
        || !launched
    {
        return;
    }
    let last = match byte(&mut output) {
        Ok(2) => number(&mut output).map(|code| Event::Exited(code as i32)),
        Ok(3) => Ok(Event::StillRunning),
        Ok(4) => text(&mut output).map(Event::ObservationError),
        _ => Err("The launch adapter sent no observation result".into()),
    };
    let _ = sender.send(last.unwrap_or_else(Event::ObservationError));
}

fn byte(input: &mut impl Read) -> Result<u8, String> {
    let mut data = [0; 1];
    input
        .read_exact(&mut data)
        .map_err(|error| error.to_string())?;
    Ok(data[0])
}

fn number(input: &mut impl Read) -> Result<u32, String> {
    let mut data = [0; 4];
    input
        .read_exact(&mut data)
        .map_err(|error| error.to_string())?;
    Ok(u32::from_le_bytes(data))
}

fn text(input: &mut impl Read) -> Result<String, String> {
    let size = number(input)? as usize;
    if size > 1_000_000 {
        return Err("Invalid launch adapter message length".into());
    }
    let mut data = vec![0; size];
    input
        .read_exact(&mut data)
        .map_err(|error| error.to_string())?;
    String::from_utf8(data).map_err(|error| error.to_string())
}
