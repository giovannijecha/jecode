// Shared by the supervised process and ConPTY child launchers.
public static class JecodeWindowsCommand {
    public static string ResolveProgram(string program) {
        // An explicit path is the caller's choice, including a relative path.
        if (program.IndexOfAny(new char[] { '\\', '/' }) >= 0 || program.IndexOf(':') >= 0)
            return program;
        string path = System.Environment.GetEnvironmentVariable("PATH") ?? "";
        string file = System.IO.Path.HasExtension(program) ? program : program + ".exe";
        foreach (string entry in path.Split(';')) {
            string directory = entry.Trim().Trim('"');
            if (directory.Length == 0) continue;
            string candidate = System.IO.Path.Combine(directory, file);
            if (System.IO.File.Exists(candidate)) return System.IO.Path.GetFullPath(candidate);
        }
        throw new System.IO.FileNotFoundException("Executable was not found on PATH", program);
    }

    public static string Quote(string value) {
        var result = new System.Text.StringBuilder("\"");
        int slashes = 0;
        foreach (char c in value) {
            if (c == '\\') { slashes++; continue; }
            if (c == '"') {
                result.Append('\\', slashes * 2 + 1).Append('"');
                slashes = 0;
                continue;
            }
            result.Append('\\', slashes);
            slashes = 0;
            result.Append(c);
        }
        result.Append('\\', slashes * 2);
        return result.Append('"').ToString();
    }
}
