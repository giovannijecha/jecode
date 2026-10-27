use super::*;
use crate::tui::theme::Style;

#[test]
fn rust_keywords_numbers_strings_and_comments_keep_the_original_code() {
    let source = r#"let widget = 40; println!("fn 40"); // let"#;
    let row = Highlighter::new("rust").line(source);
    assert_eq!(row.plain(), source);
    assert_eq!(row.background, Some(CODE_SURFACE));
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "let" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "40" && span.style == CODE_NUMBER)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "\"fn 40\"" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "// let" && span.style == MUTED)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text.contains("widget") && span.style == CODE_TEXT)
    );
}

#[test]
fn rust_lifetimes_are_distinct_from_unicode_and_escaped_char_literals() {
    let source = r"fn take<'a>(x: &'a str) -> char { 'é'; '\n'; '\u{03b1}'; '\x41' }";
    let row = Highlighter::new("rust").line(source);
    assert_eq!(row.plain(), source);
    let strings: Vec<_> = row
        .spans
        .iter()
        .filter(|span| span.style == CODE_STRING)
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(strings, ["'é'", r"'\n'", r"'\u{03b1}'", r"'\x41'"]);
}

#[test]
fn nested_comments_and_raw_strings_continue_across_lines_and_end_cleanly() {
    let mut highlighter = Highlighter::new("rs");
    let sources = [
        "/* outer",
        "/* inner */ fn 40",
        "*/ let x = r#\"fn",
        "40\"#; let y = 2;",
    ];
    let rows: Vec<_> = sources
        .iter()
        .map(|source| highlighter.line(source))
        .collect();
    for (row, source) in rows.iter().zip(sources) {
        assert_eq!(row.plain(), source);
        assert_eq!(row.background, Some(CODE_SURFACE));
    }
    assert!(rows[1].spans.iter().all(|span| span.style == MUTED));
    assert!(
        rows[2]
            .spans
            .iter()
            .any(|span| span.text == "let" && span.style == KEYWORD)
    );
    assert!(
        rows[2]
            .spans
            .iter()
            .any(|span| span.text == "r#\"fn" && span.style == CODE_STRING)
    );
    assert!(
        rows[3]
            .spans
            .iter()
            .any(|span| span.text == "40\"#" && span.style == CODE_STRING)
    );
    assert!(
        rows[3]
            .spans
            .iter()
            .any(|span| span.text == "2" && span.style == CODE_NUMBER)
    );
}

#[test]
fn shell_and_json_literals_are_highlighted_without_interpreting_their_contents() {
    let shell = "if printf '%s' 'é40'; then # 40";
    let row = Highlighter::new("bash").line(shell);
    assert_eq!(row.plain(), shell);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "if" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "'é40'" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "# 40" && span.style == MUTED)
    );
    let json = r#"{"escaped": "\"true\"", "enabled": true, "n": 40}"#;
    let row = Highlighter::new("JSON").line(json);
    assert_eq!(row.plain(), json);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "true" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "40" && span.style == CODE_NUMBER)
    );
    let mut shell = Highlighter::new("sh");
    assert_eq!(shell.line("echo 'first").plain(), "echo 'first");
    let row = shell.line("if 40' ; then");
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "if 40'" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "then" && span.style == KEYWORD)
    );
}

#[test]
fn unlabelled_and_unsupported_languages_stay_readable_plain_text() {
    for label in ["", "cobol", "unavailable"] {
        let source = "  **text** `literal` λ40";
        let row = Highlighter::new(label).line(source);
        assert_eq!(row.plain(), source);
        assert!(row.spans.iter().all(|span| span.style == CODE_TEXT));
    }
}

#[test]
fn file_extensions_select_js_and_markup_without_changing_source_text() {
    let source = "export const label = 'it\\'s ready'; // keep the source";
    let row = Highlighter::for_path("site/test/architecture.test.mjs").line(source);
    assert_eq!(row.plain(), source);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "export" && span.style == KEYWORD)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "'it\\'s ready'" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "// keep the source" && span.style == MUTED)
    );
    let source = "<urlset xmlns=\"https://example.test/schema\">";
    let row = Highlighter::for_path("site/sitemap.xml").line(source);
    assert_eq!(row.plain(), source);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "xmlns" && span.style == KEYWORD)
    );
    assert!(row.spans.iter().any(|span| span.text == "\"https://example.test/schema\"" && span.style == CODE_STRING));
}

