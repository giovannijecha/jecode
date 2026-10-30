// Strict reader for the fixed JSON object that Rust encodes for a confined
// command: string keys whose values are booleans or arrays of strings. It
// avoids loading a general serializer assembly on every supervised start.
internal sealed class JecodeWindowsSandboxPolicy {
    readonly string text;
    int position;

    JecodeWindowsSandboxPolicy(string text) { this.text = text; }

    public static System.Collections.Generic.Dictionary<string, object> Read(string text) {
        var reader = new JecodeWindowsSandboxPolicy(text);
        var policy = new System.Collections.Generic.Dictionary<string, object>();
        reader.Expect('{');
        if (!reader.Take('}')) {
            do {
                string key = reader.String();
                reader.Expect(':');
                if (policy.ContainsKey(key)) throw Invalid();
                policy[key] = reader.Value();
            } while (reader.Take(','));
            reader.Expect('}');
        }
        reader.Skip();
        if (reader.position != text.Length) throw Invalid();
        return policy;
    }

    object Value() {
        Skip();
        if (Literal("true")) return true;
        if (Literal("false")) return false;
        var values = new System.Collections.Generic.List<string>();
        Expect('[');
        if (!Take(']')) {
            do values.Add(String()); while (Take(','));
            Expect(']');
        }
        return values;
    }

    string String() {
        Expect('"');
        var value = new System.Text.StringBuilder();
        while (true) {
            if (position >= text.Length) throw Invalid();
            char character = text[position++];
            if (character == '"') return value.ToString();
            if (character < ' ') throw Invalid();
            if (character != '\\') { value.Append(character); continue; }
            if (position >= text.Length) throw Invalid();
            char escape = text[position++];
            switch (escape) {
                case '"': case '\\': case '/': value.Append(escape); break;
                case 'b': value.Append('\b'); break;
                case 'f': value.Append('\f'); break;
                case 'n': value.Append('\n'); break;
                case 'r': value.Append('\r'); break;
                case 't': value.Append('\t'); break;
                case 'u':
                    int unit;
                    if (position + 4 > text.Length || !int.TryParse(text.Substring(position, 4),
                        System.Globalization.NumberStyles.AllowHexSpecifier,
                        System.Globalization.CultureInfo.InvariantCulture, out unit))
                        throw Invalid();
                    value.Append((char)unit);
                    position += 4;
                    break;
                default: throw Invalid();
            }
        }
    }

    void Skip() {
        while (position < text.Length && (text[position] == ' ' || text[position] == '\t' ||
            text[position] == '\n' || text[position] == '\r'))
            position++;
    }

    bool Take(char expected) {
        Skip();
        if (position >= text.Length || text[position] != expected) return false;
        position++;
        return true;
    }

    void Expect(char expected) {
        if (!Take(expected)) throw Invalid();
    }

    bool Literal(string word) {
        if (System.String.CompareOrdinal(text, position, word, 0, word.Length) != 0) return false;
        position += word.Length;
        return true;
    }

    static System.Exception Invalid() {
        return new System.IO.InvalidDataException("Invalid Windows sandbox policy");
    }
}
