using System;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Runtime.InteropServices;

public static class JecodeTerminalEntry {
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int handle);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleMode(IntPtr handle, out uint mode);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetConsoleMode(IntPtr handle, uint mode);

    public static int Main(string[] args) {
        StreamWriter writer = null;
        NamedPipeServerStream pipe = null;
        try {
            if (args.Length == 1 && (args[0] == "secret" || args[0] == "wait-enter")) {
                IntPtr input = GetStdHandle(-10);
                uint saved;
                if (!GetConsoleMode(input, out saved)) return 2;
                ConsoleCancelEventHandler restore = delegate(object sender, ConsoleCancelEventArgs eventArgs) {
                    SetConsoleMode(input, saved);
                };
                Console.CancelKeyPress += restore;
                try {
                    if (args[0] == "secret" && !SetConsoleMode(input, saved & ~4u)) return 2;
                    string line = Console.ReadLine();
                    if (line == null) return 3;
                    if (args[0] == "secret") Console.Out.WriteLine(line);
                    return 0;
                } finally {
                    SetConsoleMode(input, saved);
                    Console.CancelKeyPress -= restore;
                }
            }
            if (args.Length == 4 && args[0] == "restore") {
                JecodeConsole.Restore(uint.Parse(args[1]), uint.Parse(args[2]), uint.Parse(args[3]));
                return 0;
            }
            if (args.Length != 1 || args[0].Length == 0) return 2;
            pipe = new NamedPipeServerStream(args[0], PipeDirection.Out, 1,
                PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
            pipe.WaitForConnection();
            writer = new StreamWriter(pipe, new UTF8Encoding(false), 1024, true);
            writer.NewLine = "\n";
            writer.AutoFlush = true;
            JecodeConsole.Run(Console.OpenStandardInput(), writer);
            return 0;
        } catch (Exception error) {
            if (writer != null) writer.WriteLine("E|Terminal input failed: " + error.Message);
            return 1;
        } finally {
            try { if (writer != null) writer.Dispose(); }
            finally { if (pipe != null) pipe.Dispose(); }
        }
    }
}