#[test]
fn markup_comments_and_attributes_keep_multiline_state() {
    let mut lexer = Highlighter::new("html");
    assert_eq!(lexer.line("<!-- first").plain(), "<!-- first");
    let row = lexer.line("<tag> --> <a href=\"first");
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "<tag> -->" && span.style == MUTED)
    );
    let source = "second\">value</a>";
    let row = lexer.line(source);
    assert_eq!(row.plain(), source);
    assert!(
        row.spans
            .iter()
            .any(|span| span.text == "second\"" && span.style == CODE_STRING)
    );
    assert!(
        row.spans
            .iter()
            .any(|span| span.text.contains("value") && span.style == CODE_TEXT)
    );
}

/// Each line of `source` lexed in order by the profile behind `label`.
fn lexed(label: &str, source: &[&str]) -> Vec<Line> {
    let mut highlighter = Highlighter::new(label);
    source.iter().map(|line| highlighter.line(line)).collect()
}

/// Whether the first `text` in `row` is drawn in `style` throughout.
fn styled(row: &Line, text: &str, style: Style) -> bool {
    let cells: Vec<_> = row
        .spans
        .iter()
        .flat_map(|span| span.text.chars().map(move |_| span.style))
        .collect();
    let plain = row.plain();
    plain.find(text).is_some_and(|at| {
        let start = plain[..at].chars().count();
        cells[start..start + text.chars().count()]
            .iter()
            .all(|&cell| cell == style)
    })
}

#[test]
fn profiles_cover_css_python_yaml_go_and_sql() {
    let css = lexed(
        "scss",
        &["@media (min-width: 40em) { a { color: #fff; width: 18vw; top: 28%; } } // c"],
    );
    for (text, style) in [
        ("@media", KEYWORD),
        ("40em", CODE_NUMBER),
        ("#fff", CODE_NUMBER),
        ("18vw", CODE_NUMBER),
        ("28%", CODE_NUMBER),
        ("// c", MUTED),
    ] {
        assert!(styled(&css[0], text, style), "css {text}");
    }
    let python = lexed(
        "py",
        &[
            "def load(path):",
            "    x = \"\"\"first",
            "second\"\"\" if 0.15 else r'\\d' # c",
        ],
    );
    assert!(styled(&python[0], "def", KEYWORD));
    assert!(styled(&python[1], "\"\"\"first", CODE_STRING));
    assert!(styled(&python[2], "second\"\"\"", CODE_STRING));
    assert!(styled(&python[2], "if", KEYWORD));
    assert!(styled(&python[2], "0.15", CODE_NUMBER));
    assert!(styled(&python[2], "r'\\d'", CODE_STRING));
    assert!(styled(&python[2], "# c", MUTED));
    let yaml = lexed(
        "yml",
        &[
            "name: CI # pipeline",
            "    - uses: actions/checkout@v4",
            "      run: a:b",
        ],
    );
    assert!(styled(&yaml[0], "name", KEYWORD));
    assert!(styled(&yaml[0], "CI", CODE_TEXT));
    assert!(styled(&yaml[0], "# pipeline", MUTED));
    assert!(styled(&yaml[1], "uses", KEYWORD));
    assert!(styled(&yaml[1], "actions", CODE_TEXT));
    assert!(styled(&yaml[2], "run", KEYWORD));
    assert!(styled(&yaml[2], "a:b", CODE_TEXT));
    // An apostrophe inside a plain value opens no string for later lines.
    let plain = lexed(
        "yaml",
        &[
            "description: Giovanni's site",
            "title: 'Home'",
            "tags: [\"a\", 'b']",
        ],
    );
    assert!(styled(&plain[0], "Giovanni's site", CODE_TEXT));
    assert!(styled(&plain[1], "title", KEYWORD));
    assert!(styled(&plain[1], "'Home'", CODE_STRING));
    assert!(styled(&plain[2], "\"a\"", CODE_STRING));
    assert!(styled(&plain[2], "'b'", CODE_STRING));
    let ini = lexed("ini", &["owner = Giovanni's PC", "name = \"jecode\""]);
    assert!(styled(&ini[0], "Giovanni's PC", CODE_TEXT));
    assert!(styled(&ini[1], "name", KEYWORD));
    assert!(styled(&ini[1], "\"jecode\"", CODE_STRING));
    let go = lexed(
        "golang",
        &[
            "func f() { // ready",
            "    s := `raw",
            "line` + \"x\"; return nil }",
        ],
    );
    assert!(styled(&go[0], "func", KEYWORD));
    assert!(styled(&go[0], "// ready", MUTED));
    assert!(styled(&go[1], "`raw", CODE_STRING));
    assert!(styled(&go[2], "line`", CODE_STRING));
    assert!(styled(&go[2], "return", KEYWORD));
    let sql = lexed(
        "sql",
        &["SELECT id FROM users -- all", "where name = 'x' /* c */"],
    );
    for (row, text, style) in [
        (0, "SELECT", KEYWORD),
        (0, "FROM", KEYWORD),
        (0, "users", CODE_TEXT),
        (0, "-- all", MUTED),
        (1, "where", KEYWORD),
        (1, "'x'", CODE_STRING),
        (1, "/* c */", MUTED),
    ] {
        assert!(styled(&sql[row], text, style), "sql {text}");
    }
}

