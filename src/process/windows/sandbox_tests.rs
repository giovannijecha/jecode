use crate::cancel::Cancellation;
use crate::json::Value;
use crate::permissions::Level;
use crate::redact::Redactor;
use crate::test_support::{Directory, sandbox as probe};
use crate::tools::{Begun, Tools};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn workspace(tools: &Tools, executable: &Path, args: &[&str]) -> Value {
    let arguments = Value::object([
        ("executable", Value::string(executable.to_string_lossy())),
        (
            "args",
            Value::Array(args.iter().map(|arg| Value::string(*arg)).collect()),
        ),
        ("timeout_ms", Value::number(5000)),
    ]);
    tools
        .execute_begun(
            Begun::default().with_permission(Level::Workspace, None),
            "exec",
            &arguments.encode(),
            &Cancellation::default(),
        )
        .result
}

fn text<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

#[test]
fn workspace_rust_child_and_grandchild_enforce_file_policy() {
    let root = Directory::new();
    let outside = Directory::new();
    let allowed = root.path().join("allowed.txt");
    let private = outside.path().join("private.txt");
    let escaped = outside.path().join("escaped.txt");
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&private, "private").unwrap();
    let fixture = probe::executable(root.path());
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(root.path().join("logs"), Redactor::empty());
    let arguments = Value::object([
        ("executable", Value::string(fixture.to_string_lossy())),
        ("args", probe::args()),
        (
            "env",
            Value::object([
                (probe::ROLE, Value::string("parent")),
                (probe::ALLOWED, Value::string(allowed.to_string_lossy())),
                (probe::PRIVATE, Value::string(private.to_string_lossy())),
                (probe::ESCAPED, Value::string(escaped.to_string_lossy())),
            ]),
        ),
        ("timeout_ms", Value::number(5000)),
    ]);
    let result = tools
        .execute_begun(
            Begun::default().with_permission(Level::Workspace, None),
            "exec",
            &arguments.encode(),
            &Cancellation::default(),
        )
        .result;
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    let output = text(&result, "stdout");
    for expected in [
        "INSIDE:allowed",
        "OUTSIDE_ERROR:PermissionDenied",
        "WRITE_ERROR:PermissionDenied",
        "CREATED:created",
    ] {
        assert!(output.contains(expected), "{result:?}");
    }
    assert_eq!(std::fs::read_to_string(&private).unwrap(), "private");
    assert!(!escaped.exists());
    assert!(!root.path().join("nested").exists());
}

#[test]
#[ignore = "PSEC cannot initialize .NET workloads in a Windows service session; run interactively"]
fn workspace_native_child_and_grandchild_enforce_file_policy() {
    let root = Directory::new();
    let outside = Directory::new();
    let allowed = root.path().join("allowed.txt");
    let private = outside.path().join("private.txt");
    let escaped = outside.path().join("escaped.txt");
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&private, "private").unwrap();
    let source = root.path().join("sandbox_probe.cs");
    let fixture = root.path().join("sandbox_probe.exe");
    std::fs::write(
        &source,
        r#"using System;
using System.Diagnostics;
using System.IO;
class Probe {
    static int Main(string[] args) {
        if (args.Length > 0 && args[0] == "child") {
            try { Console.WriteLine("INSIDE:" + File.ReadAllText(args[1])); }
            catch (Exception error) { Console.WriteLine("INSIDE_ERROR:" + error.GetType().Name); }
            try { Console.WriteLine("OUTSIDE:" + File.ReadAllText(args[2])); }
            catch (Exception error) { Console.WriteLine("OUTSIDE_ERROR:" + error.GetType().Name); }
            try { File.WriteAllText(args[3], "escaped"); Console.WriteLine("ESCAPED"); }
            catch (Exception error) { Console.WriteLine("WRITE_ERROR:" + error.GetType().Name); }
            string nested = Path.Combine(Path.GetDirectoryName(args[1]), "nested");
            Directory.CreateDirectory(nested);
            string file = Path.Combine(nested, "item.txt");
            File.WriteAllText(file, "created");
            Console.WriteLine("CREATED:" + File.ReadAllText(file));
            File.Delete(file);
            Directory.Delete(nested);
            return 0;
        }
        var start = new ProcessStartInfo();
        start.FileName = Process.GetCurrentProcess().MainModule.FileName;
        start.Arguments = "child \"" + args[0] + "\" \"" + args[1] + "\" \"" + args[2] + "\"";
        start.UseShellExecute = false;
        start.RedirectStandardOutput = true;
        start.CreateNoWindow = true;
        using (var child = Process.Start(start)) {
            Console.Write(child.StandardOutput.ReadToEnd());
            child.WaitForExit();
            return child.ExitCode;
        }
    }
}"#,
    )
    .unwrap();
    let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Add-Type -LiteralPath '{}' -OutputAssembly '{}' -OutputType ConsoleApplication",
        quote(&source),
        quote(&fixture)
    );
    assert!(
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .unwrap()
            .success()
    );
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(root.path().join("logs"), Redactor::empty());
    let result = workspace(
        &tools,
        &fixture,
        &[
            allowed.to_str().unwrap(),
            private.to_str().unwrap(),
            escaped.to_str().unwrap(),
        ],
    );
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    let output = text(&result, "stdout");
    assert!(output.contains("INSIDE:allowed"), "{result:?}");
    assert!(
        output.contains("OUTSIDE_ERROR:UnauthorizedAccessException"),
        "{result:?}"
    );
    assert!(
        output.contains("WRITE_ERROR:UnauthorizedAccessException"),
        "{result:?}"
    );
    assert!(output.contains("CREATED:created"), "{result:?}");
    assert!(!escaped.exists());
    assert!(!root.path().join("nested").exists());
}

