// Owned Windows boundary for OpenAI crypto and private ACLs. Input is a small
// newline-delimited protocol on stdin; every binary or path field is base64.
using System;
using System.IO;
using System.Text;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;

public static class JecodeOpenAiNative {
    public static int Main() {
        try {
            string[] lines = Console.In.ReadToEnd().TrimEnd('\n', '\r').Split('\n');
            string op = lines[0].TrimEnd('\r');
            if (op == "random") {
                int count = int.Parse(lines[1]);
                if (count < 1 || count > 4096) throw new InvalidDataException();
                byte[] bytes = new byte[count];
                using (var rng = RandomNumberGenerator.Create()) rng.GetBytes(bytes);
                Encoded(bytes);
            } else if (op == "sha256") {
                byte[] data = Convert.FromBase64String(lines[1]);
                using (var hash = SHA256.Create()) Encoded(hash.ComputeHash(data));
            } else if (op == "verify") {
                byte[] data = Convert.FromBase64String(lines[1]);
                byte[] signature = Convert.FromBase64String(lines[2]);
                byte[] modulus = Convert.FromBase64String(lines[3]);
                byte[] exponent = Convert.FromBase64String(lines[4]);
                using (var rsa = new RSACryptoServiceProvider()) {
                    rsa.PersistKeyInCsp = false;
                    rsa.ImportParameters(new RSAParameters { Modulus = modulus, Exponent = exponent });
                    using (var hash = SHA256.Create()) {
                        Console.WriteLine(rsa.VerifyData(data, hash, signature) ? "true" : "false");
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
                acl.SetOwner(sid);
                acl.SetAccessRuleProtection(true, false);
                acl.AddAccessRule(new FileSystemAccessRule(sid, FileSystemRights.FullControl, inheritance, PropagationFlags.None, AccessControlType.Allow));
                if (op == "protect_directory") Directory.SetAccessControl(path, (DirectorySecurity)acl);
                else File.SetAccessControl(path, (FileSecurity)acl);
                Console.WriteLine("true");
            } else throw new InvalidDataException();
            return 0;
        } catch {
            Console.Error.WriteLine("Native OpenAI operation failed");
            return 1;
        }
    }

    private static void Encoded(byte[] bytes) {
        Console.WriteLine("\"" + Convert.ToBase64String(bytes) + "\"");
    }
}
