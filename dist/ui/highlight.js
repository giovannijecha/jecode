// Colouring a fenced code block.
//
// Not a parser and not trying to be one: a code block on screen is read, not
// compiled, and what a reader needs is the shape — where the strings end, what
// is a comment, which words are the language's own. Five roles carry all of
// that, and a language nobody here knows gets no colour at all rather than
// colour that is wrong, because wrong highlighting reads as a wrong program.
const FAMILIES = {
    ts: "clike",
    tsx: "clike",
    typescript: "clike",
    js: "clike",
    jsx: "clike",
    javascript: "clike",
    mjs: "clike",
    java: "clike",
    c: "clike",
    cpp: "clike",
    cs: "clike",
    go: "clike",
    rust: "clike",
    rs: "clike",
    swift: "clike",
    php: "clike",
    py: "python",
    python: "python",
    sh: "shell",
    bash: "shell",
    zsh: "shell",
    shell: "shell",
    console: "shell",
    ps1: "shell",
    powershell: "shell",
    json: "json",
    jsonc: "json",
};
const KEYWORDS = {
    clike: new Set([
        "abstract", "as", "async", "await", "break", "case", "catch", "class", "const",
        "continue", "declare", "default", "delete", "do", "else", "enum", "export",
        "extends", "false", "finally", "for", "from", "func", "function", "get", "if",
        "implements", "import", "in", "instanceof", "interface", "let", "new", "null",
        "of", "package", "private", "protected", "public", "readonly", "return", "set",
        "static", "struct", "super", "switch", "this", "throw", "true", "try", "type",
        "typeof", "undefined", "var", "void", "while", "yield",
    ]),
    python: new Set([
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
        "del", "elif", "else", "except", "False", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass",
        "raise", "return", "True", "try", "while", "with", "yield",
    ]),
    shell: new Set([
        "case", "cd", "do", "done", "echo", "elif", "else", "esac", "exit", "export",
        "fi", "for", "function", "if", "in", "local", "return", "set", "then", "while",
    ]),
    json: new Set(["false", "null", "true"]),
    none: new Set(),
};
const LINE_COMMENT = {
    clike: ["//"],
    python: ["#"],
    shell: ["#"],
    json: [],
    none: [],
};
const WORD = /[A-Za-z_$][A-Za-z0-9_$]*/y;
// Hex first: the decimal branch would otherwise match the leading 0 of 0x1f
// and leave the rest of the literal looking like an identifier.
const NUMBER = /0[xXbBoO][0-9a-fA-F_]+|\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?/y;
export function family(lang) {
    return FAMILIES[lang.trim().toLowerCase().split(/\s+/)[0] ?? ""] ?? "none";
}
/**
 * Tokenize a whole block, because a block comment and a template string both
 * outlive the line they open on. State is carried between lines rather than
 * rediscovered, which is the only way the row after `/*` comes out grey.
 */
export function highlight(lines, lang) {
    const kind = family(lang);
    if (kind === "none")
        return lines.map((line) => [{ text: line, role: "plain" }]);
    const keywords = KEYWORDS[kind];
    const comments = LINE_COMMENT[kind];
    let inBlock = false;
    return lines.map((line) => {
        const tokens = [];
        let plain = "";
        let i = 0;
        const flush = () => {
            if (plain !== "")
                tokens.push({ text: plain, role: "plain" });
            plain = "";
        };
        const take = (text, role) => {
            flush();
            tokens.push({ text, role });
            i += text.length;
        };
        while (i < line.length) {
            if (inBlock) {
                const end = line.indexOf("*/", i);
                const stop = end === -1 ? line.length : end + 2;
                take(line.slice(i, stop), "comment");
                inBlock = end === -1;
                continue;
            }
            const rest = line.slice(i);
            if (kind === "clike" && rest.startsWith("/*")) {
                inBlock = true;
                continue;
            }
            const comment = comments.find((mark) => rest.startsWith(mark));
            if (comment !== undefined) {
                take(rest, "comment");
                continue;
            }
            const quote = line[i];
            if (quote === '"' || quote === "'" || quote === "`") {
                take(line.slice(i, closingQuote(line, i, quote)), "string");
                continue;
            }
            NUMBER.lastIndex = i;
            const number = NUMBER.exec(line);
            if (number !== null && number.index === i) {
                take(number[0], "number");
                continue;
            }
            WORD.lastIndex = i;
            const word = WORD.exec(line);
            if (word !== null && word.index === i) {
                if (keywords.has(word[0]))
                    take(word[0], "keyword");
                else {
                    plain += word[0];
                    i += word[0].length;
                }
                continue;
            }
            plain += quote;
            i += 1;
        }
        flush();
        return tokens.length === 0 ? [{ text: "", role: "plain" }] : tokens;
    });
}
/** Index just past the closing quote, or the end of the line if it never comes. */
function closingQuote(line, start, quote) {
    for (let i = start + 1; i < line.length; i++) {
        if (line[i] === "\\") {
            i++;
            continue;
        }
        if (line[i] === quote)
            return i + 1;
    }
    return line.length;
}
