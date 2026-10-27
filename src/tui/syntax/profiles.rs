// Language profiles for the display lexer: data only, read by `Highlighter`.

/// What one language looks like to the lexer. Numbers and identifiers are
/// universal; everything else is opted into here.
pub(super) struct Profile {
    /// File extensions and fenced-code labels, lowercase, separated by spaces.
    pub names: &'static str,
    pub line_comments: &'static [&'static str],
    /// Line comments open only at the start or after whitespace, as `#` in a
    /// shell, so `a#b` or `url(http://x)` stay code.
    pub spaced: bool,
    /// Opening and closing markers, and whether they nest.
    pub block: Option<(&'static str, &'static str, bool)>,
    pub quotes: &'static str,
    /// A quote whose contents never escape, as `'` in a shell.
    pub verbatim: Option<char>,
    pub escape: char,
    /// `"""` and `'''` strings span lines.
    pub triple: bool,
    /// Quotes open a string only where a value starts, as in YAML and INI, so
    /// the apostrophe in `name: Giovanni's site` stays text.
    pub value_quotes: bool,
    /// `r`, `b`, `u` and `f` before a quote, as in Python; `r` turns escapes off.
    pub string_prefixes: bool,
    /// Long brackets without escapes, such as Lua's `[[…]]`.
    pub long_strings: Option<(&'static str, &'static str)>,
    /// Markers that comment out a line from its start, in any case, such as
    /// `REM` and `::` in a batch file.
    pub line_start_comments: &'static [&'static str],
    /// Raw strings `r#"…"#` and char literals `'a'`, as distinct from lifetimes.
    pub rust: bool,
    /// Patterns and quote-like operators such as `s/a/b/g` and `qw(a b)`.
    pub perl: bool,
    /// `$` belongs to identifiers, as in `$true` or `$scope`.
    pub dollar: bool,
    pub caseless: bool,
    pub at_rules: bool,
    pub hex_colors: bool,
    /// A key before `:` at the start of a line or list item.
    pub yaml_keys: bool,
    /// `[section]` headers and `key =` at the start of a line.
    pub ini_keys: bool,
    /// `#include`-style directives at the start of a line.
    pub preprocessor: bool,
    /// Separated by spaces.
    pub keywords: &'static str,
}

const BASE: Profile = Profile {
    names: "",
    line_comments: &[],
    spaced: false,
    block: None,
    quotes: "\"",
    verbatim: None,
    escape: '\\',
    triple: false,
    value_quotes: false,
    string_prefixes: false,
    long_strings: None,
    line_start_comments: &[],
    rust: false,
    perl: false,
    dollar: false,
    caseless: false,
    at_rules: false,
    hex_colors: false,
    yaml_keys: false,
    ini_keys: false,
    preprocessor: false,
    keywords: "",
};

const SLASHES: &[&str] = &["//"];
const HASH: &[&str] = &["#"];
const C_BLOCK: Option<(&str, &str, bool)> = Some(("/*", "*/", false));

