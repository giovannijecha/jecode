// Wheel input through the production Rust ConPTY renderer over a resumed
// mixed conversation. Compiled only for the explicit scroll measurement.
public sealed partial class JecodeTestPty {
    // ConPTY repaints only changed cells, so frames are recognized on a
    // minimal screen model: cursor moves, erases, scrolls and printable cells.
    char[][] screen;
    int row, column, parsed, marginTop, marginBottom;

    void Screen(int width, int height) {
        screen = new char[height][];
        for (int line = 0; line < height; line++) screen[line] = new string(' ', width).ToCharArray();
        marginTop = 0; marginBottom = height - 1;
    }
    void Clear(int line, int from, int to) {
        for (int cell = Math.Max(0, from); cell < Math.Min(to, screen[line].Length); cell++) screen[line][cell] = ' ';
    }
    void ScrollRegion(int amount) {
        for (int step = 0; step < Math.Abs(amount); step++) {
            if (amount > 0) {
                for (int line = marginTop; line < marginBottom; line++) screen[line] = screen[line + 1];
                screen[marginBottom] = new string(' ', screen[0].Length).ToCharArray();
            } else {
                for (int line = marginBottom; line > marginTop; line--) screen[line] = screen[line - 1];
                screen[marginTop] = new string(' ', screen[0].Length).ToCharArray();
            }
        }
    }
    void Csi(string parameters, char final) {
        if (parameters.StartsWith("?") || parameters.StartsWith(">")) return;
        string[] parts = parameters.Split(';');
        int first = parts[0].Length > 0 ? int.Parse(parts[0]) : 0, count = Math.Max(1, first);
        int second = parts.Length > 1 && parts[1].Length > 0 ? int.Parse(parts[1]) : 0;
        int width = screen[0].Length, height = screen.Length;
        switch (final) {
            case 'H': case 'f': row = Math.Max(1, first) - 1; column = Math.Max(1, second) - 1; break;
            case 'A': row -= count; break;
            case 'B': row += count; break;
            case 'C': column += count; break;
            case 'D': column -= count; break;
            case 'G': column = count - 1; break;
            case 'd': row = count - 1; break;
            case 'K':
                if (first == 0) Clear(row, column, width);
                else if (first == 1) Clear(row, 0, column + 1);
                else Clear(row, 0, width);
                break;
            case 'X': Clear(row, column, column + count); break;
            case 'J':
                if (first == 2 || first == 3) for (int line = 0; line < height; line++) Clear(line, 0, width);
                break;
            case 'S': ScrollRegion(count); break;
            case 'T': ScrollRegion(-count); break;
            case 'r':
                marginTop = Math.Max(1, first) - 1;
                marginBottom = (second > 0 ? second : height) - 1;
                row = 0; column = 0;
                break;
        }
        row = Math.Max(0, Math.Min(row, height - 1));
        column = Math.Max(0, Math.Min(column, width));
    }
    // Applies output received since the last call. The caller holds the gate.
    void Apply() {
        string text = received.ToString(parsed, received.Length - parsed);
        int index = 0;
        while (index < text.Length) {
            char character = text[index];
            if (character == '\x1b') {
                if (index + 1 >= text.Length) break;
                char next = text[index + 1];
                if (next == '[') {
                    int end = index + 2;
                    while (end < text.Length && (text[end] < '@' || text[end] > '~')) end++;
                    if (end >= text.Length) break;
                    Csi(text.Substring(index + 2, end - index - 2), text[end]);
                    index = end + 1;
                } else if (next == ']') {
                    int bell = text.IndexOf('\x07', index), terminator = text.IndexOf("\x1b\\", index + 2);
                    int end = bell < 0 ? terminator : terminator < 0 ? bell : Math.Min(bell, terminator);
                    if (end < 0) break;
                    index = end + (end == bell ? 1 : 2);
                } else if (next == 'M') {
                    if (row == marginTop) ScrollRegion(-1); else row = Math.Max(0, row - 1);
                    index += 2;
                } else index += 2;
                continue;
            }
            if (character == '\r') column = 0;
            else if (character == '\n') { if (row == marginBottom) ScrollRegion(1); else row = Math.Min(row + 1, screen.Length - 1); }
            else if (character == '\b') column = Math.Max(0, column - 1);
            else if (character >= ' ') {
                // ConPTY relies on autowrap for long runs.
                if (column >= screen[row].Length) {
                    column = 0;
                    if (row == marginBottom) ScrollRegion(1); else row = Math.Min(row + 1, screen.Length - 1);
                }
                screen[row][column] = character;
                column++;
            }
            index++;
        }
        parsed += index;
    }
    // The reading row's "N rows below" counter, or -1 while following.
    int Counter() {
        foreach (char[] line in screen) {
            string text = new string(line);
            int at = text.IndexOf("↓ ");
            if (at < 0) continue;
            int end = at + 2;
            while (end < text.Length && char.IsDigit(text[end])) end++;
            if (end > at + 2 && text.Substring(end).StartsWith(" row")) return int.Parse(text.Substring(at + 2, end - at - 2));
        }
        return -1;
    }
    int WaitCounter(int minimum) {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        lock (gate) {
            while (clock.ElapsedMilliseconds < 20000) {
                Apply();
                int value = Counter();
                if (value >= minimum) return value;
                Monitor.Wait(gate, 100);
            }
            var rows = new StringBuilder();
            foreach (char[] line in screen) rows.Append(new string(line).TrimEnd()).Append('\n');
            throw new IOException("ConPTY screen did not reach " + minimum + " rows below:\n" + rows);
        }
    }
    static void Jitter(Random random, double unit) {
        long due = System.Diagnostics.Stopwatch.GetTimestamp() + (long)(random.NextDouble() * 20000 * unit);
        while (System.Diagnostics.Stopwatch.GetTimestamp() < due) Thread.SpinWait(20);
    }
    // Returns the last arrival once ConPTY has been silent for the interval,
    // so one repaint's trailing output cannot leak into the next sample.
    long Quiet(int milliseconds) {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        while (clock.ElapsedMilliseconds < 20000) {
            long last;
            lock (gate) { last = lastReceived; }
            Thread.Sleep(milliseconds);
            lock (gate) { if (lastReceived == last) return last; }
        }
        throw new IOException("ConPTY output did not settle within 20 seconds");
    }
    static void PrintSamples(string name, double[] samples) {
        double[] sorted = (double[])samples.Clone();
        Array.Sort(sorted);
        var culture = System.Globalization.CultureInfo.InvariantCulture;
        int p95 = (int)Math.Ceiling(sorted.Length * 0.95) - 1;
        Console.WriteLine(name + "_US=" + string.Join(",", Array.ConvertAll(samples, value => value.ToString("F0", culture))));
        Console.WriteLine(name + "_P50_P95_MAX_US=" + sorted[sorted.Length / 2].ToString("F0", culture) + "," +
            sorted[p95].ToString("F0", culture) + "," + sorted[sorted.Length - 1].ToString("F0", culture));
    }
    // The reading row's counter identifies each step's frame on the screen
    // model; settled time is the last output byte of that repaint.
    void ScrollLatency(string scenario) {
        WaitFast("FINAL-ANSWER-MARK");
        Quiet(200);
        const string up = "\x1b[<64;10;5M";
        double unit = System.Diagnostics.Stopwatch.Frequency / 1000000.0;
        int below = 0;
        var steps = new System.Collections.Generic.List<string>();
        // A random start breaks phase lock with the system timer tick, which
        // the quiet wait and ConPTY painting both follow.
        var random = new Random(7);
        double[] seen = new double[60], settled = new double[60];
        for (int index = -3; index < seen.Length; index++) {
            Jitter(random, unit);
            long sent = System.Diagnostics.Stopwatch.GetTimestamp();
            Send(up);
            int value = WaitCounter(below + 1);
            long first = System.Diagnostics.Stopwatch.GetTimestamp();
            if (value != below + 3) throw new IOException("Wheel notch did not move exactly three rows");
            steps.Add((value - below).ToString());
            below = value;
            long last = Quiet(40);
            if (index < 0) continue;
            seen[index] = (first - sent) / unit;
            settled[index] = (last - sent) / unit;
        }
        // Ten notches 4 ms apart, as a quick wheel flick; times from the last.
        double[] flickSeen = new double[20], flickSettled = new double[20];
        for (int index = 0; index < flickSeen.Length; index++) {
            long sent = 0;
            Jitter(random, unit);
            for (int notch = 0; notch < 10; notch++) {
                long due = System.Diagnostics.Stopwatch.GetTimestamp() + (long)(4000 * unit);
                while (notch > 0 && System.Diagnostics.Stopwatch.GetTimestamp() < due) Thread.SpinWait(20);
                Send(up);
                sent = System.Diagnostics.Stopwatch.GetTimestamp();
            }
            int value = WaitCounter(below + 30);
            long first = System.Diagnostics.Stopwatch.GetTimestamp();
            if (value != below + 30) throw new IOException("Wheel flick did not move exactly thirty rows");
            steps.Add("f" + (value - below));
            below = value;
            long last = Quiet(40);
            flickSeen[index] = (first - sent) / unit;
            flickSettled[index] = (last - sent) / unit;
        }
        string name = "NATIVE_SCROLL_" + scenario.Substring("scroll-latency-".Length);
        PrintSamples(name + "_STEP_SEEN", seen);
        PrintSamples(name + "_STEP_SETTLED", settled);
        PrintSamples(name + "_FLICK_SEEN", flickSeen);
        PrintSamples(name + "_FLICK_SETTLED", flickSettled);
        Console.WriteLine(name + "_ROWS_PER_STEP=" + string.Join(",", steps));
        Send("\x11");
        WaitFast("NATIVE_LATENCY_RESTORED");
        Finish(true);
    }
    public static void RunScroll(string executable, string directory, string scenario) {
        using (var pty = new JecodeTestPty()) {
            string[] size = scenario.Substring("scroll-latency-".Length).Split('x');
            short width = short.Parse(size[0]), height = short.Parse(size[1]);
            pty.Resize(width, height);
            pty.Screen(width, height);
            pty.Start(executable, directory, scenario);
            pty.ScrollLatency(scenario);
        }
    }
}
