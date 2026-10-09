using System;
using System.IO;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

// This owned Windows boundary keeps the job handle outside the target process tree.
public static class JecodeProcess {
    // Add-Type can compile this owned source once as a console executable.
    public static int Main() { return Run(); }

    const uint CreateSuspended = 0x00000004;
    const uint ExtendedStartupInfoPresent = 0x00080000;
    const int AttributeJobList = 0x0002000d;
    const uint CreateNoWindow = 0x08000000;
    const uint StartfUseStdHandles = 0x00000100;
    const uint JobLimitKillOnClose = 0x00002000;
    const uint DuplicateSameAccess = 0x00000002;
    const uint HandleFlagInherit = 0x00000001;
    const uint Infinite = 0xffffffff;

    [StructLayout(LayoutKind.Sequential)]
    struct SecurityAttributes {
        public int Length;
        public IntPtr SecurityDescriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool InheritHandle;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int Size;
        public IntPtr Reserved;
        public IntPtr Desktop;
        public IntPtr Title;
        public int X;
        public int Y;
        public int Width;
        public int Height;
        public int XChars;
        public int YChars;
        public int FillAttribute;
        public uint Flags;
        public short ShowWindow;
        public short Reserved2;
        public IntPtr Reserved3;
        public IntPtr StdInput;
        public IntPtr StdOutput;
        public IntPtr StdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct StartupInfoEx {
        public StartupInfo Startup;
        public IntPtr AttributeList;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInformation {
        public IntPtr Process;
        public IntPtr Thread;
        public int ProcessId;
        public int ThreadId;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimitInformation {
        public long ProcessUserTimeLimit;
        public long JobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedLimitInformation {
        public BasicLimitInformation Basic;
        public IoCounters Io;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct BasicAccountingInformation {
        public long TotalUserTime;
        public long TotalKernelTime;
        public long ThisPeriodTotalUserTime;
        public long ThisPeriodTotalKernelTime;
        public uint TotalPageFaultCount;
        public uint TotalProcesses;
        public uint ActiveProcesses;
        public uint TotalTerminatedProcesses;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref ExtendedLimitInformation info, int length);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool QueryInformationJobObject(IntPtr job, int infoClass, out BasicAccountingInformation info, int length, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "CreateProcessW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CreateProcess(string application, StringBuilder commandLine, IntPtr processAttributes,
        IntPtr threadAttributes, [MarshalAs(UnmanagedType.Bool)] bool inheritHandles, uint flags,
        IntPtr environment, string directory, ref StartupInfoEx startup, out ProcessInformation process);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool InitializeProcThreadAttributeList(IntPtr list, int count, int flags, ref IntPtr size);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags, IntPtr attribute,
        IntPtr value, IntPtr size, IntPtr previous, IntPtr returned);
    [DllImport("kernel32.dll")]
    static extern void DeleteProcThreadAttributeList(IntPtr list);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool TerminateProcess(IntPtr process, uint exitCode);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int kind);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool DuplicateHandle(IntPtr sourceProcess, IntPtr source, IntPtr targetProcess,
        out IntPtr target, uint access, [MarshalAs(UnmanagedType.Bool)] bool inherit, uint options);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool CreatePipe(out IntPtr read, out IntPtr write, ref SecurityAttributes attributes, uint size);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetHandleInformation(IntPtr handle, uint mask, uint flags);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool WriteFile(IntPtr handle, byte[] data, int length, out int written, IntPtr overlapped);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "SearchPathW")]
    static extern uint SearchPath(string path, string file, string extension, uint length,
        StringBuilder result, IntPtr filePart);

    static Exception Failure(string operation) {
        return new IOException(operation + " failed (Windows error " + Marshal.GetLastWin32Error() + ")");
    }

    static string ReadString(BinaryReader input) {
        int length = input.ReadInt32();
        if (length < 0 || length > 1000000) throw new InvalidDataException("Invalid command field length");
        byte[] bytes = input.ReadBytes(checked(length * 2));
        if (bytes.Length != length * 2) throw new EndOfStreamException("Incomplete command field");
        return Encoding.Unicode.GetString(bytes);
    }

    static string ResolveProgram(string program) {
        if (Path.GetDirectoryName(program) != null && Path.GetDirectoryName(program).Length > 0)
            return program;
        var result = new StringBuilder(32768);
        uint length = SearchPath(null, program, ".exe", (uint)result.Capacity, result, IntPtr.Zero);
        if (length == 0 || length >= result.Capacity) throw Failure("SearchPath");
        return result.ToString();
    }