pub(super) const PROFILES: &[Profile] = &[
    Profile {
        names: "rust rs",
        line_comments: SLASHES,
        block: Some(("/*", "*/", true)),
        rust: true,
        keywords: "as async await break const continue crate dyn else enum extern false fn \
            for if impl in let loop match mod move mut pub ref return self Self static \
            struct super trait true type unsafe use where while",
        ..BASE
    },
    Profile {
        names: "javascript js mjs cjs jsx typescript ts mts cts tsx",
        line_comments: SLASHES,
        block: C_BLOCK,
        quotes: "\"'`",
        dollar: true,
        keywords: "async await break case catch class const continue debugger default delete \
            do else export extends false finally for from function if import in instanceof \
            interface let new null of return static super switch this throw true try type \
            typeof undefined var void while yield",
        ..BASE
    },
    Profile {
        names: "json jsonc",
        line_comments: SLASHES,
        block: C_BLOCK,
        keywords: "true false null",
        ..BASE
    },
    Profile {
        names: "bash sh shell zsh",
        line_comments: HASH,
        spaced: true,
        quotes: "\"'",
        verbatim: Some('\''),
        keywords: "case do done elif else esac fi for function if in select then until while",
        ..BASE
    },
    Profile {
        names: "css scss sass less",
        line_comments: SLASHES,
        spaced: true,
        block: C_BLOCK,
        quotes: "\"'",
        at_rules: true,
        hex_colors: true,
        ..BASE
    },
    Profile {
        names: "python py pyi pyw",
        line_comments: HASH,
        quotes: "\"'",
        triple: true,
        string_prefixes: true,
        keywords: "False None True and as assert async await break case class continue def \
            del elif else except finally for from global if import in is lambda match \
            nonlocal not or pass raise return self try while with yield",
        ..BASE
    },
    Profile {
        names: "yaml yml",
        line_comments: HASH,
        spaced: true,
        quotes: "\"'",
        verbatim: Some('\''),
        value_quotes: true,
        yaml_keys: true,
        keywords: "true false null",
        ..BASE
    },
    Profile {
        names: "toml ini cfg conf properties",
        line_comments: &["#", ";"],
        spaced: true,
        quotes: "\"'",
        verbatim: Some('\''),
        triple: true,
        value_quotes: true,
        ini_keys: true,
        keywords: "true false",
        ..BASE
    },
    Profile {
        names: "go golang",
        line_comments: SLASHES,
        block: C_BLOCK,
        quotes: "\"'`",
        verbatim: Some('`'),
        keywords: "break case chan const continue default defer else fallthrough false for \
            func go goto if import interface iota map nil package range return select struct \
            switch true type var",
        ..BASE
    },
    Profile {
        names: "c h cc cpp cxx hpp hh c++ cs csharp java kt kts kotlin swift m mm",
        line_comments: SLASHES,
        block: C_BLOCK,
        quotes: "\"'",
        preprocessor: true,
        keywords: "abstract as async auto await bool boolean break byte case catch char \
            class const constexpr continue default defer delegate delete do double else enum \
            explicit export extends extension extern false final finally float for foreach \
            fun func goto guard if implements import in inline instanceof int interface \
            internal is let long namespace new nil null object operator override package \
            private protected public readonly return sealed self short signed sizeof static \
            struct super switch template this throw throws true try typedef typename typeof \
            uint union unsigned using val var virtual void volatile when where while yield",
        ..BASE
    },
    Profile {
        names: "powershell pwsh ps1 psm1 psd1",
        line_comments: HASH,
        spaced: true,
        block: Some(("<#", "#>", false)),
        quotes: "\"'",
        verbatim: Some('\''),
        escape: '`',
        dollar: true,
        caseless: true,
        keywords: "begin break catch class continue data do dynamicparam else elseif end \
            enum exit filter finally for foreach function if in param process return switch \
            throw trap try until using while $true $false $null",
        ..BASE
    },
    Profile {
        names: "sql",
        line_comments: &["--"],
        block: C_BLOCK,
        quotes: "'\"",
        verbatim: Some('\''),
        caseless: true,
        keywords: "add all alter and as asc begin between by case check commit constraint \
            count create default delete desc distinct drop else end exists false foreign \
            from full group having in index inner insert into is join key left like limit \
            not null offset on or order outer primary references returning right rollback \
            select set table then transaction true union unique update values view when \
            where with",
        ..BASE
    },
    Profile {
        names: "perl pl pm t",
        line_comments: HASH,
        quotes: "\"'`",
        dollar: true,
        perl: true,
        keywords: "and bless chomp close cmp defined delete die do each else elsif eq eval \
            exists for foreach ge grep gt if join keys last le local lt map my ne next no \
            not open or our package pop print printf push redo ref require return say scalar \
            shift sort split sprintf sub undef unless unshift until use values warn while",
        ..BASE
    },
    Profile {
        names: "ruby rb",
        line_comments: HASH,
        quotes: "\"'`",
        keywords: "alias and begin break case class def defined do else elsif end ensure \
            false for if in module next nil not or redo require rescue retry return self \
            super then true undef unless until when while yield",
        ..BASE
    },
    Profile {
        names: "lua",
        line_comments: &["--"],
        block: Some(("--[[", "]]", false)),
        quotes: "\"'",
        long_strings: Some(("[[", "]]")),
        keywords: "and break do else elseif end false for function goto if in local nil not \
            or repeat return then true until while",
        ..BASE
    },
    Profile {
        names: "php",
        line_comments: &["//", "#"],
        block: C_BLOCK,
        quotes: "\"'`",
        caseless: true,
        keywords: "abstract and array as break case catch class clone const continue declare \
            default do echo else elseif empty enum extends false final finally fn for \
            foreach function global if implements include include_once instanceof interface \
            isset list match namespace new null or print private protected public readonly \
            require require_once return static switch throw trait true try unset use var \
            while yield",
        ..BASE
    },
    Profile {
        names: "bat cmd batch",
        line_start_comments: &["rem", "::"],
        verbatim: Some('"'),
        caseless: true,
        keywords: "call defined do echo else endlocal exist exit for goto if in not set \
            setlocal",
        ..BASE
    },
];

pub(super) const MARKUP: &str = "xml html htm xhtml svg";
pub(super) const MARKDOWN: &str = "markdown md mdx";
