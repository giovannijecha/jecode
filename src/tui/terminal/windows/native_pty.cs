using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

// Owned, headless verification host for the documented Windows ConPTY ABI.
public sealed class JecodeTestPty : IDisposable {
    [StructLayout(LayoutKind.Sequential)]
    struct Coord { public short X, Y; public Coord(short x, short y) { X = x; Y = y; } }
    [StructLayout(LayoutKind.Sequential)]
    struct Startup {
        public int Size;
        public IntPtr Reserved, Desktop, Title;
        public int X, Y, Width, Height, XChars, YChars, Attribute;
        public uint Flags;
        public short Show, ReservedSize;
        public IntPtr ReservedData, Input, Output, Error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct Extended { public Startup Startup; public IntPtr Attributes; }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr Process, Thread; public uint Id, ThreadId; }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CreatePipe(out IntPtr read, out IntPtr write, IntPtr security, uint size);
    [DllImport("kernel32.dll")]
    static extern int CreatePseudoConsole(Coord size, IntPtr input, IntPtr output, uint flags, out IntPtr console);
    [DllImport("kernel32.dll")]
    static extern int ResizePseudoConsole(IntPtr console, Coord size);
    [DllImport("kernel32.dll")]
    static extern void ClosePseudoConsole(IntPtr console);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool InitializeProcThreadAttributeList(IntPtr list, int count, uint flags, ref IntPtr size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags, IntPtr attribute, IntPtr value, IntPtr size, IntPtr previous, IntPtr returned);
    [DllImport("kernel32.dll")]
    static extern void DeleteProcThreadAttributeList(IntPtr list);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcessW(string application, StringBuilder command, IntPtr processSecurity, IntPtr threadSecurity, bool inherit, uint flags, IntPtr environment, string directory, ref Extended startup, out ProcessInfo process);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")]
    static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
    [DllImport("kernel32.dll")]
    static extern bool GetExitCodeProcess(IntPtr handle, out uint code);
    [DllImport("kernel32.dll")]
    static extern bool TerminateProcess(IntPtr handle, uint code);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetStdHandle(int handle);
    [DllImport("kernel32.dll")]
    static extern bool SetStdHandle(int handle, IntPtr value);

    IntPtr console, process;
    uint processId;
    FileStream input, output;
    Thread reader;
    readonly StringBuilder received = new StringBuilder();
    readonly object gate = new object();

    static void Check(bool success) {
        if (!success) throw new IOException("ConPTY fixture error " + Marshal.GetLastWin32Error());
    }
    static void Result(int result) { if (result < 0) Marshal.ThrowExceptionForHR(result); }

    public JecodeTestPty() {
        IntPtr inputRead, inputWrite, outputRead, outputWrite;
        Check(CreatePipe(out inputRead, out inputWrite, IntPtr.Zero, 0));
        Check(CreatePipe(out outputRead, out outputWrite, IntPtr.Zero, 0));
        try {
            Result(CreatePseudoConsole(new Coord(80, 24), inputRead, outputWrite, 0, out console));
            input = new FileStream(new SafeFileHandle(inputWrite, true), FileAccess.Write);
            output = new FileStream(new SafeFileHandle(outputRead, true), FileAccess.Read);
        } finally { CloseHandle(inputRead); CloseHandle(outputWrite); }
        reader = new Thread(() => {
            using (var text = new StreamReader(output, Encoding.UTF8)) {
                char[] buffer = new char[8192];
                try {
                    int count;
                    while ((count = text.Read(buffer, 0, buffer.Length)) > 0) {
                        lock (gate) {
                            received.Append(buffer, 0, count);
                            if (received.Length > 4 * 1024 * 1024) throw new IOException("ConPTY output exceeded fixture limit");
                            Monitor.PulseAll(gate);
                        }
                    }
                } catch (IOException) { }
            }
        });
        reader.IsBackground = true;
        reader.Start();
    }

