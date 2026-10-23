using System;
using System.Globalization;
using System.Runtime.InteropServices;

// Owned Windows boundary for the free space available to this process at an
// actual archive directory. The API follows mounted volumes and UNC shares.
public static class JecodeCheckpointSpace {
    [DllImport("kernel32.dll", EntryPoint = "GetDiskFreeSpaceExW",
        CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetDiskFreeSpaceEx(
        string directory,
        out ulong available,
        out ulong capacity,
        out ulong totalFree);

    public static int Main(string[] args) {
        if (args.Length != 1) return 2;
        // UNC share roots require a trailing separator, and ordinary archive
        // directories accept it too.
        string directory = args[0];
        if (!directory.EndsWith("\\") && !directory.EndsWith("/"))
            directory += "\\";
        ulong available, capacity, totalFree;
        if (!GetDiskFreeSpaceEx(directory, out available, out capacity, out totalFree)) {
            Console.Out.WriteLine("Windows error " + Marshal.GetLastWin32Error());
            return 3;
        }
        Console.Out.WriteLine(available.ToString(CultureInfo.InvariantCulture));
        return 0;
    }
}