#[test]
fn profiles_cover_markdown_c_toml_and_powershell() {
    let markdown = lexed(
        "md",
        &["## Title", "Run `cargo test` now.", "    # not a heading"],
    );
    assert!(styled(&markdown[0], "## Title", STRONG.bold()));
    assert!(styled(&markdown[1], "`cargo test`", CODE_STRING));
    assert!(styled(&markdown[1], "now.", CODE_TEXT));
    assert!(styled(&markdown[2], "# not", CODE_TEXT));
    let c = lexed(
        "c++",
        &["#include <vector>", "int main() { return 0x1F; } // ok"],
    );
    assert!(styled(&c[0], "#include", KEYWORD));
    assert!(styled(&c[0], "vector", CODE_TEXT));
    assert!(styled(&c[1], "int", KEYWORD));
    assert!(styled(&c[1], "0x1F", CODE_NUMBER));
    assert!(styled(&c[1], "// ok", MUTED));
    let toml = lexed(
        "toml",
        &["[package]", "name = \"jecode\" # crate", "edition = 2024"],
    );
    assert!(styled(&toml[0], "[package]", STRONG.bold()));
    assert!(styled(&toml[1], "name", KEYWORD));
    assert!(styled(&toml[1], "\"jecode\"", CODE_STRING));
    assert!(styled(&toml[1], "# crate", MUTED));
    assert!(styled(&toml[2], "2024", CODE_NUMBER));
    let shell = lexed(
        "pwsh",
        &[
            "<# block",
            "still #>",
            "If ($True) { Write-Host \"a`\"b\" } # note",
        ],
    );
    assert!(styled(&shell[0], "<# block", MUTED));
    assert!(styled(&shell[1], "still #>", MUTED));
    assert!(styled(&shell[2], "If", KEYWORD));
    assert!(styled(&shell[2], "$True", KEYWORD));
    assert!(styled(&shell[2], "\"a`\"b\"", CODE_STRING));
    assert!(styled(&shell[2], "# note", MUTED));
}

#[test]
fn perl_patterns_and_quote_like_operators_never_open_stray_strings() {
    let perl = lexed(
        "perl",
        &[
            "$text =~ s/harness's/x/g; # fix",
            "print \"ok\" if $#items > 0;",
            "my @parts = split /,/, qw(a b);",
            "$name =~ m{a{2}}i and print 'done';",
        ],
    );
    assert!(styled(&perl[0], "s/harness's/x/g", CODE_STRING));
    assert!(styled(&perl[0], "; ", CODE_TEXT));
    assert!(styled(&perl[0], "# fix", MUTED));
    // The apostrophe left no string open for the following lines.
    assert!(styled(&perl[1], "print", KEYWORD));
    assert!(styled(&perl[1], "\"ok\"", CODE_STRING));
    assert!(styled(&perl[1], "$#items > ", CODE_TEXT));
    assert!(styled(&perl[2], "my", KEYWORD));
    assert!(styled(&perl[2], "/,/", CODE_STRING));
    assert!(styled(&perl[2], "qw(a b)", CODE_STRING));
    assert!(styled(&perl[3], "m{a{2}}i", CODE_STRING));
    assert!(styled(&perl[3], "and", KEYWORD));
    assert!(styled(&perl[3], "'done'", CODE_STRING));
    // Division stays code.
    assert!(styled(&lexed("pm", &["$x / $y"])[0], "$x / $y", CODE_TEXT));
}

