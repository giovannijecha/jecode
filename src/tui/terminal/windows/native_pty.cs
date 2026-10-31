using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

// Owned, headless verification host for the documented Windows ConPTY ABI.
public sealed partial class JecodeTestPty : IDisposable {
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
        // Read available pipe bytes directly. StreamReader.Read(char[]) may
        // hold a short final repaint until its character buffer fills or EOF.
        reader = new Thread(() => {
            using (output) {
                byte[] bytes = new byte[8192];
                char[] characters = new char[Encoding.UTF8.GetMaxCharCount(bytes.Length)];
                Decoder decoder = Encoding.UTF8.GetDecoder();
                try {
                    int count;
                    while ((count = output.Read(bytes, 0, bytes.Length)) > 0) {
                        int length = decoder.GetChars(bytes, 0, count, characters, 0, false);
                        lock (gate) {
                            received.Append(characters, 0, length);
                            if (received.Length > 4 * 1024 * 1024) throw new IOException("ConPTY output exceeded fixture limit");
                            Monitor.PulseAll(gate);
                        }
                    }
                    int tail = decoder.GetChars(new byte[0], 0, 0, characters, 0, true);
                    if (tail > 0) {
                        lock (gate) {
                            received.Append(characters, 0, tail);
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

    public void Start(string executable, string directory, string scenario, string launcher = null) {
        Environment.SetEnvironmentVariable("JECODE_NATIVE_DIR", directory);
        Environment.SetEnvironmentVariable("JECODE_NATIVE_CASE", scenario);
        if (scenario.StartsWith("startup-") || scenario == "npm-launcher") {
            Environment.SetEnvironmentVariable("JECODE_HOME", directory);
            Environment.SetEnvironmentVariable("OPENROUTER_API_KEY", null);
            Environment.SetEnvironmentVariable("OPENROUTER_MODEL", null);
        }
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
            var command = new StringBuilder(scenario == "npm-launcher"
                ? "\"" + executable + "\" \"" + launcher + "\""
                : "\"" + executable + "\" --exact tui::terminal::windows::native_tests::native_console_fixture --ignored --nocapture --test-threads=1");
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
    void EnsureVt(string text, string label) {
        lock (gate) {
            if (!received.ToString().Contains(text)) throw new IOException("ConPTY missing " + label + ": " + text);
        }
    }
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

    public void WaitRaw(string text, int after = 0) {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        lock (gate) {
            while (clock.ElapsedMilliseconds < 20000) {
                if (received.ToString(after, received.Length - after).Contains(text)) return;
                Monitor.Wait(gate, 20);
            }
            int length = Math.Min(received.Length, 2000);
            throw new IOException("ConPTY did not emit " + text + ": " +
                received.ToString(received.Length - length, length).Replace("\x1b", "[ESC]"));
        }
    }

    public void EnsureNoRaw(string text, int after) {
        lock (gate) {
            if (received.ToString(after, received.Length - after).Contains(text))
                throw new IOException("ConPTY unexpectedly emitted " + text);
        }
    }

    void EnsureNoText(string text, int after) {
        lock (gate) {
            string plain = System.Text.RegularExpressions.Regex.Replace(
                received.ToString(after, received.Length - after), "\x1b\\[[0-?]*[ -/]*[@-~]", "");
            int found = plain.IndexOf(text, StringComparison.Ordinal);
            if (found >= 0) {
                if (text != "[ESC]") throw new IOException("ConPTY unexpectedly showed " + text);
                int start = Math.Max(0, found - 100), end = Math.Min(plain.Length, found + 160);
                throw new IOException("ConPTY unexpectedly showed [ESC] near: " +
                    plain.Substring(start, end - start).Replace("\r", " ").Replace("\n", " "));
            }
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
            if (scenario == "startup-plain") {
                pty.Wait("Providers:"); pty.Send("/cancel\r");
                pty.Wait("NATIVE_STARTUP_RESTORED"); pty.Finish(true);
            } else if (scenario == "startup-setup-empty") {
                pty.Wait("Providers:"); pty.Send("1\r");
                pty.Wait("OpenRouter API key (hidden):");
                pty.EnsureNoText("Enter keeps the saved key", 0); pty.Send("/cancel\r");
                pty.Wait("NATIVE_STARTUP_RESTORED"); pty.Finish(true);
            } else if (scenario == "startup-splash") {
                pty.Splash();
            } else if (scenario.StartsWith("startup-")) {
                pty.Wait("Ask anything");
                pty.Wait(scenario.StartsWith("startup-fallback") ? "Default provider unavailable" : "No configured provider");
                pty.EnsureVt("JECODE_WARNING_FOREGROUND", "warning foreground");
                pty.EnsureVt("JECODE_WARNING_SURFACE", "warning surface");
                if (scenario == "startup-unconfigured-prompt") {
                    int typed = pty.Mark(); pty.Send("Keep my task");
                    pty.Wait("Keep my task", typed);
                    int request = pty.Mark(); pty.Send("\r");
                    pty.Wait("No configured provider", request);
                    int cleared = pty.Mark(); pty.Send("\x15");
                    pty.Wait("Ask anything", cleared);
                }
                int next = pty.Mark(); pty.Send("/new");
                pty.Wait("Commands", next); pty.Send("\r");
                string result = scenario == "startup-fallback-new" ? "current model kept"
                    : scenario == "startup-fallback" ? "Choose a model" : "No configured provider";
                pty.Wait(result, next);
                pty.EnsureNoText("has no configured account", next);
                int settings = pty.Mark(); pty.Send("/settings");
                pty.Wait("Commands", settings); pty.Send("\r");
                pty.Wait("Providers", settings);
                pty.Send("\x1b"); Thread.Sleep(150); pty.Send("\x11");
                pty.Wait("NATIVE_STARTUP_RESTORED");
                pty.Finish(true);
            } else if (scenario.StartsWith("setup-")) {
                pty.Wait("SETUP_SIGN_IN_WAITING");
                if (scenario == "setup-cancel") pty.Send("\r");
                pty.Wait("SETUP_NEXT_PROMPT: ");
                pty.Send("next\r");
                pty.Wait("SETUP_NATIVE_DONE");
                pty.Finish(true);
            } else if (scenario == "latency-idle" || scenario == "latency-active") {
                pty.Latency(directory, scenario == "latency-active");
            } else if (scenario == "direct-input") {
                pty.Wait("Ask anything");
                pty.Send("start terminal"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("Native terminal started.");
                pty.Send("/terminal"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("1 running");
                pty.Send("\r");
                pty.Wait("i  input");
                int started = pty.Mark(); pty.Send("i");
                pty.Wait("Ctrl+]  back", started);
                pty.Wait("NATIVE_PASSWORD_READY");
                pty.Send("NATIVE_SECRET_42\uD83D\uDE42\r");
                pty.Wait("NATIVE_PTY_DONE", started);
                pty.EnsureNoText("NATIVE_SECRET_42", started);
                pty.EnsureNoText("[ESC]", started);
                int detached = pty.Mark(); pty.Send("\x1d");
                pty.Wait("i  input", detached);
                // Match the new input-mode frame, not an earlier queued repaint.
                int reattached = pty.Mark(); pty.Send("i");
                pty.Wait("Ctrl+]  back", reattached);
                pty.Wait("NATIVE_SECOND_READY");
                pty.Send("finish\r");
                pty.Wait("NATIVE_PTY_END", detached);
                pty.Wait("exited 0", detached);
                pty.EnsureNoText("[ESC]", detached);
                pty.Send("\x1b"); Thread.Sleep(150); pty.Send("\x1b"); Thread.Sleep(150);
                pty.Send("kept draft"); pty.Wait("kept draft");
                pty.Send("\x11");
                pty.Wait("NATIVE_DIRECT_INPUT_RESTORED");
                pty.Finish(true);
            } else if (scenario == "background-light") {
                pty.WaitRaw("48;2;12;12;12m");
                pty.Wait("NATIVE_LIGHT_FRAME");
                pty.EnsureNoRaw("\x1b[49m", 0);
                pty.Wait("NATIVE_BACKGROUND_RESTORED");
                pty.Finish(true);
            } else if (scenario == "background-native") {
                pty.WaitFast("Ask anything");
                int reported = pty.Mark();
                pty.WaitRaw("48;2;12;12;12m", reported);
                pty.Send("\x11");
                pty.Wait("NATIVE_BACKGROUND_DETECTED_RESTORED");
                pty.Finish(true);
            } else if (scenario == "permissions") {
                pty.Permissions();
            } else if (scenario == "auth-panels") {
                pty.Wait("NATIVE_AUTH_PANELS_VERIFIED");
                pty.Finish(true);
            } else if (scenario == "interactive") {
                // Each renderer phase completes before the TUI startup wait begins.
                pty.Wait("NATIVE_FULL_WIDTH_VERIFIED");
                pty.Wait("NATIVE_CARDS_VERIFIED");
                pty.Wait("NATIVE_ACTIVITY_VERIFIED");
                pty.Wait("NATIVE_AGENTS_VERIFIED");
                pty.Wait("NATIVE_WORK_TOOLS_VERIFIED");
                pty.Wait("Ask anything");
                pty.Send("native prompt α🙂"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("Native row 099");
                pty.Wait("Done");
                int chapter = pty.Mark(); pty.Send("\x1b[1;3H");
                pty.Wait("READ  drop me.txt", chapter);
                pty.Wait("1 line", chapter);
                pty.Send("\x1b[1;3F");
                int expanded = pty.Mark(); pty.Send("\x0f");
                pty.Wait("Tool details expanded", expanded);
                pty.Send("after expansion");
                pty.Wait("after expansion", expanded);
                pty.EnsureNoText("native detail", expanded);
                int clearExpanded = pty.Mark(); pty.Send("\x15");
                pty.Wait("Ask anything", clearExpanded);
                int collapsed = pty.Mark(); pty.Send("\x0f");
                pty.Wait("Tool details collapsed", collapsed);
                pty.Send("after collapse");
                pty.Wait("after collapse", collapsed);
                int clearCollapsed = pty.Mark(); pty.Send("\x15");
                pty.Wait("Ask anything", clearCollapsed);
                // Routing at normal, narrow and short sizes; focus starts on the current row.
                pty.Send("/routing"); Thread.Sleep(150); pty.Send("\r");
                pty.Wait("Routing · fixture/model");
                pty.Wait("no routing preferences");
                pty.Wait("$/M after discounts");
                // Focus moves while the panel shrinks; the draft and column header leave first.
                int pinned = pty.Mark(); pty.Send("\x1b[A"); pty.Wait("› two/fp8", pinned);
                int narrow = pty.Mark(); pty.Resize(40, 12); pty.Send("\x1b[A"); pty.Wait("› one/fp8", narrow);
                int shortened = pty.Mark(); pty.Resize(44, 6); pty.Send("\x1b[A"); pty.Wait("› Auto", shortened);
                // Esc closes quietly; the draft below only reaches an open composer.
                pty.Send("\x1b"); Thread.Sleep(150);
                pty.Resize(80, 24);
                pty.Send("kept draft β🙂"); Thread.Sleep(150);
                // Windows Terminal delivers a file drop as a paste of its quoted path.
                pty.Send("\x1b[200~\"" + System.IO.Path.Combine(directory, "drop me.txt") + "\"\x1b[201~");
                pty.Wait("Attached 1 item");
                pty.Send("\x1b[<64;10;5M"); pty.Wait("Alt+End  back to bottom");
                pty.Send("\x1b[1;3F"); Thread.Sleep(150);
                pty.Send("\x10"); pty.Wait("Prompt history");
                pty.Send("\x0e"); Thread.Sleep(150);
                pty.Send("\x1b[6~"); Thread.Sleep(100);
                pty.Send("\x1b[5~"); Thread.Sleep(100);
                int draftKept = pty.Mark();
                pty.Send("\x1b[A"); Thread.Sleep(100); pty.Send("\x1b[B");
                pty.Resize(40, 12); pty.Resize(5, 2); pty.Resize(80, 24);
                pty.Wait("kept draft", draftKept);
                pty.Send("\x11");
                pty.Wait("jecode closed · 1 prompt · 2 tools");
                pty.Wait("   jecode resume ");
                pty.Wait("NATIVE_RESTORED");
                pty.Finish(true);
            } else if (scenario == "input-recovery") {
                pty.Wait("NATIVE_INPUT_RECOVERY_RESTORED");
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
                pty.Wait("Delete this conversation and its files?", confirmDelete);
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
