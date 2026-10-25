$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
try {
    $request = [Console]::In.ReadToEnd() | ConvertFrom-Json
    switch ($request.op) {
        'random' {
            $count = [int]$request.count
            if ($count -lt 1 -or $count -gt 4096) { throw 'Invalid random length' }
            $bytes = New-Object byte[] $count
            $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
            try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
            [Console]::Out.WriteLine((ConvertTo-Json -Compress ([Convert]::ToBase64String($bytes))))
        }
        'sha256' {
            $bytes = [Convert]::FromBase64String([string]$request.data)
            $hash = [System.Security.Cryptography.SHA256]::Create()
            try { $result = $hash.ComputeHash($bytes) } finally { $hash.Dispose() }
            [Console]::Out.WriteLine((ConvertTo-Json -Compress ([Convert]::ToBase64String($result))))
        }
        'verify' {
            $parameters = New-Object System.Security.Cryptography.RSAParameters
            $parameters.Modulus = [Convert]::FromBase64String([string]$request.n)
            $parameters.Exponent = [Convert]::FromBase64String([string]$request.e)
            $rsa = [System.Security.Cryptography.RSACryptoServiceProvider]::new()
            try {
                $rsa.PersistKeyInCsp = $false
                $rsa.ImportParameters($parameters)
                $sha = [System.Security.Cryptography.SHA256]::Create()
                try {
                    $valid = $rsa.VerifyData([Convert]::FromBase64String([string]$request.data), $sha, [Convert]::FromBase64String([string]$request.signature))
                } finally { $sha.Dispose() }
            } finally { $rsa.Dispose() }
            [Console]::Out.WriteLine(([string]$valid).ToLowerInvariant())
        }
        { $_ -eq 'protect_directory' -or $_ -eq 'protect_file' } {
            $path = [string]$request.path
            $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
            if ($request.op -eq 'protect_directory') {
                $acl = New-Object System.Security.AccessControl.DirectorySecurity
                $inheritance = [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
            } else {
                $acl = New-Object System.Security.AccessControl.FileSecurity
                $inheritance = [System.Security.AccessControl.InheritanceFlags]::None
            }
            # Protect access without requesting the unrelated WRITE_OWNER right.
            $acl.SetAccessRuleProtection($true, $false)
            $rule = New-Object System.Security.AccessControl.FileSystemAccessRule($sid, [System.Security.AccessControl.FileSystemRights]::FullControl, $inheritance, [System.Security.AccessControl.PropagationFlags]::None, [System.Security.AccessControl.AccessControlType]::Allow)
            $acl.AddAccessRule($rule)
            if ($request.op -eq 'protect_directory') {
                [System.IO.Directory]::SetAccessControl($path, $acl)
            } else {
                [System.IO.File]::SetAccessControl($path, $acl)
            }
            [Console]::Out.WriteLine('true')
        }
        default { throw 'Unknown native operation' }
    }
} catch {
    [Console]::Error.WriteLine('Native operation failed')
    exit 1
}
