// Window titles as ConPTY forwards them to the hosting terminal, such as
// Windows Terminal: the title protocol, not rendered conversation text.
public sealed partial class JecodeTestPty {
    const string CustomTitle = "JECODE_CUSTOM_TITLE";

    System.Collections.Generic.List<string> Titles() {
        var titles = new System.Collections.Generic.List<string>();
        var pattern = new System.Text.RegularExpressions.Regex("\x1b\\][02];([^\x07\x1b]*)(?:\x07|\x1b\\\\)");
        lock (gate) {
            foreach (System.Text.RegularExpressions.Match match in pattern.Matches(received.ToString()))
                titles.Add(match.Groups[1].Value);
        }
        return titles;
    }

    // From the title before the first session on, each TUI session shows Jecode
    // and gives that title back; nothing else is sent and it is the last title.
    // Fixtures past the shell sentinel set the custom title first. ConPTY sends
    // titles as it paints, so the count is a minimum; the native console title
    // checks cover every session exactly.
    public void EnsureTitles(string scenario, int sessions, bool custom) {
        var titles = Titles();
        string trace = scenario + ": " + string.Join(" / ", titles.ToArray());
        int first = titles.IndexOf("Jecode");
        if (first < 1) throw new IOException("ConPTY sent no Jecode title after a previous one: " + trace);
        string previous = titles[first - 1];
        if (custom && previous != CustomTitle) throw new IOException("ConPTY did not send the custom title first: " + trace);
        int shown = 0;
        for (int index = first; index < titles.Count; index++) {
            if (titles[index] == "Jecode") shown++;
            else if (titles[index] != previous) throw new IOException("ConPTY sent an unexpected title: " + trace);
        }
        if (shown < sessions) throw new IOException("ConPTY sent Jecode for fewer than " + sessions + " sessions: " + trace);
        if (titles[titles.Count - 1] != previous) throw new IOException("ConPTY did not restore the previous title: " + trace);
        Console.WriteLine("NATIVE_TITLES " + trace);
    }

    public void EnsureNoTitle(string scenario) {
        if (Titles().Contains("Jecode")) throw new IOException(scenario + " set the Jecode title");
        Console.WriteLine("NATIVE_TITLES " + scenario + ": no Jecode title");
    }

    public void EnsureTitleVariants() {
        var titles = Titles();
        if (!titles.Contains("Jecode") || titles[titles.Count - 1] != CustomTitle)
            throw new IOException("ConPTY did not finish the title variants with the custom title");
        Console.WriteLine("NATIVE_TITLES title-variants: empty, long, existing and changed titles restored");
    }
}
