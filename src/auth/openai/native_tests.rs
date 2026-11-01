use super::*;
use crate::test_support::Directory;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

fn shell(directory: &Directory, script: &str) {
    let output = crate::process::run_leaf(
        Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .env("JECODE_ACL_FIXTURE", directory.path()),
        None,
        Some(Duration::from_secs(15)),
        64 * 1024,
        &Cancellation::default(),
        None,
        &mut |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        output.exit_code,
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn account_acl_requires_no_owner_change() {
    let directory = Directory::new();
    shell(
        &directory,
        r#"
$ErrorActionPreference = 'Stop'
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
[IO.Directory]::SetAccessControl($env:JECODE_ACL_FIXTURE, $acl)
$path = Join-Path $env:JECODE_ACL_FIXTURE 'cached'
[IO.Directory]::CreateDirectory($path) | Out-Null
[IO.File]::WriteAllText((Join-Path $path 'account.json'), '{}')
"#,
    );
    let account = directory.path().join("cached");
    protect(&account, true).unwrap();
    protect(&account.join("account.json"), false).unwrap();
    shell(
        &directory,
        r#"
$ErrorActionPreference = 'Stop'
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
foreach ($item in (Get-ChildItem -LiteralPath $env:JECODE_ACL_FIXTURE -Recurse)) {
    $acl = $item.GetAccessControl()
    $rules = @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
    if (-not $acl.AreAccessRulesProtected -or $rules.Count -ne 1 -or
        $rules[0].IdentityReference -ne $sid -or $rules[0].IsInherited -or
        $rules[0].AccessControlType -ne 'Allow' -or $rules[0].FileSystemRights -ne 'FullControl') {
        throw 'Account ACL must grant only the current user full control'
    }
}
"#,
    );
}

#[test]
fn operations_share_one_helper_and_recover_after_it_exits() {
    let hex = |bytes: Vec<u8>| {
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let cancel = Cancellation::default();
    assert_eq!(hex(sha256(b"abc", &cancel).unwrap()), digest);
    let helper = server::process_id();
    assert_eq!(random(32, &cancel).unwrap().len(), 32);
    assert!(helper.is_some());
    assert_eq!(server::process_id(), helper);
    server::exit_for_test();
    assert_eq!(hex(sha256(b"abc", &cancel).unwrap()), digest);
    assert_ne!(server::process_id(), helper);
}

#[test]
fn helper_stays_alive_while_a_request_is_in_progress() {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new(crate::native::helper("openai_crypto").unwrap());
    command
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(crate::process::CREATE_NO_WINDOW);
    crate::auth::isolate_environment(&mut command);
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"sha256\n").unwrap();
    input.flush().unwrap();
    std::thread::sleep(Duration::from_secs(11));
    assert!(child.try_wait().unwrap().is_none());
    input.write_all(b"YWJj\n").unwrap();
    input.flush().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
}
