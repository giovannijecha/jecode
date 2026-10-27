using System;
using System.IO;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

// Owned Windows boundary for an interactive terminal. The Rust process
// supervisor owns this host's Job Object; CreateProcessW inherits that job.
public static class JecodePty {
    const uint CreateSuspended = 0x00000004;
    const uint ExtendedStartup = 0x00080000;
    const int PseudoConsoleAttribute = 0x00020016;
    const uint Infinite = 0xffffffff;

    [StructLayout(LayoutKind.Sequential)]
    struct Coord {
        public short X, Y;
        public Coord(short x, short y) { X = x; Y = y; }
    }
    [StructLayout(LayoutKind.Sequential)]
    struct StartupInfo {
        public int Size;
        public IntPtr Reserved, Desktop, Title;
        public int X, Y, Width, Height, XChars, YChars, Attribute;
        public uint Flags;
        public short Show, ReservedSize;
        public IntPtr ReservedData, Input, Output, Error;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedStartupInfo { public StartupInfo Startup; public IntPtr Attributes; }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr Process, Thread; public uint Id, ThreadId; }
    sealed class ConsoleState { public IntPtr Handle; }

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
    static extern bool CreateProcessW(string application, StringBuilder command, IntPtr processSecurity,
        IntPtr threadSecurity, bool inherit, uint flags, IntPtr environment, string directory,
        ref ExtendedStartupInfo startup, out ProcessInfo process);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")]
    static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetStdHandle(int kind, IntPtr handle);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true, EntryPoint = "SearchPathW")]
    static extern uint SearchPath(string path, string file, string extension, uint length,
        StringBuilder result, IntPtr filePart);

    static void Check(bool success, string operation) {
        if (!success) throw new IOException(operation + " failed (Windows error " + Marshal.GetLastWin32Error() + ")");
    }
    static void CheckHr(int result) {
        if (result < 0) Marshal.ThrowExceptionForHR(result);
    }
    static void Close(ref IntPtr handle) {
        if (handle != IntPtr.Zero) { CloseHandle(handle); handle = IntPtr.Zero; }
    }
    static string ResolveProgram(string program) {
        if (!string.IsNullOrEmpty(Path.GetDirectoryName(program))) return program;
        var result = new StringBuilder(32768);
        uint length = SearchPath(null, program, ".exe", (uint)result.Capacity, result, IntPtr.Zero);
        if (length == 0 || length >= result.Capacity)
            throw new IOException("SearchPath failed (Windows error " + Marshal.GetLastWin32Error() + ")");
        return result.ToString();
    }
    static string Quote(string value) {
        var result = new StringBuilder("\"");
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
    static ProcessInfo StartChild(string program, string[] arguments, IntPtr console) {
        string executable = ResolveProgram(program);
        var command = new StringBuilder(Quote(program));
        foreach (string argument in arguments) command.Append(' ').Append(Quote(argument));
        IntPtr bytes = IntPtr.Zero;
        InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref bytes);
        if (bytes == IntPtr.Zero) throw new IOException("Could not size ConPTY startup attributes");
        IntPtr attributes = Marshal.AllocHGlobal(bytes);
        bool initialized = false;
        try {
            Check(InitializeProcThreadAttributeList(attributes, 1, 0, ref bytes), "InitializeProcThreadAttributeList");
            initialized = true;
            Check(UpdateProcThreadAttribute(attributes, 0, new IntPtr(PseudoConsoleAttribute),
                console, new IntPtr(IntPtr.Size), IntPtr.Zero, IntPtr.Zero), "UpdateProcThreadAttribute");
            var startup = new ExtendedStartupInfo();
            startup.Startup.Size = Marshal.SizeOf(typeof(ExtendedStartupInfo));
            startup.Attributes = attributes;
            // The supervisor redirects this host's standard handles. Clear
            // them only during client creation so ConPTY supplies its console.
            IntPtr savedInput = GetStdHandle(-10), savedOutput = GetStdHandle(-11), savedError = GetStdHandle(-12);
            try {
                Check(SetStdHandle(-10, IntPtr.Zero), "SetStdHandle input");
                Check(SetStdHandle(-11, IntPtr.Zero), "SetStdHandle output");
                Check(SetStdHandle(-12, IntPtr.Zero), "SetStdHandle error");
                ProcessInfo child;
                Check(CreateProcessW(executable, command, IntPtr.Zero, IntPtr.Zero, false,
                    CreateSuspended | ExtendedStartup, IntPtr.Zero, null, ref startup, out child), "CreateProcessW");
                return child;
            } finally {
                SetStdHandle(-10, savedInput);
                SetStdHandle(-11, savedOutput);
                SetStdHandle(-12, savedError);
            }
        } finally {
            if (initialized) DeleteProcThreadAttributeList(attributes);
            Marshal.FreeHGlobal(attributes);
        }
    }
    static NamedPipeServerStream ControlPipe(string name) {
        var security = new PipeSecurity();
        security.SetAccessRuleProtection(true, false);
        security.AddAccessRule(new PipeAccessRule(WindowsIdentity.GetCurrent().User,
            PipeAccessRights.FullControl, AccessControlType.Allow));
        return new NamedPipeServerStream(name, PipeDirection.In, 1, PipeTransmissionMode.Byte,
            PipeOptions.None, 4096, 4096, security);
    }
    static bool ReadExact(Stream source, byte[] bytes) {
        int offset = 0;
        while (offset < bytes.Length) {
            int count = source.Read(bytes, offset, bytes.Length - offset);
            if (count == 0) {
                if (offset == 0) return false;
                throw new EndOfStreamException("Incomplete PTY control frame");
            }
            offset += count;
        }
        return true;
    }
    static void ControlLoop(Stream source, FileStream input, ConsoleState state) {
        byte[] header = new byte[5];
        while (ReadExact(source, header)) {
            uint length = BitConverter.ToUInt32(header, 1);
            if (length > 1024 * 1024) throw new InvalidDataException("PTY control frame is too large");
            byte[] payload = new byte[length];
            if (!ReadExact(source, payload)) throw new EndOfStreamException("Incomplete PTY control payload");
            switch (header[0]) {
                case 1:
                    if (payload.Length > 0) {
                        input.Write(payload, 0, payload.Length);
                        input.Flush();
                    }
                    break;
                case 2:
                    if (payload.Length != 4) throw new InvalidDataException("Invalid PTY resize frame");
                    ushort cols = BitConverter.ToUInt16(payload, 0), rows = BitConverter.ToUInt16(payload, 2);
                    if (cols == 0 || rows == 0 || cols > short.MaxValue || rows > short.MaxValue)
                        throw new InvalidDataException("Invalid PTY size");
                    lock (state) {
                        if (state.Handle != IntPtr.Zero)
                            CheckHr(ResizePseudoConsole(state.Handle, new Coord((short)cols, (short)rows)));
                    }
                    break;
                case 3:
                    if (payload.Length != 0) throw new InvalidDataException("Invalid PTY interrupt frame");
                    input.WriteByte(3); // ConPTY delivers Ctrl-C to the console client.
                    input.Flush();
                    break;
                case 4:
                    if (payload.Length != 0) throw new InvalidDataException("Invalid PTY EOF frame");
                    input.Dispose();
                    return;
                default:
                    throw new InvalidDataException("Unknown PTY control operation");
            }
        }
    }
    static void ForwardOutput(Stream source, Stream destination, Action<Exception> failure) {
        byte[] buffer = new byte[8192];
        bool writable = true;
        try {
            int count;
            while ((count = source.Read(buffer, 0, buffer.Length)) > 0) {
                if (!writable) continue;
                try { destination.Write(buffer, 0, count); }
                catch (IOException ex) { writable = false; failure(ex); }
            }
            if (writable) destination.Flush();
        } catch (Exception ex) { failure(ex); }
    }
    public static int Main(string[] args) {
        IntPtr inputRead = IntPtr.Zero, inputWrite = IntPtr.Zero;
        IntPtr outputRead = IntPtr.Zero, outputWrite = IntPtr.Zero, console = IntPtr.Zero;
        ProcessInfo child = new ProcessInfo();
        FileStream input = null, output = null;
        NamedPipeServerStream control = null;
        Thread reader = null;
        object gate = new object();
        var state = new ConsoleState();
        Exception outputFailure = null, controlFailure = null;
        bool finished = false;
        try {
            if (args.Length < 4) throw new InvalidDataException("Missing PTY host arguments");
            string name = args[0];
            if (name.Length == 0) throw new InvalidDataException("Missing PTY control endpoint");
            foreach (char c in name)
                if (!char.IsLetterOrDigit(c) && c != '-') throw new InvalidDataException("Invalid PTY control endpoint");
            short cols = short.Parse(args[1]), rows = short.Parse(args[2]);
            if (cols <= 0 || rows <= 0) throw new InvalidDataException("Invalid PTY size");
            Check(CreatePipe(out inputRead, out inputWrite, IntPtr.Zero, 0), "CreatePipe input");
            Check(CreatePipe(out outputRead, out outputWrite, IntPtr.Zero, 0), "CreatePipe output");
            CheckHr(CreatePseudoConsole(new Coord(cols, rows), inputRead, outputWrite, 0, out console));
            state.Handle = console;
            Close(ref inputRead); Close(ref outputWrite);
            input = new FileStream(new SafeFileHandle(inputWrite, true), FileAccess.Write);
            inputWrite = IntPtr.Zero;
            output = new FileStream(new SafeFileHandle(outputRead, true), FileAccess.Read);
            outputRead = IntPtr.Zero;
            string[] arguments = new string[args.Length - 4];
            Array.Copy(args, 4, arguments, 0, arguments.Length);
            child = StartChild(args[3], arguments, console);
            reader = new Thread(() => ForwardOutput(output, Console.OpenStandardOutput(),
                ex => { lock (gate) outputFailure = ex; }));
            reader.IsBackground = true;
            reader.Start();
            if (ResumeThread(child.Thread) == Infinite) throw new IOException("ResumeThread failed");
            // The endpoint appears only after the target was successfully started.
            control = ControlPipe(name);
            control.WaitForConnection();
            var controller = new Thread(() => {
                try { ControlLoop(control, input, state); }
                catch (Exception ex) {
                    lock (gate) controlFailure = ex;
                    TerminateProcess(child.Process, 1);
                }
            });
            controller.IsBackground = true;
            controller.Start();
            if (WaitForSingleObject(child.Process, Infinite) != 0) throw new IOException("WaitForSingleObject failed");
            uint exitCode;
            Check(GetExitCodeProcess(child.Process, out exitCode), "GetExitCodeProcess");
            lock (state) { ClosePseudoConsole(state.Handle); state.Handle = IntPtr.Zero; console = IntPtr.Zero; }
            reader.Join();
            lock (gate) {
                if (outputFailure != null) throw new IOException("ConPTY output forwarding failed", outputFailure);
                if (controlFailure != null) throw new IOException("ConPTY control failed", controlFailure);
            }
            finished = true;
            return unchecked((int)exitCode);
        } catch (Exception ex) {
            Console.Error.WriteLine("Windows PTY bridge: " + ex.Message);
            return 1;
        } finally {
            if (child.Process != IntPtr.Zero) {
                if (!finished) TerminateProcess(child.Process, 1);
                CloseHandle(child.Process);
            }
            if (child.Thread != IntPtr.Zero) CloseHandle(child.Thread);
            lock (state) {
                if (state.Handle != IntPtr.Zero) ClosePseudoConsole(state.Handle);
                state.Handle = IntPtr.Zero;
            }
            if (control != null) control.Dispose();
            if (input != null) input.Dispose();
            if (output != null) output.Dispose();
            Close(ref inputRead); Close(ref inputWrite); Close(ref outputRead); Close(ref outputWrite);
        }
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint ResumeThread(IntPtr thread);
}
