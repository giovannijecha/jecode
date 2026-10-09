using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using Microsoft.Win32.SafeHandles;

// Owned adapter for the documented Windows console ABI. Rust remains safe code.
public static class JecodeConsole {
    [StructLayout(LayoutKind.Explicit, Size = 20)]
    public struct Record {
        [FieldOffset(0)] public ushort Type;
        [FieldOffset(4)] public int Down;
        [FieldOffset(8)] public ushort Repeat;
        [FieldOffset(10)] public ushort Key;
        [FieldOffset(14)] public ushort Character;
        [FieldOffset(16)] public uint Controls;
        [FieldOffset(8)] public uint MouseButtons;
        [FieldOffset(16)] public uint MouseFlags;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr GetStdHandle(int value);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleMode(IntPtr handle, out uint mode);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetConsoleMode(IntPtr handle, uint mode);
    [DllImport("kernel32.dll")]
    static extern uint GetConsoleOutputCP();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetConsoleOutputCP(uint codePage);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetNumberOfConsoleInputEvents(IntPtr handle, out uint count);
    [DllImport("kernel32.dll", EntryPoint = "ReadConsoleInputW", SetLastError = true)]
    static extern bool ReadConsoleInput(IntPtr handle, [Out] Record[] records, uint length, out uint count);
    [StructLayout(LayoutKind.Sequential)]
    struct ScreenInfo {
        public short Width, Height, CursorX, CursorY;
        public ushort Attributes;
        public short Left, Top, Right, Bottom, MaximumX, MaximumY;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetConsoleScreenBufferInfo(IntPtr handle, out ScreenInfo info);

    static void Check(bool success, string operation = "input") {
        if (!success) throw new IOException("Windows console " + operation + " error " + Marshal.GetLastWin32Error());
    }
    static int Modifiers(Record key) {
        return ((key.Controls & 3) != 0 ? 1 : 0) |
            ((key.Controls & 16) != 0 ? 2 : 0) | ((key.Controls & 12) != 0 ? 4 : 0);
    }
    public static List<Record> Keys(Record[] records, uint count) {
        var keys = new List<Record>();
        for (int index = 0; index < count; index++) AddKey(records[index], keys);
        return keys;
    }
    static bool KeyEvent(Record key) {
        // Unicode input may arrive on Alt release (including UTF-16 surrogate pairs).
        bool unicodeRelease = key.Down == 0 && key.Character != 0 && (key.Key == 18 || key.Key == 231);
        return key.Type == 1 && (key.Down != 0 || unicodeRelease) &&
            !((key.Key == 16 || key.Key == 17 || key.Key == 18) && key.Character == 0) &&
            // ConPTY may synthesize a pasted character as Alt+numpad digits,
            // then deliver the character on Alt release. Digits are not keys.
            !(key.Character == 0 && (key.Controls & 3) != 0 && key.Key >= 96 && key.Key <= 105);
    }
    static void AddKey(Record key, List<Record> keys) {
        if (!KeyEvent(key)) return;
        for (int repeat = 0; repeat < Math.Max(1, (int)key.Repeat); repeat++) keys.Add(key);
    }
    public static bool BufferedText(List<Record> keys, bool recentBurst) {
        int printable = 0;
        foreach (Record key in keys) {
            int modifiers = Modifiers(key) & 5;
            bool altGrText = modifiers == 5 && key.Character >= 32;
            if (key.Character == 0 || (modifiers != 0 && !altGrText)) return false;
            if (key.Character >= 32) printable++;
        }
        return keys.Count > 0 && (recentBurst || (keys.Count > 1 && printable > 0));
    }
    public static int Wheel(Record record) {
        return record.Type == 2 && record.MouseFlags == 4 ? (short)(record.MouseButtons >> 16) : 0;
    }
    static void WriteKeys(List<Record> keys, TextWriter writer, ref bool pasteBurst, ref long lastText, long now) {
        if (pasteBurst && now - lastText > 50) pasteBurst = false;
        if (BufferedText(keys, pasteBurst)) {
            // Some consoles discard paste markers; insert buffered text rather
            // than execute its newlines as prompts.
            var units = new List<string>();
            foreach (Record key in keys) units.Add(key.Character.ToString());
            writer.WriteLine("P|" + string.Join(",", units));
            pasteBurst = true; lastText = now;
        } else {
            foreach (Record key in keys) writer.WriteLine("K|" + key.Key + "|" + Modifiers(key) + "|" + key.Character);
        }
    }
    public static void Emit(Record[] records, uint count, TextWriter writer, ref bool pasteBurst, ref long lastText, ref int wheel, long now) {
        var keys = new List<Record>();
        for (int index = 0; index < count; index++) {
            int delta = Wheel(records[index]);
            if (delta != 0) {
                WriteKeys(keys, writer, ref pasteBurst, ref lastText, now); keys.Clear();
                wheel += delta;
                int rows = -(wheel / 120) * 3;
                wheel %= 120;
                if (rows != 0) writer.WriteLine("W|" + rows);
            } else { AddKey(records[index], keys); }
        }
        WriteKeys(keys, writer, ref pasteBurst, ref lastText, now);
    }
    // ReadConsoleInput can split one paste into several reads. Keep printable
    // records together until the burst is quiet, then emit one paste event.
    public sealed class Burst {
        const int Gap = 50;
        const int MaxUnits = 1024 * 1024 + 1;
        readonly List<Record> pending = new List<Record>();
        long lastText = -1000;
        int wheel;
        int printable;
        bool bulk;
        bool overflow;

