using System;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public static class JecodeConsoleProbe {
    [StructLayout(LayoutKind.Sequential)]
    struct Coord { public short X, Y; public Coord(short x, short y) { X = x; Y = y; } }
    [StructLayout(LayoutKind.Sequential)]
    struct Screen {
        public short Width, Height, X, Y;
        public ushort Attributes;
        public short Left, Top, Right, Bottom, MaximumX, MaximumY;
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleMode(SafeFileHandle handle, out uint mode);
    [DllImport("kernel32.dll")]
    static extern uint GetConsoleOutputCP();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleScreenBufferInfo(SafeFileHandle handle, out Screen screen);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool ReadConsoleOutputCharacterW(SafeFileHandle handle, [Out, MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.U2, SizeParamIndex = 2)] char[] text, uint count, Coord position, out uint read);

    static void Check(bool result) { if (!result) throw new IOException("Console probe error " + Marshal.GetLastWin32Error()); }
    public static string Snapshot() {
        using (var input = CreateFileW("CONIN$", 0xc0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero))
        using (var output = CreateFileW("CONOUT$", 0xc0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero)) {
            uint inputMode, outputMode, read;
            Check(GetConsoleMode(input, out inputMode)); Check(GetConsoleMode(output, out outputMode));
            Screen screen; Check(GetConsoleScreenBufferInfo(output, out screen));
            int count = screen.Width * (screen.Bottom - screen.Top + 1);
            var text = new char[count];
            Check(ReadConsoleOutputCharacterW(output, text, (uint)count, new Coord(0, screen.Top), out read));
            return inputMode + "|" + outputMode + "|" + GetConsoleOutputCP() + "\n" + new string(text, 0, (int)read);
        }
    }
}
