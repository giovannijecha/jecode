public sealed partial class JecodeTestPty {
    public void Splash() {
        WaitFast("Ask anything");
        int full = Mark();
        Resize(100, 32);
        WaitFast("/ commands   Alt+V image   Ctrl+Q quit", full);
        WaitFast("✓ 1 skill   ✓ AGENTS.md", full);
        WaitFast("Recent in this folder", full);
        WaitFast("Fix the session expiry check in auth", full);
        WaitFast("/resume all", full);
        EnsureVt("JECODE_SPLASH_ACCENT", "splash accent");
        EnsureVt("JECODE_SPLASH_LIT", "splash caret");
        CaptureSplash("full");

        int medium = Mark();
        Resize(64, 16);
        WaitFast("▄", medium);
        WaitFast("Recent in this folder", medium);
        CaptureSplash("compact");

        int narrow = Mark();
        Resize(48, 16);
        WaitFast(">_ jecode", narrow);
        WaitFast("Recent in this folder", narrow);
        CaptureSplash("narrow");

        int restored = Mark();
        Resize(100, 32);
        WaitFast("/ commands", restored);

        // The commands key arrives as a key, also in the same write as typed text.
        int commands = Mark();
        Send("burst draft\x07");
        WaitFast("draft kept", commands);
        WaitFast("JECODE_COMMAND_COUNT commands", commands);
        int filtered = Mark();
        Send("mod");
        WaitFast("1 of JECODE_COMMAND_COUNT commands", filtered);
        int returned = Mark();
        Send("\x1b");
        WaitFast("Ctrl+G commands", returned);
        WaitFast("burst draft", returned);
        Send("\x15");
        Thread.Sleep(150);

        int prompted = Mark();
        Send("first prompt");
        WaitFast("first prompt", prompted);
        Send("\r");
        WaitFast("Fixture answered.", prompted);
        WaitFast("Done", prompted);
        WaitFast(">_ jecode", prompted);
        CaptureSplash("after");
        Send("\x11");
        WaitFast("NATIVE_STARTUP_RESTORED");
        Finish(true);
    }

    void CaptureSplash(string stage) {
        var prefix = Environment.GetEnvironmentVariable("JECODE_STARTUP_CAPTURE");
        if (!String.IsNullOrEmpty(prefix)) {
            lock (gate) File.WriteAllText(prefix + "." + stage + ".vt", received.ToString());
        }
    }
}
