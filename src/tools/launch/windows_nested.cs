using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

// Disposable native fixture: inner breakaway is allowed, outer breakaway is
// forbidden. A child using CREATE_BREAKAWAY_FROM_JOB can still remain in outer.
public static partial class JecodeLaunch {
    const uint JobKillOnClose = 0x00002000;
    const uint JobBreakawayOk = 0x00000800;

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

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool SetInformationJobObject(IntPtr job, int infoClass,
        ref ExtendedLimitInformation info, int length);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    static int NestedFixture(string executable, string testName) {
        IntPtr outer = IntPtr.Zero, inner = IntPtr.Zero;
        var child = new ProcessInformation();
        try {
            outer = CreateJobObject(IntPtr.Zero, null);
            inner = CreateJobObject(IntPtr.Zero, null);
            if (outer == IntPtr.Zero || inner == IntPtr.Zero) throw new IOException("CreateJobObject failed: " + Error());
            var limits = new ExtendedLimitInformation();
            limits.Basic.LimitFlags = 0;
            if (!SetInformationJobObject(outer, 9, ref limits, Marshal.SizeOf(typeof(ExtendedLimitInformation))))
                throw new IOException("SetInformationJobObject outer failed: " + Error());
            limits.Basic.LimitFlags = JobKillOnClose | JobBreakawayOk;
            if (!SetInformationJobObject(inner, 9, ref limits, Marshal.SizeOf(typeof(ExtendedLimitInformation))))
                throw new IOException("SetInformationJobObject inner failed: " + Error());
            if (!AssignProcessToJobObject(outer, GetCurrentProcess()))
                throw new IOException("AssignProcessToJobObject outer failed: " + Error());
            var startup = new StartupInfo();
            startup.Size = Marshal.SizeOf(typeof(StartupInfo));
            startup.Flags = StartfUseStdHandles;
            startup.StdInput = GetStdHandle(-10);
            startup.StdOutput = GetStdHandle(-11);
            startup.StdError = GetStdHandle(-12);
            var command = new StringBuilder(JecodeWindowsCommand.Quote(executable));
            command.Append(" --exact ").Append(JecodeWindowsCommand.Quote(testName)).Append(" --nocapture");
            if (!CreateProcess(executable, command, IntPtr.Zero, IntPtr.Zero, true, CreateSuspended,
                IntPtr.Zero, null, ref startup, out child))
                throw new IOException("CreateProcess nested fixture failed: " + Error());
            if (!AssignProcessToJobObject(inner, child.Process))
                throw new IOException("AssignProcessToJobObject inner failed: " + Error());
            if (ResumeThread(child.Thread) == Infinite)
                throw new IOException("ResumeThread nested fixture failed: " + Error());
            uint wait = WaitForSingleObject(child.Process, 20000);
            if (wait != WaitObject0) throw new IOException("Nested fixture timed out or could not be observed");
            uint code;
            if (!GetExitCodeProcess(child.Process, out code))
                throw new IOException("GetExitCodeProcess nested fixture failed: " + Error());
            Console.WriteLine("nested job fixture exited " + code);
            return unchecked((int)code);
        } catch (Exception error) {
            Console.Error.WriteLine(error.Message);
            return 1;
        } finally {
            if (child.Process != IntPtr.Zero) {
                if (WaitForSingleObject(child.Process, 0) != WaitObject0)
                    TerminateProcess(child.Process, 1);
                CloseHandle(child.Process);
            }
            if (child.Thread != IntPtr.Zero) CloseHandle(child.Thread);
            if (inner != IntPtr.Zero) CloseHandle(inner);
            if (outer != IntPtr.Zero) CloseHandle(outer);
        }
    }
}
