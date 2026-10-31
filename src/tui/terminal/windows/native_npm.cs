public sealed partial class JecodeTestPty {
    public static void RunNpm(string node, string launcher, string directory) {
        using (var pty = new JecodeTestPty()) {
            pty.Start(node, directory, "npm-launcher", launcher);
            pty.Wait("Ask anything");
            pty.Wait("No configured provider");
            int draft = pty.Mark(); pty.Send("npm draft");
            pty.Wait("npm draft", draft);
            pty.Resize(40, 12);
            pty.Resize(80, 24);
            int cleared = pty.Mark(); pty.Send("\x03");
            pty.Wait("Ask anything", cleared);
            pty.Send("kept after resize");
            pty.Wait("kept after resize");
            pty.Send("\x11");
            pty.Wait("jecode closed");
            pty.Finish(true);
            pty.AssertAlternateScreenRestored();
        }
    }

    void AssertAlternateScreenRestored() {
        lock (gate) {
            string text = received.ToString();
            int entered = Count(text, "\x1b[?1049h");
            int left = Count(text, "\x1b[?1049l");
            if (entered == 0 || entered != left)
                throw new IOException("npm launcher did not restore alternate screen: " + entered + " entries, " + left + " exits");
        }
    }

    static int Count(string text, string needle) {
        int count = 0, offset = 0;
        while ((offset = text.IndexOf(needle, offset, StringComparison.Ordinal)) >= 0) {
            count++; offset += needle.Length;
        }
        return count;
    }
}
