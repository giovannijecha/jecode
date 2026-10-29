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
        Send("3");
        Wait("Native second read complete.");
        Send("kept draft");
        Wait("kept draft");
        Send("\x11");
        Wait("NATIVE_PERMISSIONS_RESTORED");
        Finish(true);
    }
}