#[test]
fn workspace_native_rust_probe_receives_selected_environment() {
    let root = Directory::new();
    let fixture = probe::environment_executable(root.path());
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(root.path().join("logs"), Redactor::empty());
    let arguments = Value::object([
        ("executable", Value::string(fixture.to_string_lossy())),
        ("args", probe::environment_args()),
        (
            "env",
            Value::object([("JECODE_TEST_ENV", Value::string("sentinel"))]),
        ),
        ("timeout_ms", Value::number(5000)),
    ]);
    let result = tools
        .execute_begun(
            Begun::default().with_permission(Level::Workspace, None),
            "exec",
            &arguments.encode(),
            &Cancellation::default(),
        )
        .result;
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    assert!(
        text(&result, "stdout").contains("WORKSPACE_NATIVE:sentinel:"),
        "{result:?}"
    );
}

#[test]
#[ignore = "PSEC cannot initialize PowerShell in a Windows service session; run interactively"]
fn workspace_native_powershell_runs_without_user_profile() {
    let root = Directory::new();
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(root.path().join("logs"), Redactor::empty());
    let arguments = Value::object([
        ("executable", Value::string("powershell.exe")),
        (
            "args",
            Value::Array(
                [
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "[Console]::WriteLine('WORKSPACE_POWERSHELL:' + $env:JECODE_TEST_ENV + ':' + $env:JECODE_SECRET_CANARY)",
                ]
                .map(Value::string)
                .to_vec(),
            ),
        ),
        (
            "env",
            Value::object([("JECODE_TEST_ENV", Value::string("sentinel"))]),
        ),
        ("timeout_ms", Value::number(5000)),
    ]);
    let result = tools
        .execute_begun(
            Begun::default().with_permission(Level::Workspace, None),
            "exec",
            &arguments.encode(),
            &Cancellation::default(),
        )
        .result;
    assert_eq!(
        result.get("exit_code"),
        Some(&Value::number(0)),
        "{result:?}"
    );
    assert!(
        text(&result, "stdout").trim() == "WORKSPACE_POWERSHELL:sentinel:",
        "{result:?}"
    );
}

