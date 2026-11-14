using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

// A bounded launch observer. It never starts user code until the application
// itself has been checked outside every enclosing Job Object.
public static partial class JecodeLaunch {
    const uint CreateSuspended = 0x00000004;
    const uint DetachedProcess = 0x00000008;
    const uint NewProcessGroup = 0x00000200;
    const uint StartfUseStdHandles = 0x00000100;
    const uint GenericReadWrite = 0xC0000000;
    const uint OpenExisting = 3;
    const uint WaitObject0 = 0;
    const uint WaitTimeout = 258;
    const uint Infinite = 0xffffffff;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int Size;
        public string Reserved;
        public string Desktop;
        public string Title;
        public uint X;
        public uint Y;
        public uint XSize;
        public uint YSize;
        public uint XCountChars;
        public uint YCountChars;
        public uint FillAttribute;
        public uint Flags;
        public short ShowWindow;
        public short Reserved2;
        public IntPtr ReservedBytes;
        public IntPtr StdInput;
        public IntPtr StdOutput;
        public IntPtr StdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation {
        public IntPtr Process;
        public IntPtr Thread;
        public uint ProcessId;
        public uint ThreadId;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct SecurityAttributes {
        public int Length;
        public IntPtr Descriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool Inherit;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool IsProcessInJob(IntPtr process, IntPtr job, [MarshalAs(UnmanagedType.Bool)] out bool inJob);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "CreateProcessW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CreateProcess(string application, StringBuilder commandLine,
        IntPtr processAttributes, IntPtr threadAttributes, [MarshalAs(UnmanagedType.Bool)] bool inherit,
        uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInformation process);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "CreateFileW")]
    static extern IntPtr CreateFile(string name, uint access, uint share, ref SecurityAttributes attributes,
        uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetHandleInformation(IntPtr handle, uint mask, uint flags);

    static int Error() { return Marshal.GetLastWin32Error(); }

    static string ReadString(BinaryReader input) {
        int length = input.ReadInt32();
        if (length < 0 || length > 1000000) throw new InvalidDataException("Invalid launch field length");
        byte[] bytes = input.ReadBytes(checked(length * 2));
        if (bytes.Length != length * 2) throw new EndOfStreamException("Incomplete launch field");
        return Encoding.Unicode.GetString(bytes);
    }

    static void Message(BinaryWriter output, byte kind, uint pid, string detail) {
        output.Write(kind);
        if (kind == 1 || kind == 5) output.Write(pid);
        if (kind == 0 || kind == 4 || kind == 5) {
            byte[] text = Encoding.UTF8.GetBytes(detail);
            output.Write(text.Length);
            output.Write(text);
        }
        output.Flush();
    }

    static bool StopSuspended(IntPtr process) {
        return TerminateProcess(process, 1) && WaitForSingleObject(process, 5000) == WaitObject0;
    }

    public static int Main(string[] args) {
        if (args.Length == 3 && args[0] == "--nested-test")
            return NestedFixture(args[1], args[2]);
        return Launch();
    }

    static int Launch() {
        var output = new BinaryWriter(Console.OpenStandardOutput());
        var process = new ProcessInformation();
        IntPtr nul = IntPtr.Zero;
        bool resumed = false, announced = false;
        try {
            bool inJob;
            if (!IsProcessInJob(GetCurrentProcess(), IntPtr.Zero, out inJob))
                throw new IOException("Could not verify host Job Object membership (Windows error " + Error() + ")");
            if (inJob) {
                Message(output, 0, 0, "The host kept the launch helper inside a Job Object; nothing was launched");
                return 1;
            }
            var input = new BinaryReader(Console.OpenStandardInput());
            string program = ReadString(input);
            int count = input.ReadInt32();
            if (count < 0 || count > 100000) throw new InvalidDataException("Invalid argument count");
            var command = new StringBuilder(JecodeWindowsCommand.Quote(program));
            for (int i = 0; i < count; i++) command.Append(' ').Append(JecodeWindowsCommand.Quote(ReadString(input)));
            int waitMs = input.ReadInt32();
            if (waitMs < 0 || waitMs > 60000) throw new InvalidDataException("Invalid observation time");
            string executable = JecodeWindowsCommand.ResolveProgram(program);
            var security = new SecurityAttributes();
            security.Length = Marshal.SizeOf(typeof(SecurityAttributes));
            security.Inherit = true;
            nul = CreateFile("NUL", GenericReadWrite, 3, ref security, OpenExisting, 0, IntPtr.Zero);
            if (nul == new IntPtr(-1)) throw new IOException("Could not open the null device (Windows error " + Error() + ")");
            foreach (int kind in new int[] { -10, -11, -12 }) {
                IntPtr handle = GetStdHandle(kind);
                if (handle != IntPtr.Zero && handle != new IntPtr(-1) && !SetHandleInformation(handle, 1, 0))
                    throw new IOException("Could not isolate the launch helper's pipes (Windows error " + Error() + ")");
            }
            var startup = new StartupInfo();
            startup.Size = Marshal.SizeOf(typeof(StartupInfo));
            startup.Flags = StartfUseStdHandles;
            startup.StdInput = nul;
            startup.StdOutput = nul;
            startup.StdError = nul;
            if (!CreateProcess(executable, command, IntPtr.Zero, IntPtr.Zero, true,
                CreateSuspended | DetachedProcess | NewProcessGroup, IntPtr.Zero, null, ref startup, out process))
                throw new IOException("CreateProcess failed (Windows error " + Error() + ")");
            if (!IsProcessInJob(process.Process, IntPtr.Zero, out inJob) || inJob) {
                string reason = inJob ? "The application remained in a Job Object" :
                    "Could not verify application Job Object membership (Windows error " + Error() + ")";
                if (StopSuspended(process.Process)) {
                    Message(output, 0, 0, reason + "; nothing was launched");
                } else {
                    Message(output, 5, process.ProcessId, reason + "; could not confirm cleanup of the suspended application");
                }
                return 1;
            }
            if (ResumeThread(process.Thread) == Infinite)
                throw new IOException("ResumeThread failed (Windows error " + Error() + ")");
            resumed = true;
            Message(output, 1, process.ProcessId, "");
            announced = true;
            uint wait = WaitForSingleObject(process.Process, (uint)waitMs);
            if (wait == WaitTimeout) {
                output.Write((byte)3);
            } else if (wait == WaitObject0) {
                uint code;
                if (!GetExitCodeProcess(process.Process, out code))
                    throw new IOException("GetExitCodeProcess failed (Windows error " + Error() + ")");
                output.Write((byte)2);
                output.Write(unchecked((int)code));
            } else {
                throw new IOException("WaitForSingleObject failed (Windows error " + Error() + ")");
            }
            output.Flush();
            return 0;
        } catch (Exception error) {
            if (process.Process != IntPtr.Zero && !resumed && StopSuspended(process.Process)) {
                Message(output, 0, 0, error.Message + "; nothing was launched");
            } else if (announced) {
                Message(output, 4, 0, error.Message);
            } else if (process.Process != IntPtr.Zero) {
                Message(output, 5, process.ProcessId, error.Message + "; launch outcome is unknown");
            } else {
                Message(output, 0, 0, error.Message + "; nothing was launched");
            }
            return 1;
        } finally {
            if (process.Thread != IntPtr.Zero) CloseHandle(process.Thread);
            if (process.Process != IntPtr.Zero) CloseHandle(process.Process);
            if (nul != IntPtr.Zero && nul != new IntPtr(-1)) CloseHandle(nul);
        }
    }
}