    public void Start(string executable, string directory, string scenario) {
        Environment.SetEnvironmentVariable("JECODE_NATIVE_DIR", directory);
        Environment.SetEnvironmentVariable("JECODE_NATIVE_CASE", scenario);
        IntPtr bytes = IntPtr.Zero;
        InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref bytes);
        IntPtr attributes = Marshal.AllocHGlobal(bytes);
        bool initialized = false;
        try {
            Check(InitializeProcThreadAttributeList(attributes, 1, 0, ref bytes));
            initialized = true;
            Check(UpdateProcThreadAttribute(attributes, 0, new IntPtr(0x20016), console, new IntPtr(IntPtr.Size), IntPtr.Zero, IntPtr.Zero));
            var startup = new Extended();
            startup.Startup.Size = Marshal.SizeOf(typeof(Extended));
            startup.Attributes = attributes;
            var command = new StringBuilder("\"" + executable + "\" --exact tui::terminal::windows::native_tests::native_console_fixture --ignored --nocapture --test-threads=1");
            ProcessInfo child;
            // A redirected test runner has pipe-valued standard handles. Leave
            // them empty only while creating the client so ConPTY supplies its
            // own console handles. The host's reader uses independent streams.
            IntPtr savedInput = GetStdHandle(-10), savedOutput = GetStdHandle(-11), savedError = GetStdHandle(-12);
            try {
                Check(SetStdHandle(-10, IntPtr.Zero)); Check(SetStdHandle(-11, IntPtr.Zero)); Check(SetStdHandle(-12, IntPtr.Zero));
                Check(CreateProcessW(executable, command, IntPtr.Zero, IntPtr.Zero, false, 0x80000, IntPtr.Zero, directory, ref startup, out child));
            } finally {
                SetStdHandle(-10, savedInput); SetStdHandle(-11, savedOutput); SetStdHandle(-12, savedError);
            }
            process = child.Process;
            processId = child.Id;
            CloseHandle(child.Thread);
        } finally {
            if (initialized) DeleteProcThreadAttributeList(attributes);
            Marshal.FreeHGlobal(attributes);
        }
    }

    public void Send(string text) {
        byte[] bytes = Encoding.UTF8.GetBytes(text);
        input.Write(bytes, 0, bytes.Length); input.Flush();
    }
    public int Mark() { lock (gate) { return received.Length; } }
    public void Wait(string text, int after = 0) {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        while (clock.ElapsedMilliseconds < 20000) {
            lock (gate) {
                string plain = System.Text.RegularExpressions.Regex.Replace(received.ToString(after, received.Length - after), "\x1b\\[[0-?]*[ -/]*[@-~]", "");
                if (plain.Contains(text)) return;
            }
            Thread.Sleep(10);
        }
        lock (gate) { throw new IOException("ConPTY did not show " + text + ": " + received); }
    }

    // Reader wakeups locate the first matching VT bytes without a 10 ms
    // polling floor. This observes the stream, not terminal pixels or keys.
    public void WaitFast(string text, int after = 0) {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        lock (gate) {
            while (clock.ElapsedMilliseconds < 20000) {
                string plain = System.Text.RegularExpressions.Regex.Replace(
                    received.ToString(after, received.Length - after),
                    "\x1b\\[[0-?]*[ -/]*[@-~]", "");
                if (plain.Contains(text)) return;
                long remaining = 20000 - clock.ElapsedMilliseconds;
                if (remaining <= 0) break;
                Monitor.Wait(gate, (int)remaining);
            }
            int length = Math.Min(received.Length, 2000);
            throw new IOException("ConPTY did not show " + text + ": " + received.ToString(received.Length - length, length));
        }
    }

    static int ReadCount(string path) {
        for (int attempt = 0; attempt < 20; attempt++) {
            try {
                int count;
                if (int.TryParse(File.ReadAllText(path), out count)) return count;
            } catch (IOException) { }
            Thread.Sleep(2);
        }
        throw new IOException("Checkpoint write counter was not readable");
    }

    static int WaitNextCount(string path, int previous) {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        while (clock.ElapsedMilliseconds < 20000) {
            if (ReadCount(path) > previous) return ReadCount(path);
            Thread.Sleep(5);
        }
        throw new IOException("Checkpoint writes stopped during ConPTY latency samples");
    }

    static void PrintLatency(string phase, string kind, double[] samples) {
        double[] sorted = (double[])samples.Clone();
        Array.Sort(sorted);
        var culture = System.Globalization.CultureInfo.InvariantCulture;
        string name = "NATIVE_LATENCY_" + phase + "_" + kind;
        Console.WriteLine(name + "_US=" + string.Join(",", Array.ConvertAll(samples, value => value.ToString("F3", culture))));
        Console.WriteLine(name + "_P50_US=" + sorted[49].ToString("F3", culture));
        Console.WriteLine(name + "_P95_US=" + sorted[94].ToString("F3", culture));
    }

    void Latency(string directory, bool active) {
        WaitFast("Ask anything");
        string phase = active ? "active" : "idle";
        string counter = Path.Combine(directory, "home", "latency-count.txt");
        int before = active ? WaitNextCount(counter, 0) : 0, midway = before;
        double[] burst = new double[100], key = new double[100];
        for (int index = 0; index < burst.Length; index++) {
            string marker = "q" + index.ToString("D3");
            int start = Mark();
            long sent = System.Diagnostics.Stopwatch.GetTimestamp();
            Send(marker);
            WaitFast(marker, start);
            long seen = System.Diagnostics.Stopwatch.GetTimestamp();
            burst[index] = (seen - sent) * 1000000.0 / System.Diagnostics.Stopwatch.Frequency;
            int keyStart = Mark();
            sent = System.Diagnostics.Stopwatch.GetTimestamp();
            Send("!");
            WaitFast("!", keyStart);
            seen = System.Diagnostics.Stopwatch.GetTimestamp();
            key[index] = (seen - sent) * 1000000.0 / System.Diagnostics.Stopwatch.Frequency;
            int clear = Mark();
            Send("\x15");
            WaitFast("Ask anything", clear);
            if (active && index == 49) midway = WaitNextCount(counter, before);
        }
        int after = active ? WaitNextCount(counter, midway) : 0;
        PrintLatency(phase, "BURST4", burst);
        PrintLatency(phase, "KEY", key);
        if (active) Console.WriteLine("NATIVE_LATENCY_ACTIVE_WRITES=" + before + "," + midway + "," + after);
        using (var child = System.Diagnostics.Process.GetProcessById((int)processId)) {
            child.Refresh();
            Console.WriteLine("NATIVE_LATENCY_" + phase + "_CHILD_RUST_WORKING_SET_BYTES=" + child.WorkingSet64);
            Console.WriteLine("NATIVE_LATENCY_" + phase + "_CHILD_RUST_PRIVATE_BYTES=" + child.PrivateMemorySize64);
            Console.WriteLine("NATIVE_LATENCY_" + phase + "_CHILD_RUST_PEAK_RSS_BYTES=" + child.PeakWorkingSet64);
        }
        Send("\x11");
        WaitFast("NATIVE_LATENCY_RESTORED");
        Finish(true);
    }
    public void Resize(short width, short height) { Result(ResizePseudoConsole(console, new Coord(width, height))); Thread.Sleep(200); }
    public void Kill() { Check(TerminateProcess(process, 17)); Finish(false); }
    public void Finish(bool success) {
        if (WaitForSingleObject(process, 20000) != 0) throw new IOException("ConPTY child did not exit");
        uint code; Check(GetExitCodeProcess(process, out code));
        if (success && code != 0) {
            lock (gate) { throw new IOException("ConPTY child exited " + code + ": " + received); }
        }
        CloseHandle(process); process = IntPtr.Zero;
    }

    public static void Run(string executable, string directory, string scenario) {
        using (var pty = new JecodeTestPty()) {
            pty.Start(executable, directory, scenario);
            if (scenario == "latency-idle" || scenario == "latency-active") {
                pty.Latency(directory, scenario == "latency-active");
            } else if (scenario == "interactive") {
                pty.Wait("Ask anything");
                pty.Send("native prompt α🙂"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("Native row 099");
                pty.Wait("Complete ·");
                int expanded = pty.Mark(); pty.Send("\x0f");
                pty.Wait("native detail 09", expanded);
                int collapsed = pty.Mark(); pty.Send("\x0f");
                pty.Wait("Native row 099", collapsed);
                // Routing at normal, narrow and short sizes; focus starts on the current row.
                pty.Send("/routing"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("Routing · fixture/model");
                pty.Wait("current · no routing preferences");
                pty.Wait("$/M input/output after discounts");
                pty.Resize(40, 12); pty.Send("\x1b[A"); pty.Wait("Pinned to two/fp8");
                pty.Resize(44, 6); pty.Send("\x1b[A"); pty.Wait("› 2. one/fp8");
                // Esc closes quietly; the draft below only reaches an open composer.
                pty.Send("\x1b"); Thread.Sleep(150);
                pty.Resize(80, 24);
                pty.Send("kept draft β🙂"); Thread.Sleep(150);
                // Windows Terminal delivers a file drop as a paste of its quoted path.
                pty.Send("\x1b[200~\"" + System.IO.Path.Combine(directory, "drop me.txt") + "\"\x1b[201~");
                pty.Wait("Attached 1 item");
                pty.Send("\x1b[<64;10;5M"); pty.Wait("Back to bottom");
                pty.Send("\x1b"); Thread.Sleep(150);
                pty.Send("\x10"); pty.Wait("Prompt history");
                pty.Send("\x0e"); Thread.Sleep(150);
                pty.Send("\x1b[1;3A"); pty.Wait("No pending drafts");
                pty.Send("\x1b"); Thread.Sleep(150);
                pty.Send("\x1b[6~"); Thread.Sleep(100);
                pty.Send("\x1b[5~"); Thread.Sleep(100);
                pty.Send("\x1b[A"); Thread.Sleep(100); pty.Send("\x1b[B");
                pty.Resize(40, 12); pty.Resize(5, 2); pty.Resize(80, 24);
                pty.Send("\x11");
                pty.Wait("Jecode closed: 1 prompts, 2 tools");
                pty.Wait("Resume: jecode resume");
                pty.Wait("NATIVE_RESTORED");
                pty.Finish(true);
            } else if (scenario == "delete-current") {
                pty.Wait("Ask anything");
                pty.Send("/resume"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("No saved conversations");
                int initiallyEmpty = pty.Mark(); pty.Send("\x1b");
                pty.Wait("Ask anything", initiallyEmpty);
                pty.Send("native delete request"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("Native completed.");
                int openSaved = pty.Mark(); pty.Send("/resume"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("native delete request", openSaved);
                int confirmDelete = pty.Mark(); pty.Send("\x04");
                pty.Wait("Delete?", confirmDelete);
                int deleted = pty.Mark(); pty.Send("\r");
                pty.Wait("No saved conversations", deleted);
                int afterDelete = pty.Mark(); pty.Send("\x1b");
                pty.Wait("Ask anything", afterDelete);
                int noTools = pty.Mark(); pty.Send("\x0f");
                pty.Send("still in composer");
                pty.Wait("still in composer", noTools);
                pty.Send("\x11");
                pty.Wait("NATIVE_DELETE_RESTORED");
                pty.Finish(true);
            } else if (scenario == "owner-death") {
                pty.Wait("NATIVE_OWNER_READY");
                pty.Kill(); Thread.Sleep(1200);
                pty.Start(executable, directory, "verify-owner-death");
                pty.Wait("NATIVE_OWNER_RESTORED");
                pty.Finish(true);
            } else {
                pty.Wait("NATIVE_GUARDIAN_RESTORED");
                pty.Finish(true);
            }
        }
    }

    public void Dispose() {
        if (process != IntPtr.Zero) { TerminateProcess(process, 19); WaitForSingleObject(process, 3000); CloseHandle(process); }
        if (console != IntPtr.Zero) { ClosePseudoConsole(console); console = IntPtr.Zero; }
        if (input != null) input.Dispose();
        if (reader != null && !reader.Join(3000)) throw new IOException("ConPTY reader did not close");
    }
}
