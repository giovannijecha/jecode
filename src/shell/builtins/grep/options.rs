//! Command-line options of `grep`.

#[derive(Default)]
pub(super) struct Options {
    pub(super) patterns: Vec<String>,
    pub(super) extended: bool,
    pub(super) fixed: bool,
    pub(super) insensitive: bool,
    pub(super) invert: bool,
    pub(super) numbers: bool,
    pub(super) count: bool,
    pub(super) listed: Option<bool>,
    pub(super) quiet: bool,
    pub(super) silent: bool,
    pub(super) names: Option<bool>,
    pub(super) only: bool,
    pub(super) words: bool,
    pub(super) whole: bool,
    pub(super) recursive: bool,
    pub(super) after: usize,
    pub(super) before: usize,
    pub(super) max: Option<usize>,
    pub(super) include: Vec<String>,
    pub(super) exclude: Vec<String>,
    pub(super) exclude_dir: Vec<String>,
    pub(super) operands: Vec<String>,
}

pub(super) fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut rest = args[1..].iter();
    let mut operands_only = false;
    let number = |value: &str| {
        value
            .parse::<usize>()
            .map_err(|_| format!("grep: invalid number: {value}"))
    };
    while let Some(arg) = rest.next() {
        if operands_only || arg == "-" || !arg.starts_with('-') {
            options.operands.push(arg.clone());
            continue;
        }
        if arg == "--" {
            operands_only = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = long
                .split_once('=')
                .map_or((long, None), |(name, value)| (name, Some(value)));
            let mut value = || {
                inline
                    .map(str::to_owned)
                    .or_else(|| rest.next().cloned())
                    .ok_or(format!("grep: --{name} needs a value"))
            };
            match name {
                "include" => options.include.push(value()?),
                "exclude" => options.exclude.push(value()?),
                "exclude-dir" => options.exclude_dir.push(value()?),
                "regexp" => options.patterns.push(value()?),
                "max-count" => options.max = Some(number(&value()?)?),
                "after-context" => options.after = number(&value()?)?,
                "before-context" => options.before = number(&value()?)?,
                "context" => {
                    let lines = number(&value()?)?;
                    (options.after, options.before) = (lines, lines);
                }
                "color" | "colour" | "binary-files" => {}
                "extended-regexp" => options.extended = true,
                "perl-regexp" => {
                    return Err("grep: Perl regular expressions are unsupported".into());
                }
                "fixed-strings" => options.fixed = true,
                "ignore-case" => options.insensitive = true,
                "invert-match" => options.invert = true,
                "line-number" => options.numbers = true,
                "count" => options.count = true,
                "files-with-matches" => options.listed = Some(true),
                "files-without-match" => options.listed = Some(false),
                "quiet" | "silent" => options.quiet = true,
                "no-messages" => options.silent = true,
                "recursive" | "dereference-recursive" => options.recursive = true,
                "only-matching" => options.only = true,
                "word-regexp" => options.words = true,
                "line-regexp" => options.whole = true,
                "no-filename" => options.names = Some(false),
                "with-filename" => options.names = Some(true),
                _ => return Err(format!("grep: unsupported option --{name}")),
            }
            continue;
        }
        let flags: Vec<char> = arg[1..].chars().collect();
        let mut index = 0;
        while index < flags.len() {
            let flag = flags[index];
            index += 1;
            let mut value = || -> Result<String, String> {
                if index < flags.len() {
                    let attached = flags[index..].iter().collect();
                    index = flags.len();
                    Ok(attached)
                } else {
                    rest.next()
                        .cloned()
                        .ok_or(format!("grep: -{flag} needs a value"))
                }
            };
            match flag {
                'E' => options.extended = true,
                'P' => return Err("grep: Perl regular expressions are unsupported".into()),
                'G' => options.extended = false,
                'F' => options.fixed = true,
                'i' | 'y' => options.insensitive = true,
                'v' => options.invert = true,
                'n' => options.numbers = true,
                'c' => options.count = true,
                'l' => options.listed = Some(true),
                'L' => options.listed = Some(false),
                'q' => options.quiet = true,
                's' => options.silent = true,
                'h' => options.names = Some(false),
                'H' => options.names = Some(true),
                'o' => options.only = true,
                'w' => options.words = true,
                'x' => options.whole = true,
                'r' | 'R' => options.recursive = true,
                'I' | 'a' | 'U' => {}
                'e' => options.patterns.push(value()?),
                'm' => options.max = Some(number(&value()?)?),
                'A' => options.after = number(&value()?)?,
                'B' => options.before = number(&value()?)?,
                'C' => {
                    let lines = number(&value()?)?;
                    (options.after, options.before) = (lines, lines);
                }
                digit if digit.is_ascii_digit() => {
                    let lines = number(&flags[index - 1..].iter().collect::<String>())?;
                    index = flags.len();
                    (options.after, options.before) = (lines, lines);
                }
                _ => return Err(format!("grep: unsupported option -{flag}")),
            }
        }
    }
    if options.patterns.is_empty() {
        if options.operands.is_empty() {
            return Err("grep: expected a pattern".into());
        }
        options.patterns.push(options.operands.remove(0));
    }
    Ok(options)
}
