    // Keep only a short tail while the Rust child paints the entire matrix.
    // The host reader otherwise stops when its 4 MiB fixture buffer fills.
    public static void WorkCards(string executable, string directory, string scenario) {
        using (var pty = new JecodeTestPty()) {
            pty.Resize(120, 40);
            pty.Start(executable, directory, scenario);
            var clock = System.Diagnostics.Stopwatch.StartNew();
            string summary = null;
            long exitedAt = -1;
            lock (pty.gate) {
                while (clock.ElapsedMilliseconds < 540000) {
                    int length = pty.received.Length;
                    int start = Math.Max(0, length - 65536);
                    string tail = pty.received.ToString(start, length - start);
                    if (tail.Contains("NATIVE_WORK_RESTORED")) {
                        string plain = System.Text.RegularExpressions.Regex.Replace(
                            tail, "\x1b\\[[0-?]*[ -/]*[@-~]", "");
                        var match = System.Text.RegularExpressions.Regex.Match(
                            plain, @"NATIVE_WORK cases=\d+ pages=\d+");
                        if (!match.Success) throw new IOException("ConPTY WORK summary missing");
                        summary = match.Value;
                        break;
                    }
                    if (length > 256 * 1024)
                        pty.received.Remove(0, length - 65536);
                    if (WaitForSingleObject(pty.process, 0) == 0) {
                        uint code;
                        Check(GetExitCodeProcess(pty.process, out code));
                        if (exitedAt < 0) exitedAt = clock.ElapsedMilliseconds;
                        if (clock.ElapsedMilliseconds - exitedAt > 5000) {
                            string diagnostic = System.Text.RegularExpressions.Regex.Replace(
                                tail, "\x1b\\[[0-?]*[ -/]*[@-~]", "");
                            if (diagnostic.Length > 2000)
                                diagnostic = diagnostic.Substring(diagnostic.Length - 2000);
                            throw new IOException("ConPTY WORK child exited " + code +
                                " before sentinel: " + diagnostic);
                        }
                    }
                    Monitor.Wait(pty.gate, 1000);
                }
            }
            if (summary == null) throw new IOException("ConPTY WORK matrix timed out");
            pty.Finish(true);
            Console.WriteLine(summary);
        }
    }