    static string Quote(string value) {
        var result = new StringBuilder("\"");
        int slashes = 0;
        foreach (char c in value) {
            if (c == '\\') { slashes++; continue; }
            if (c == '"') {
                result.Append('\\', slashes * 2 + 1);
                result.Append('"');
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

    static IntPtr InheritedStdHandle(int kind) {
        IntPtr process = GetCurrentProcess();
        IntPtr original = GetStdHandle(kind);
        IntPtr copy;
        if (!DuplicateHandle(process, original, process, out copy, 0, true, DuplicateSameAccess))
            throw Failure("DuplicateHandle");
        return copy;
    }

    static void SendFailure(string message) {
        byte[] bytes = Encoding.UTF8.GetBytes(message);
        Stream output = Console.OpenStandardOutput();
        output.WriteByte(0);
        byte[] length = BitConverter.GetBytes(bytes.Length);
        output.Write(length, 0, length.Length);
        output.Write(bytes, 0, bytes.Length);
        output.Flush();
    }

    static void SendCompletion(NamedPipeServerStream pipe, byte kind, byte[] payload) {
        pipe.WriteByte(kind);
        byte[] header = kind == 1 ? payload : BitConverter.GetBytes(payload.Length);
        pipe.Write(header, 0, header.Length);
        if (kind == 0) pipe.Write(payload, 0, payload.Length);
    }

    static void CopyInput(Stream source, IntPtr destination) {
        byte[] buffer = new byte[8192];
        try {
            int count;
            while ((count = source.Read(buffer, 0, buffer.Length)) > 0) {
                int offset = 0;
                while (offset < count) {
                    byte[] piece = new byte[count - offset];
                    Buffer.BlockCopy(buffer, offset, piece, 0, piece.Length);
                    int written;
                    if (!WriteFile(destination, piece, piece.Length, out written, IntPtr.Zero) || written == 0)
                        return;
                    offset += written;
                }
            }
        } catch (IOException) { }
        finally { CloseHandle(destination); }
    }

    public static int Run() {
        IntPtr job = IntPtr.Zero, inputRead = IntPtr.Zero, inputWrite = IntPtr.Zero;
        IntPtr output = IntPtr.Zero, error = IntPtr.Zero;
        IntPtr attributeList = IntPtr.Zero, jobValue = IntPtr.Zero;
        bool attributesInitialized = false;
        NamedPipeServerStream completion = null;
        IAsyncResult pendingConnection = null;
        ProcessInformation process = new ProcessInformation();
        bool ready = false;
        try {
            Stream source = Console.OpenStandardInput();
            var protocol = new BinaryReader(source, Encoding.Unicode);
            int ownerId = protocol.ReadInt32();
            string pipeName = ReadString(protocol);
            var owner = System.Diagnostics.Process.GetProcessById(ownerId);
            var ownerWatcher = new Thread(() => {
                owner.WaitForExit();
                Environment.Exit(1);
            });
            ownerWatcher.IsBackground = true;
            ownerWatcher.Start();
            completion = new NamedPipeServerStream(pipeName, PipeDirection.Out, 1,
                PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
            pendingConnection = completion.BeginWaitForConnection(null, null);
            string program = ReadString(protocol);
            string executable = ResolveProgram(program);
            int count = protocol.ReadInt32();
            if (count < 0 || count > 100000) throw new InvalidDataException("Invalid argument count");
            var commandLine = new StringBuilder(Quote(program));
            for (int i = 0; i < count; i++) commandLine.Append(' ').Append(Quote(ReadString(protocol)));

            job = CreateJobObject(IntPtr.Zero, null);
            if (job == IntPtr.Zero) throw Failure("CreateJobObject");
            var limits = new ExtendedLimitInformation();
            limits.Basic.LimitFlags = JobLimitKillOnClose;
            if (!SetInformationJobObject(job, 9, ref limits, Marshal.SizeOf(typeof(ExtendedLimitInformation))))
                throw Failure("SetInformationJobObject");

            var security = new SecurityAttributes();
            security.Length = Marshal.SizeOf(typeof(SecurityAttributes));
            security.InheritHandle = true;
            if (!CreatePipe(out inputRead, out inputWrite, ref security, 0)) throw Failure("CreatePipe");
            if (!SetHandleInformation(inputWrite, HandleFlagInherit, 0)) throw Failure("SetHandleInformation");
            output = InheritedStdHandle(-11);
            error = InheritedStdHandle(-12);
            IntPtr attributeSize = IntPtr.Zero;
            InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attributeSize);
            if (attributeSize == IntPtr.Zero) throw Failure("InitializeProcThreadAttributeList size");
            attributeList = Marshal.AllocHGlobal(attributeSize);
            if (!InitializeProcThreadAttributeList(attributeList, 1, 0, ref attributeSize))
                throw Failure("InitializeProcThreadAttributeList");
            attributesInitialized = true;
            jobValue = Marshal.AllocHGlobal(IntPtr.Size);
            Marshal.WriteIntPtr(jobValue, job);
            if (!UpdateProcThreadAttribute(attributeList, 0, new IntPtr(AttributeJobList),
                jobValue, new IntPtr(IntPtr.Size), IntPtr.Zero, IntPtr.Zero))
                throw Failure("UpdateProcThreadAttribute job list");
            var startup = new StartupInfoEx();
            startup.Startup.Size = Marshal.SizeOf(typeof(StartupInfoEx));
            startup.Startup.Flags = StartfUseStdHandles;
            startup.Startup.StdInput = inputRead;
            startup.Startup.StdOutput = output;
            startup.Startup.StdError = error;
            startup.AttributeList = attributeList;
            if (!CreateProcess(executable, commandLine, IntPtr.Zero, IntPtr.Zero, true,
                CreateSuspended | CreateNoWindow | ExtendedStartupInfoPresent, IntPtr.Zero, null,
                ref startup, out process))
                throw Failure("CreateProcess");
            Stream readyStream = Console.OpenStandardOutput();
            readyStream.WriteByte(1);
            readyStream.Flush();
            ready = true;
            completion.EndWaitForConnection(pendingConnection);
            if (ResumeThread(process.Thread) == Infinite) throw Failure("ResumeThread");
            CloseHandle(inputRead); inputRead = IntPtr.Zero;
            CloseHandle(output); output = IntPtr.Zero;
            CloseHandle(error); error = IntPtr.Zero;
            IntPtr writerHandle = inputWrite;
            inputWrite = IntPtr.Zero;
            var writer = new Thread(() => CopyInput(source, writerHandle));
            writer.IsBackground = true;
            try { writer.Start(); }
            catch { CloseHandle(writerHandle); throw; }

            if (WaitForSingleObject(process.Process, Infinite) != 0) throw Failure("WaitForSingleObject");
            uint exitCode;
            if (!GetExitCodeProcess(process.Process, out exitCode)) throw Failure("GetExitCodeProcess");
            BasicAccountingInformation accounting;
            do {
                if (!QueryInformationJobObject(job, 1, out accounting,
                    Marshal.SizeOf(typeof(BasicAccountingInformation)), IntPtr.Zero))
                    throw Failure("QueryInformationJobObject");
                if (accounting.ActiveProcesses > 0) Thread.Sleep(10);
            } while (accounting.ActiveProcesses > 0);
            SendCompletion(completion, 1, BitConverter.GetBytes(exitCode));
            return unchecked((int)exitCode);
        } catch (Exception ex) {
            if (!ready) SendFailure(ex.Message);
            else {
                try { SendCompletion(completion, 0, Encoding.UTF8.GetBytes(ex.Message)); }
                catch { }
            }
            return 1;
        } finally {
            if (process.Process != IntPtr.Zero) {
                if (!ready) TerminateProcess(process.Process, 1);
                CloseHandle(process.Process);
            }
            if (process.Thread != IntPtr.Zero) CloseHandle(process.Thread);
            if (inputRead != IntPtr.Zero) CloseHandle(inputRead);
            if (inputWrite != IntPtr.Zero) CloseHandle(inputWrite);
            if (output != IntPtr.Zero) CloseHandle(output);
            if (error != IntPtr.Zero) CloseHandle(error);
            if (attributesInitialized) DeleteProcThreadAttributeList(attributeList);
            if (attributeList != IntPtr.Zero) Marshal.FreeHGlobal(attributeList);
            if (jobValue != IntPtr.Zero) Marshal.FreeHGlobal(jobValue);
            if (job != IntPtr.Zero) CloseHandle(job);
            if (completion != null) completion.Dispose();
        }
    }
}
