// The OS owns filesystem and network policy for the lifetime of this handle.
// No host ACL, firewall rule, or global loopback exemption is changed.
internal sealed class JecodeWindowsSandbox : System.IDisposable {
    const int SecurityEnvironmentAttribute = 0x00020023;
    const uint TerminateOnClose = 1;
    const ulong FileSystemDeny = 1;
    const ulong NetworkIngress = 8;

    [System.Runtime.InteropServices.DllImport("processmodel.dll", ExactSpelling = true)]
    static extern int IsProcessSecurityEnvironmentVersionSupported(
        uint major, out byte available, out uint minor);
    [System.Runtime.InteropServices.DllImport("processmodel.dll", ExactSpelling = true)]
    static extern int QueryProcessSecurityEnvironmentSupport(out ulong support);
    [System.Runtime.InteropServices.DllImport("processmodel.dll", ExactSpelling = true)]
    static extern int CreateProcessSecurityEnvironment(
        byte[] specification, uint size, uint flags, out System.IntPtr environment);
    [System.Runtime.InteropServices.DllImport("processmodel.dll", ExactSpelling = true)]
    static extern void CloseProcessSecurityEnvironment(System.IntPtr environment);
    [System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError = true)]
    static extern bool UpdateProcThreadAttribute(System.IntPtr list, uint flags,
        System.IntPtr attribute, System.IntPtr value, System.IntPtr size,
        System.IntPtr previous, System.IntPtr returned);
    [System.Runtime.InteropServices.DllImport("kernel32.dll", EntryPoint = "GetEnvironmentStringsW", SetLastError = true)]
    static extern System.IntPtr GetEnvironmentStrings();
    [System.Runtime.InteropServices.DllImport("kernel32.dll", EntryPoint = "FreeEnvironmentStringsW", SetLastError = true)]
    static extern bool FreeEnvironmentStrings(System.IntPtr block);

    System.IntPtr environment;
    System.IntPtr attributeValue;

    public static JecodeWindowsSandbox Prepare(string json) {
        if (System.String.IsNullOrEmpty(json)) return null;
        var policy = JecodeWindowsSandboxPolicy.Read(json);
        if (policy == null || policy.Count != 4 || !policy.ContainsKey("read") ||
            !policy.ContainsKey("write") || !policy.ContainsKey("deny") ||
            !policy.ContainsKey("network") || !(policy["network"] is bool) ||
            !(bool)policy["network"])
            throw new System.IO.InvalidDataException("Invalid Windows sandbox policy");
        var read = Paths(policy["read"]);
        var write = Paths(policy["write"]);
        var deny = Paths(policy["deny"]);
        RequireNativeSupport();
        var sandbox = new JecodeWindowsSandbox();
        try {
            // A write grant already includes read access. Keep the two native
            // lists disjoint when Rust supplied the same workspace in both.
            var writable = new System.Collections.Generic.HashSet<string>(write,
                System.StringComparer.OrdinalIgnoreCase);
            var readOnly = new System.Collections.Generic.List<string>();
            foreach (string path in read) if (!writable.Contains(path)) readOnly.Add(path);
            byte[] specification = JecodeWindowsSandboxSpec.Encode(
                readOnly.ToArray(), write, deny);
            int result = CreateProcessSecurityEnvironment(specification,
                checked((uint)specification.Length), TerminateOnClose, out sandbox.environment);
            if (result < 0 || sandbox.environment == System.IntPtr.Zero)
                throw new System.IO.IOException(
                    "CreateProcessSecurityEnvironment failed (HRESULT 0x" +
                    result.ToString("x8") + ")");
            sandbox.attributeValue = System.Runtime.InteropServices.Marshal.AllocHGlobal(System.IntPtr.Size);
            System.Runtime.InteropServices.Marshal.WriteIntPtr(sandbox.attributeValue, sandbox.environment);
            return sandbox;
        } catch {
            sandbox.Dispose();
            throw;
        }
    }

    static void RequireNativeSupport() {
        try {
            byte available;
            uint minor;
            int version = IsProcessSecurityEnvironmentVersionSupported(1, out available, out minor);
            if (version < 0 || available == 0 || minor < 1)
                throw new System.PlatformNotSupportedException(
                    "Workspace command confinement requires Windows Process Security Environment 1.1");
            ulong support;
            int query = QueryProcessSecurityEnvironmentSupport(out support);
            if (query < 0 || (support & (FileSystemDeny | NetworkIngress)) !=
                (FileSystemDeny | NetworkIngress))
                throw new System.PlatformNotSupportedException(
                    "Windows Process Security Environment lacks filesystem deny or ingress policy");
        } catch (System.DllNotFoundException error) {
            throw new System.PlatformNotSupportedException(
                "Workspace command confinement requires a compatible Windows Process Security Environment", error);
        } catch (System.EntryPointNotFoundException error) {
            throw new System.PlatformNotSupportedException(
                "Workspace command confinement requires a compatible Windows Process Security Environment", error);
        }
    }

    static string[] Paths(object value) {
        var values = value as System.Collections.IList;
        if (values == null) throw new System.IO.InvalidDataException("Invalid sandbox path list");
        var paths = new string[values.Count];
        for (int i = 0; i < values.Count; i++) {
            string path = values[i] as string;
            if (System.String.IsNullOrEmpty(path))
                throw new System.IO.InvalidDataException("Sandbox paths must be absolute");
            if (path.StartsWith(@"\\?\UNC\", System.StringComparison.OrdinalIgnoreCase))
                path = @"\\" + path.Substring(8);
            else if (path.StartsWith(@"\\?\", System.StringComparison.OrdinalIgnoreCase))
                path = path.Substring(4);
            if (!System.IO.Path.IsPathRooted(path) ||
                !System.String.Equals(System.IO.Path.GetFullPath(path), path,
                    System.StringComparison.OrdinalIgnoreCase))
                throw new System.IO.InvalidDataException("Sandbox paths must be canonical and absolute");
            paths[i] = path;
        }
        return paths;
    }

    public void AddAttribute(System.IntPtr list) {
        if (!UpdateProcThreadAttribute(list, 0,
            new System.IntPtr(SecurityEnvironmentAttribute), attributeValue,
            new System.IntPtr(System.IntPtr.Size), System.IntPtr.Zero, System.IntPtr.Zero))
            throw new System.IO.IOException("Could not set process security environment (Windows error " +
                System.Runtime.InteropServices.Marshal.GetLastWin32Error() + ")");
    }

    public static System.IntPtr InheritedEnvironment() {
        System.IntPtr block = GetEnvironmentStrings();
        if (block == System.IntPtr.Zero)
            throw new System.IO.IOException("Could not read Windows process environment (error " +
                System.Runtime.InteropServices.Marshal.GetLastWin32Error() + ")");
        return block;
    }

    public static void FreeInheritedEnvironment(System.IntPtr block) {
        if (block != System.IntPtr.Zero) FreeEnvironmentStrings(block);
    }

    public void Dispose() {
        if (attributeValue != System.IntPtr.Zero)
            System.Runtime.InteropServices.Marshal.FreeHGlobal(attributeValue);
        attributeValue = System.IntPtr.Zero;
        if (environment != System.IntPtr.Zero)
            CloseProcessSecurityEnvironment(environment);
        environment = System.IntPtr.Zero;
    }
}