        static bool Text(Record key) {
            int modifiers = Modifiers(key) & 5;
            return key.Character != 0 &&
                (modifiers == 0 || (modifiers == 5 && key.Character >= 32));
        }
        static void WriteKey(Record key, TextWriter writer) {
            writer.WriteLine("K|" + key.Key + "|" + Modifiers(key) + "|" + key.Character);
        }
        void WritePending(TextWriter writer) {
            if (overflow || pending.Count == MaxUnits) {
                // Do not pass a truncated bracketed paste to the decoder: its
                // closing marker may have been dropped with the suffix.
                writer.WriteLine("I|overflow");
            } else if (pending.Count > 1 && printable > 0) {
                var units = new List<string>(pending.Count);
                foreach (Record key in pending) units.Add(key.Character.ToString());
                writer.WriteLine("P|" + string.Join(",", units));
            } else {
                foreach (Record key in pending) WriteKey(key, writer);
            }
            pending.Clear();
            printable = 0;
            bulk = false;
            overflow = false;
        }
        public void Flush(TextWriter writer) {
            if (pending.Count == 0) return;
            // A lone typed character followed by Enter is a submission. If a
            // larger text batch preceded Enter, preserve it as paste content.
            if (!bulk && !overflow && pending.Count == 2 &&
                pending[0].Character >= 32 && pending[1].Key == 13 && pending[1].Character == 13) {
                Record enter = pending[1];
                pending.RemoveAt(1);
                WritePending(writer);
                WriteKey(enter, writer);
                return;
            }
            WritePending(writer);
        }
        public void Idle(TextWriter writer, long now) {
            if (pending.Count > 0 && now - lastText > Gap) Flush(writer);
        }
        public void Emit(Record[] records, uint count, TextWriter writer, long now) {
            Idle(writer, now);
            int batchPrintable = 0;
            for (int index = 0; index < count; index++) {
                Record key = records[index];
                int delta = Wheel(key);
                if (delta != 0) {
                    Flush(writer);
                    batchPrintable = 0;
                    wheel += delta;
                    int rows = -(wheel / 120) * 3;
                    wheel %= 120;
                    if (rows != 0) writer.WriteLine("W|" + rows);
                    continue;
                }
                if (!KeyEvent(key)) continue;
                bool text = Text(key);
                int repeat = Math.Max(1, (int)key.Repeat);
                for (int item = 0; item < repeat; item++) {
                    if (!text) {
                        Flush(writer);
                        batchPrintable = 0;
                        WriteKey(key, writer);
                    } else {
                        if (key.Character >= 32 && ++batchPrintable > 1) bulk = true;
                        if (pending.Count < MaxUnits) pending.Add(key);
                        else overflow = true;
                        if (key.Character >= 32) printable++;
                        lastText = now;
                    }
                }
            }
        }
    }
    public static void Restore(uint inputMode, uint outputMode, uint codePage) {
        using (var input = CreateFileW("CONIN$", 0xc0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero))
        using (var output = CreateFileW("CONOUT$", 0xc0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero)) {
            IntPtr outputHandle = output.DangerousGetHandle();
            SetConsoleMode(outputHandle, outputMode | 5u);
            try {
                Console.Out.Write("\x1b[0m\x1b[?2026l\x1b[?1006l\x1b[?1000l\x1b[?2004l\x1b[?1049l\x1b[?25h");
                Console.Out.Flush();
            } finally {
                Check(SetConsoleMode(input.DangerousGetHandle(), inputMode), "restore input mode");
                Check(SetConsoleMode(outputHandle, outputMode), "restore output mode");
                Check(SetConsoleOutputCP(codePage), "restore output encoding");
            }
        }
    }
    public static void Run(Stream pipe, StreamWriter writer) {
        using (SafeFileHandle console = CreateFileW("CONIN$", 0xc0000000u, 3, IntPtr.Zero, 3, 0, IntPtr.Zero)) {
            Check(!console.IsInvalid);
            IntPtr input = console.DangerousGetHandle(), output = GetStdHandle(-11);
            uint inputMode, outputMode;
            Check(GetConsoleMode(input, out inputMode), "read input mode");
            Check(GetConsoleMode(output, out outputMode), "read output mode");
            uint codePage = GetConsoleOutputCP();
            writer.WriteLine("R|" + inputMode + "|" + outputMode + "|" + codePage);
            try {
                // No line buffering, echo, Ctrl+C processing, quick edit or VT key conversion.
                Check(SetConsoleMode(input, (inputMode & ~0x247u) | 0x98u));
                // Delay wrapping so painting the bottom-right cell cannot scroll the screen.
                Check(SetConsoleMode(output, outputMode | 0x0du));
                Check(SetConsoleOutputCP(65001), "set UTF-8 output");
                Console.Out.Write("\x1b[?1049h\x1b[?2004h\x1b[?1000h\x1b[?1006h\x1b[?25l");
                Console.Out.Flush();
                byte[] stop = new byte[1];
                var stopping = pipe.ReadAsync(stop, 0, 1);
                Record[] records = new Record[4096];
                string size = "";
                var burst = new Burst();
                var clock = System.Diagnostics.Stopwatch.StartNew();
                while (!stopping.IsCompleted) {
                    ScreenInfo info;
                    Check(GetConsoleScreenBufferInfo(output, out info), "measure screen");
                    string current = (info.Right - info.Left + 1) + "|" + (info.Bottom - info.Top + 1);
                    if (current != size) {
                        size = current;
                        // One native snapshot and one event: dimensions and position cannot split.
                        writer.WriteLine("S|" + size + "|" + Math.Max(0, info.CursorY - info.Top) + "|" + Math.Max(0, info.CursorX - info.Left));
                    }
                    uint available;
                    Check(GetNumberOfConsoleInputEvents(input, out available));
                    if (available > 0) {
                        uint count;
                        Check(ReadConsoleInput(input, records, (uint)records.Length, out count));
                        burst.Emit(records, count, writer, clock.ElapsedMilliseconds);
                    } else burst.Idle(writer, clock.ElapsedMilliseconds);
                    Thread.Sleep(10);
                }
                burst.Flush(writer);
            } finally {
                try {
                    Console.Out.Write("\x1b[0m\x1b[?2026l\x1b[?1006l\x1b[?1000l\x1b[?2004l\x1b[?1049l\x1b[?25h");
                    Console.Out.Flush();
                } finally {
                    SetConsoleMode(input, inputMode);
                    SetConsoleMode(output, outputMode);
                    SetConsoleOutputCP(codePage);
                }
            }
        }
    }
}