#[test]
fn profiles_cover_ruby_lua_php_and_batch() {
    let ruby = lexed("rb", &["def ready? # ask", "  return \"yes\" unless nil"]);
    assert!(styled(&ruby[0], "def", KEYWORD));
    assert!(styled(&ruby[0], "# ask", MUTED));
    assert!(styled(&ruby[1], "\"yes\"", CODE_STRING));
    assert!(styled(&ruby[1], "unless", KEYWORD));
    let lua = lexed(
        "lua",
        &[
            "local s = [[long",
            "text]] -- c",
            "--[[ block",
            "]] return nil",
        ],
    );
    assert!(styled(&lua[0], "local", KEYWORD));
    assert!(styled(&lua[0], "[[long", CODE_STRING));
    assert!(styled(&lua[1], "text]]", CODE_STRING));
    assert!(styled(&lua[1], "-- c", MUTED));
    assert!(styled(&lua[2], "--[[ block", MUTED));
    assert!(styled(&lua[3], "]]", MUTED));
    assert!(styled(&lua[3], "return", KEYWORD));
    let php = lexed(
        "php",
        &["$value = TRUE; # hash", "echo 'x'; // slash /* c */"],
    );
    assert!(styled(&php[0], "$value", CODE_TEXT));
    assert!(styled(&php[0], "TRUE", KEYWORD));
    assert!(styled(&php[0], "# hash", MUTED));
    assert!(styled(&php[1], "echo", KEYWORD));
    assert!(styled(&php[1], "'x'", CODE_STRING));
    assert!(styled(&php[1], "// slash /* c */", MUTED));
    let batch = lexed(
        "cmd",
        &[
            "@ECHO off",
            "REM note",
            ":: label note",
            "set X=\"C:\\dir\\\" & goto end",
            "remark",
        ],
    );
    assert!(styled(&batch[0], "ECHO", KEYWORD));
    assert!(styled(&batch[1], "REM note", MUTED));
    assert!(styled(&batch[2], ":: label note", MUTED));
    assert!(styled(&batch[3], "set", KEYWORD));
    assert!(styled(&batch[3], "\"C:\\dir\\\"", CODE_STRING));
    assert!(styled(&batch[3], "goto", KEYWORD));
    assert!(styled(&batch[4], "remark", CODE_TEXT));
}

#[test]
fn fenced_labels_and_file_extensions_select_the_same_profiles() {
    let source = "for x in \"y\" `z` # 12";
    for (label, path) in [
        ("python", "a.py"),
        ("yaml", "a.yaml"),
        ("golang", "a.go"),
        ("csharp", "a.cs"),
        ("kotlin", "a.kt"),
        ("powershell", "a.ps1"),
        ("perl", "a.pl"),
        ("ruby", "a.rb"),
        ("lua", "a.lua"),
        ("batch", "a.bat"),
        ("markdown", "README.md"),
        ("CSS", "style.CSS"),
    ] {
        let fenced = Highlighter::new(label).line(source);
        assert_eq!(fenced, Highlighter::for_path(path).line(source), "{label}");
        assert_ne!(fenced, Highlighter::new("").line(source), "{label}");
    }
    // Names and keywords are single-space lists, across continued lines too.
    for profile in PROFILES {
        for list in [profile.names, profile.keywords] {
            assert!(
                list.split(' ')
                    .all(|word| !word.is_empty() || list.is_empty())
            );
        }
    }
    let row = Highlighter::for_path("notes.unknown").line("fn x() { \"s\" } // 40");
    assert!(row.spans.iter().all(|span| span.style == CODE_TEXT));
}
