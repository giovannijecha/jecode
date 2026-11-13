use super::syntax::*;
use crate::attachments::MARKER;

#[test]
fn a_sigil_opens_only_alone_at_a_word_boundary() {
    let marker = format!("see {MARKER}");
    for (text, cursor, expected) in [
        ("", 0, true),
        ("see ", 4, true),
        ("line\n", 5, true),
        (marker.as_str(), marker.len(), true),
        ("see  more", 4, true),
        // An address, an escape, a word that follows and an opening bracket stay text.
        ("user", 4, false),
        ("\\", 1, false),
        ("echo HOME", 5, false),
        ("(", 1, false),
        ("US", 2, false),
    ] {
        assert_eq!(opens(text, cursor), expected, "{text:?} at {cursor}");
    }
}

#[test]
fn tokens_find_skills_and_files_but_leave_other_symbols_alone() {
    let text = "use $review on @src/main.rs. Mail a@b.com, pay $5.00 or $(cmd)";
    let found = tokens(text);
    assert_eq!(
        found,
        [
            Token {
                sigil: Sigil::Skill,
                start: 4,
                end: 11,
                value: "review".into(),
            },
            Token {
                sigil: Sigil::File,
                start: 15,
                end: 27,
                value: "src/main.rs".into(),
            },
        ]
    );
    assert_eq!(&text[found[1].start..found[1].end], "@src/main.rs");
    // Sentence punctuation ends a skill name only before a boundary.
    assert_eq!(tokens("try $review, then")[0].value, "review");
    assert!(tokens("$5.00").is_empty());
}

#[test]
fn quoted_files_unescape_and_unclosed_quotes_are_text() {
    let text = r#"read @"docs/my file.md" and @"a\"b\\c""#;
    let values: Vec<_> = tokens(text).into_iter().map(|token| token.value).collect();
    assert_eq!(values, ["docs/my file.md", r#"a"b\c"#]);
    assert!(tokens(r#"@"docs/my file"#).is_empty());
    assert!(tokens("@\"a\nb\"").is_empty());
    for marker in [MARKER.to_string(), format!("\\{MARKER}")] {
        assert!(tokens(&format!("@\"a{marker}b\"")).is_empty());
    }
}

#[test]
fn the_caret_finds_the_reference_it_touches_including_a_lone_sigil() {
    let text = "a $old b @x.rs";
    assert_eq!(at(text, 2).unwrap().value, "old");
    assert_eq!(at(text, 6).unwrap().value, "old");
    assert!(at(text, 7).is_none());
    assert_eq!(at(text, 14).unwrap().sigil, Sigil::File);
    let lone = at("see @", 5).unwrap();
    assert_eq!((lone.start, lone.end, lone.value.as_str()), (4, 5, ""));
}

#[test]
fn file_references_quote_only_when_needed_and_read_back_unchanged() {
    for (path, written) in [
        ("src/main.rs", "@src/main.rs"),
        ("docs/données.md", "@docs/données.md"),
        ("docs/日本.md", "@docs/日本.md"),
        ("docs/my file.md", r#"@"docs/my file.md""#),
        ("notes.", r#"@"notes.""#),
        ("a(b).rs", r#"@"a(b).rs""#),
        (r#"say"hi".txt"#, r#"@"say\"hi\".txt""#),
        (r"back\slash", r#"@"back\\slash""#),
    ] {
        let reference = file(path).unwrap();
        assert_eq!(reference, written);
        let token = &tokens(&format!("see {reference} now"))[0];
        assert_eq!(token.value, path, "{reference}");
    }
    assert_eq!(file("bad\nname"), None);
    assert_eq!(file(&format!("bad{MARKER}name")), None);
    assert_eq!(file(""), None);
    assert_eq!(skill("review"), "$review");
}

#[test]
fn serialized_file_references_decode_without_changing_literal_filename_characters() {
    for path in [
        "docs/my file.md",
        r#"say"hi".txt"#,
        r#""quoted".txt"#,
        r"back\slash",
        r#"a\"b.txt"#,
    ] {
        assert_eq!(typed_path(&file(path).unwrap()), path, "{path:?}");
    }
}

#[test]
fn typed_paths_accept_both_separators_quotes_and_a_leading_sigil() {
    assert_eq!(typed_path(r#"@"docs\my file.md""#), "docs/my file.md");
    assert_eq!(typed_path(r".\src\tui\input.rs"), "src/tui/input.rs");
    assert_eq!(typed_path("  src/main.rs "), "src/main.rs");
    assert_eq!(typed_path("données"), "données");
}
