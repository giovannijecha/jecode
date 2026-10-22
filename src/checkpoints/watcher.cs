using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading;

// Reports workspace changes to Jecode, one line per notification:
// R ready, C changed path, D deleted path, N old<TAB>new renamed path,
// O notifications were lost, S n marker echo, E error before exit.
// It exits when its input closes or the parent process exits.
public static class JecodeWatcher {
    static readonly object Gate = new object();
    static TextWriter Output;

    // Add-Type can compile this owned source once as a console executable.
    public static int Main(string[] arguments) { return Run(arguments); }

    static void Emit(string line) {
        lock (Gate) {
            try {
                Output.Write(line);
                Output.Write('\n');
                Output.Flush();
            } catch (Exception) {
                Environment.Exit(3);
            }
        }
    }

    // Win32 names cannot contain control characters, so lines stay unambiguous.
    static void Path(string kind, string name) {
        if (name == null) { Emit("O"); } else { Emit(kind + " " + name); }
    }

    static int Run(string[] arguments) {
        int parentId;
        if (arguments.Length != 2 || !int.TryParse(arguments[1], out parentId)) { return 2; }
        Output = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false));
        Process parent;
        try { parent = Process.GetProcessById(parentId); } catch (Exception) { return 4; }
        FileSystemWatcher watcher;
        try {
            watcher = new FileSystemWatcher(arguments[0]);
            watcher.IncludeSubdirectories = true;
            watcher.InternalBufferSize = 64 * 1024;
            watcher.NotifyFilter = NotifyFilters.FileName | NotifyFilters.DirectoryName
                | NotifyFilters.LastWrite | NotifyFilters.Size | NotifyFilters.Attributes
                | NotifyFilters.CreationTime | NotifyFilters.Security;
            watcher.Changed += delegate(object sender, FileSystemEventArgs change) { Path("C", change.Name); };
            watcher.Created += delegate(object sender, FileSystemEventArgs change) { Path("C", change.Name); };
            watcher.Deleted += delegate(object sender, FileSystemEventArgs change) { Path("D", change.Name); };
            watcher.Renamed += delegate(object sender, RenamedEventArgs change) {
                if (change.OldName == null || change.Name == null) { Emit("O"); }
                else { Emit("N " + change.OldName + "\t" + change.Name); }
            };
            watcher.Error += delegate(object sender, ErrorEventArgs failure) {
                Exception error = failure.GetException();
                if (error is InternalBufferOverflowException) { Emit("O"); return; }
                Emit("E " + error.Message.Replace('\r', ' ').Replace('\n', ' '));
                Environment.Exit(5);
            };
            watcher.EnableRaisingEvents = true;
        } catch (Exception error) {
            Emit("E " + error.Message.Replace('\r', ' ').Replace('\n', ' '));
            return 5;
        }
        Emit("R");
        Thread input = new Thread(delegate() {
            TextReader reader = new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false));
            string line;
            while ((line = reader.ReadLine()) != null) {
                if (line.StartsWith("S ")) { Emit(line); }
            }
            Environment.Exit(0);
        });
        input.IsBackground = true;
        input.Start();
        while (!parent.WaitForExit(1000)) { }
        GC.KeepAlive(watcher);
        return 0;
    }
}
