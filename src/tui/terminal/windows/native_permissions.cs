// Real permission menu and request states through the production Rust ConPTY renderer.
public sealed partial class JecodeTestPty {
    void Command(string command) {
        Send(command);
        Thread.Sleep(120);
        Send("\r");
    }

    void Permissions() {
        Wait("Ask anything");
        Command("/permissions");
        Wait("Permissions");
        Wait("this conversation");
        int narrowed = Mark();
        Resize(44, 12);
        Wait("Permissions", narrowed);
        Send("1");
        Wait("Read");
        Resize(80, 24);

        Command("/settings");
        Wait("Default permissions");
        int defaults = Mark();
        Send("4");
        WaitFast("Read and analyze; no project changes", defaults);
        int saved = Mark();
        Send("3");
        WaitFast("this conversation stays Read", saved);
        Send("\x1b");
        Thread.Sleep(200);

        int level = Mark();
        Command("/permissions");
        WaitFast("new conversations use Full", level);
        int applied = Mark();
        Send("2");
        WaitFast("this conversation", applied);
        Command("read outside");
        Wait("Waiting for permission");
        Wait("Read access outside this folder");
        Wait("Approve and remember");
        Send("2");
        Wait("Native first read complete.");

        int remembered = Mark();
        Command("/permissions");
        WaitFast("1 for this conversation", remembered);
        int grants = Mark();
        Send("4");
        WaitFast("outside-note.txt", grants);
        Send("\x04");
        Wait("Nothing remembered");
        Send("\x1b");
        Thread.Sleep(180);
        Send("\x1b");
        Thread.Sleep(180);

        int request = Mark();
        Command("read outside again");
        Wait("Waiting for permission", request);
        Wait("Reads of the outside files above", request);
        WaitWrappedFile("outside-other.txt", request);
        Send("3");
        Wait("Native second read complete.");

        // A call too long for the dock pages to its end; the choices still decide.
        int paged = Mark();
        Command("run the long command");
        Wait("PgUp/PgDn", paged);
        Wait("native-step-01", paged);
        for (int page = 0; page < 5; page++) {
            Send("\x1b[6~");
            Thread.Sleep(120);
        }
        Wait("native-step-40", paged);
        Send("3");
        Wait("nothing ran; the agent was told", paged);
        Wait("Native long command denied.", paged);
        Send("kept draft");
        Wait("kept draft");
        Send("\x11");
        Wait("NATIVE_PERMISSIONS_RESTORED");
        Finish(true);
    }

    // One filename can straddle two rows when the fixture path grows. Verify
    // every character in order while allowing only that single row break.
    void WaitWrappedFile(string filename, int after) {
        var alternatives = new System.Collections.Generic.List<string> {
            System.Text.RegularExpressions.Regex.Escape(filename)
        };
        for (int split = 1; split < filename.Length; split++) {
            alternatives.Add(
                System.Text.RegularExpressions.Regex.Escape(filename.Substring(0, split)) +
                @"\s+" +
                System.Text.RegularExpressions.Regex.Escape(filename.Substring(split))
            );
        }
        var pattern = new System.Text.RegularExpressions.Regex(
            string.Join("|", alternatives),
            System.Text.RegularExpressions.RegexOptions.CultureInvariant
        );
        var clock = System.Diagnostics.Stopwatch.StartNew();
        lock (gate) {
            while (clock.ElapsedMilliseconds < 20000) {
                string plain = System.Text.RegularExpressions.Regex.Replace(
                    received.ToString(after, received.Length - after),
                    "\x1b\\[[0-?]*[ -/]*[@-~]", ""
                );
                if (pattern.IsMatch(plain)) return;
                long remaining = 20000 - clock.ElapsedMilliseconds;
                if (remaining <= 0) break;
                Monitor.Wait(gate, (int)remaining);
            }
            int length = Math.Min(received.Length, 2000);
            throw new IOException("ConPTY did not show wrapped file " + filename + ": " +
                received.ToString(received.Length - length, length));
        }
    }
}
