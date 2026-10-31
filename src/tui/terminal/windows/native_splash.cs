public sealed partial class JecodeTestPty {
    public void Splash() {
        WaitFast("Ask anything");
        int full = Mark();
        Resize(100, 32);
        WaitFast("Tip  /permissions sets what Jecode may change here", full);
        WaitFast("✓ 1 skill   ✓ AGENTS.md", full);
        WaitFast("Recent in this folder", full);
        WaitFast("Fix the session expiry check in auth", full);
        WaitFast("/resume  all conversations", full);
        EnsureVt("JECODE_SPLASH_ACCENT", "splash accent");
        EnsureVt("JECODE_SPLASH_LIT", "splash caret");
        CaptureSplash("full");

        int medium = Mark();
        Resize(64, 20);
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
        WaitFast("Tip  /permissions", restored);

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
