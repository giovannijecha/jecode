using System;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

public static class JecodeConsoleProbe {
    public static void Main(string[] args) {
        // `title TEXT` sets a custom console title, as a shell or user would.
        if (args.Length == 2 && args[0] == "title") { Check(SetConsoleTitleW(args[1])); return; }
        byte[] bytes = System.Text.Encoding.UTF8.GetBytes(Snapshot());
        using (var output = Console.OpenStandardOutput()) {
            output.Write(bytes, 0, bytes.Length);
        }
    }
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
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetConsoleTitleW([Out, MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.U2)] char[] title, uint size);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SetConsoleTitleW(string title);

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
            // A third line, after the visible cells, holds the cursor relative to the window;
            // a fourth holds the console title.
            var title = new char[65536];
            uint length = GetConsoleTitleW(title, (uint)title.Length);
            return inputMode + "|" + outputMode + "|" + GetConsoleOutputCP() + "\n" + new string(text, 0, (int)read)
                + "\n" + screen.X + "," + (screen.Y - screen.Top) + "\n" + new string(title, 0, (int)length);
        }
    }
}
