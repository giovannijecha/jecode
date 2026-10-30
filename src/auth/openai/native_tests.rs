use super::*;
use crate::test_support::Directory;

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