#[test]
#[ignore = "host-to-sandbox loopback is unavailable on this Windows build; local servers run in Full"]
fn workspace_native_loopback_client_and_server() {
    use std::io::{BufRead, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Stdio;

    let root = Directory::new();
    let mut tools = Tools::new(root.path()).unwrap();
    tools.configure_output(root.path().join("logs"), Redactor::empty());
    let host = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = host.local_addr().unwrap().port();
    host.set_nonblocking(true).unwrap();
    let received = std::thread::spawn(move || {
        let started = Instant::now();
        loop {
            match host.accept() {
                Ok((mut peer, _)) => {
                    let mut buffer = [0u8; 1024];
                    let _ = peer.read(&mut buffer);
                    peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                        .unwrap();
                    return true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("host listener failed: {error}"),
            }
            if started.elapsed() > Duration::from_secs(6) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let client = workspace(
        &tools,
        Path::new("curl.exe"),
        &[
            "--max-time",
            "3",
            "--silent",
            "--show-error",
            &format!("http://127.0.0.1:{port}/"),
        ],
    );
    let client_connected = received.join().unwrap();

    let source = root.path().join("loopback_server.cs");
    let fixture = root.path().join("loopback_server.exe");
    std::fs::write(
        &source,
        r#"using System;
using System.Net;
using System.Net.Sockets;
class Server {
    static int Main(string[] args) {
        var listener = new TcpListener(IPAddress.Loopback, int.Parse(args[0]));
        listener.Start();
        Console.WriteLine("READY");
        Console.Out.Flush();
        var pending = listener.BeginAcceptTcpClient(null, null);
        if (!pending.AsyncWaitHandle.WaitOne(4000)) { listener.Stop(); return 2; }
        using (var client = listener.EndAcceptTcpClient(pending)) {
            var bytes = System.Text.Encoding.ASCII.GetBytes("HELLO");
            client.GetStream().Write(bytes, 0, bytes.Length);
        }
        listener.Stop();
        return 0;
    }
}"#,
    )
    .unwrap();
    let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Add-Type -LiteralPath '{}' -OutputAssembly '{}' -OutputType ConsoleApplication",
        quote(&source),
        quote(&fixture)
    );
    assert!(
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()
            .unwrap()
            .success()
    );
    let baseline_port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut baseline = Command::new(&fixture)
        .arg(baseline_port.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut baseline_ready = String::new();
    std::io::BufReader::new(baseline.stdout.take().unwrap())
        .read_line(&mut baseline_ready)
        .unwrap();
    assert_eq!(baseline_ready.trim(), "READY");
    let mut baseline_peer = TcpStream::connect(("127.0.0.1", baseline_port)).unwrap();
    let mut baseline_bytes = [0u8; 5];
    baseline_peer.read_exact(&mut baseline_bytes).unwrap();
    assert_eq!(&baseline_bytes, b"HELLO");
    assert!(baseline.wait().unwrap().success());
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let arguments = Value::object([
        ("executable", Value::string(fixture.to_string_lossy())),
        ("args", Value::Array(vec![Value::string(port.to_string())])),
        ("wait_ms", Value::number(0)),
        ("timeout_ms", Value::number(5000)),
    ]);
    let started = tools
        .execute_begun(
            Begun::default().with_permission(Level::Workspace, None),
            "exec",
            &arguments.encode(),
            &Cancellation::default(),
        )
        .result;
    let id = text(&started, "process_id").to_owned();
    assert!(!id.is_empty(), "{started:?}");
    let mut output = text(&started, "stdout").to_owned();
    let deadline = Instant::now() + Duration::from_secs(6);
    while !output.contains("READY") && Instant::now() < deadline {
        let polled = tools.execute(
            "process",
            &Value::object([
                ("action", Value::string("poll")),
                ("process_id", Value::string(&id)),
                ("wait_ms", Value::number(200)),
            ])
            .encode(),
        );
        output.push_str(text(&polled, "stdout"));
        if text(&polled, "status") != "running" {
            break;
        }
    }
    let server_ready = output.contains("READY");
    let mut server_connected = false;
    let mut server_error = String::new();
    if server_ready {
        match TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_secs(2),
        ) {
            Ok(mut peer) => {
                peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut bytes = [0u8; 5];
                match peer.read_exact(&mut bytes) {
                    Ok(()) => server_connected = &bytes == b"HELLO",
                    Err(error) => server_error = format!("read: {error}"),
                }
            }
            Err(error) => server_error = format!("connect: {error}"),
        }
    }
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        let polled = tools.execute(
            "process",
            &Value::object([
                ("action", Value::string("poll")),
                ("process_id", Value::string(&id)),
                ("wait_ms", Value::number(200)),
            ])
            .encode(),
        );
        if text(&polled, "status") != "running" {
            break;
        }
    }
    assert!(
        client.get("exit_code") == Some(&Value::number(0))
            && text(&client, "stdout") == "OK"
            && client_connected
            && server_ready
            && server_connected,
        "client={client:?}; host_received={client_connected}; server_ready={server_ready}; server_output={output:?}; host_connected={server_connected}; server_error={server_error}; baseline={baseline_ready:?}"
    );
}
