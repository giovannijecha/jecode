// Owned Windows boundary for OpenAI crypto and private ACLs. Input is a small
// newline-delimited protocol on stdin; every binary or path field is base64.
// With "serve", requests repeat until stdin closes or the helper is idle, and
// each answer is one line ("!" for a failed operation).
using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;

public static class JecodeOpenAiNative {
    const int IdleMilliseconds = 10000;

    public static int Main(string[] args) {
        if (args.Length == 1 && args[0] == "serve") return Serve();
        try {
            string[] lines = Console.In.ReadToEnd().TrimEnd('\n', '\r').Split('\n');
            for (int index = 0; index < lines.Length; index++) lines[index] = lines[index].TrimEnd('\r');
            Console.WriteLine(Execute(lines));
            return 0;
        } catch {
            Console.Error.WriteLine("Native OpenAI operation failed");
            return 1;
        }
    }

    static int Fields(string op) {
        switch (op) {
            case "random": case "sha256": case "protect_directory": case "protect_file": return 1;
            case "verify": return 4;
            default: return -1;
        }
    }

    static int Serve() {
        // Exit when idle; the caller resends a request that found it gone.
        object idleGate = new object();
        int last = Environment.TickCount;
        bool active = false;
        var idle = new Timer(state => {
            lock (idleGate) {
                if (!active && unchecked(Environment.TickCount - last) > IdleMilliseconds) Environment.Exit(0);
            }
        }, null, 1000, 1000);
        try {
            while (true) {
                string op = Console.In.ReadLine();
                if (op == null) return 0;
                lock (idleGate) active = true;
                int count = Fields(op);
                if (count < 0) return 1;
                string[] lines = new string[count + 1];
                lines[0] = op;
                for (int index = 1; index <= count; index++) {
                    lines[index] = Console.In.ReadLine();
                    if (lines[index] == null) return 1;
                }
                string answer;
                try { answer = Execute(lines); } catch { answer = "!"; }
                Console.Out.WriteLine(answer);
                Console.Out.Flush();
                lock (idleGate) {
                    last = Environment.TickCount;
                    active = false;
                }
            }
        } finally {
            idle.Dispose();
        }
    }

    static string Execute(string[] lines) {
        string op = lines[0];
        if (op == "random") {
            int count = int.Parse(lines[1]);
            if (count < 1 || count > 4096) throw new InvalidDataException();
            byte[] bytes = new byte[count];
            using (var rng = RandomNumberGenerator.Create()) rng.GetBytes(bytes);
            return Encoded(bytes);
        } else if (op == "sha256") {
            byte[] data = Convert.FromBase64String(lines[1]);
            using (var hash = SHA256.Create()) return Encoded(hash.ComputeHash(data));
        } else if (op == "verify") {
            byte[] data = Convert.FromBase64String(lines[1]);
            byte[] signature = Convert.FromBase64String(lines[2]);
            byte[] modulus = Convert.FromBase64String(lines[3]);
            byte[] exponent = Convert.FromBase64String(lines[4]);
            using (var rsa = new RSACryptoServiceProvider()) {
                rsa.PersistKeyInCsp = false;
                rsa.ImportParameters(new RSAParameters { Modulus = modulus, Exponent = exponent });
                using (var hash = SHA256.Create()) {
                    return rsa.VerifyData(data, hash, signature) ? "true" : "false";
                }
            }
        } else if (op == "protect_directory" || op == "protect_file") {
            string path = Encoding.UTF8.GetString(Convert.FromBase64String(lines[1]));
            SecurityIdentifier sid = WindowsIdentity.GetCurrent().User;
            FileSystemSecurity acl;
            InheritanceFlags inheritance;
            if (op == "protect_directory") {
                acl = new DirectorySecurity();
                inheritance = InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit;
            } else {
                acl = new FileSecurity();
                inheritance = InheritanceFlags.None;
            }
            // Change only the DACL: a creator can protect its own object
            // without the additional WRITE_OWNER right.
            acl.SetAccessRuleProtection(true, false);
            acl.AddAccessRule(new FileSystemAccessRule(sid, FileSystemRights.FullControl, inheritance, PropagationFlags.None, AccessControlType.Allow));
            if (op == "protect_directory") Directory.SetAccessControl(path, (DirectorySecurity)acl);
            else File.SetAccessControl(path, (FileSecurity)acl);
            return "true";
        }
        throw new InvalidDataException();
    }

    private static string Encoded(byte[] bytes) {
        return "\"" + Convert.ToBase64String(bytes) + "\"";
    }
}
