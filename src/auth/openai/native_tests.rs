use super::*;
use crate::{json::Value, test_support::Directory};

fn shell(directory: &Directory, script: &str) {
    run(
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
        &Cancellation::default(),
    )
    .unwrap();
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
foreach ($name in @('cached', 'fallback')) {
    $path = Join-Path $env:JECODE_ACL_FIXTURE $name
    [IO.Directory]::CreateDirectory($path) | Out-Null
    [IO.File]::WriteAllText((Join-Path $path 'account.json'), '{}')
}
"#,
    );
    for name in ["cached", "fallback"] {
        for (path, is_directory) in [
            (directory.path().join(name), true),
            (directory.path().join(name).join("account.json"), false),
        ] {
            if name == "cached" {
                protect(&path, is_directory).unwrap();
            } else {
                let request = Value::object([
                    (
                        "op",
                        Value::string(if is_directory {
                            "protect_directory"
                        } else {
                            "protect_file"
                        }),
                    ),
                    ("path", Value::string(path.to_string_lossy())),
                ]);
                let output = run(
                    Command::new("powershell.exe").args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        include_str!("native.ps1"),
                    ]),
                    Some(request.encode().into_bytes()),
                    &Cancellation::default(),
                )
                .unwrap();
                assert_eq!(String::from_utf8(output).unwrap().trim(), "true");
            }
        }
    }
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
