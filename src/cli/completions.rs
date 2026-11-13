//! Shell completion scripts generated from the command table.

use super::grammar::{self, Command};

/// Each command path with the words that may follow it.
fn rows() -> Vec<(String, Vec<String>)> {
    let mut rows = Vec::new();
    fn visit(command: &Command, path: &str, rows: &mut Vec<(String, Vec<String>)>) {
        let mut words: Vec<String> = command
            .subcommands
            .iter()
            .map(|sub| sub.name.to_owned())
            .collect();
        if command.name == "completions" {
            words.extend(grammar::SHELLS.iter().map(|shell| (*shell).to_owned()));
        }
        let mut options: Vec<&grammar::Opt> = command.options.to_vec();
        options.extend(grammar::GLOBAL.iter().copied());
        if path.is_empty() {
            options.push(&grammar::VERSION);
        }
        words.extend(options.iter().map(|option| format!("--{}", option.long)));
        rows.push((path.to_owned(), words));
        for sub in command.subcommands {
            let next = if path.is_empty() {
                sub.name.to_owned()
            } else {
                format!("{path} {}", sub.name)
            };
            visit(sub, &next, rows);
        }
    }
    visit(&grammar::ROOT, "", &mut rows);
    rows
}

/// Options whose values come from a fixed list, by long name.
fn choices() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut choices: Vec<_> = grammar::known()
        .into_iter()
        .filter(|option| !option.choices.is_empty())
        .map(|option| (option.long, option.choices.to_vec()))
        .collect();
    choices.push(("routing", vec!["auto"]));
    choices
}

/// Options taking a free value, such as a path or a model ID: shells offer
/// files there instead of command words.
fn free() -> Vec<&'static grammar::Opt> {
    grammar::known()
        .into_iter()
        .filter(|option| {
            option.value.is_some() && option.choices.is_empty() && option.long != "routing"
        })
        .collect()
}

/// `--long` and `-s` spellings of the free options.
fn free_spellings() -> Vec<String> {
    free()
        .into_iter()
        .flat_map(|option| {
            std::iter::once(format!("--{}", option.long))
                .chain(option.short.map(|short| format!("-{short}")))
        })
        .collect()
}

pub fn script(shell: &str) -> String {
    match shell {
        "bash" => bash(),
        "zsh" => zsh(),
        "fish" => fish(),
        "powershell" => powershell(),
        _ => unreachable!("the parser accepts only supported shells"),
    }
}

fn case_rows(indent: &str) -> String {
    rows()
        .iter()
        .map(|(path, words)| format!("{indent}\"{path}\") echo \"{}\" ;;\n", words.join(" ")))
        .collect()
}

fn value_rows(indent: &str) -> String {
    choices()
        .iter()
        .map(|(option, values)| format!("{indent}--{option}) echo \"{}\" ;;\n", values.join(" ")))
        .collect()
}

/// A `case` pattern matching every free option.
fn free_pattern() -> String {
    free_spellings().join("|")
}

fn bash() -> String {
    format!(
        "# jecode completion for bash. Load with: source <(jecode completions bash)\n\
_jecode_words() {{\n    case \"$1\" in\n{}        *) echo \"\" ;;\n    esac\n}}\n\
_jecode_values() {{\n    case \"$1\" in\n{}        *) return 1 ;;\n    esac\n}}\n\
_jecode() {{\n    local cur=\"${{COMP_WORDS[COMP_CWORD]}}\" prev=\"${{COMP_WORDS[COMP_CWORD-1]}}\" path=\"\" word i values\n    COMPREPLY=()\n    case \"$prev\" in {free}) return ;; esac\n    if values=$(_jecode_values \"$prev\"); then\n        COMPREPLY=($(compgen -W \"$values\" -- \"$cur\"))\n        return\n    fi\n    for ((i = 1; i < COMP_CWORD; i++)); do\n        word=\"${{COMP_WORDS[i]}}\"\n        case \"$word\" in\n            --) return ;;\n            -*) ;;\n            *) case \" $(_jecode_words \"$path\") \" in\n                   *\" $word \"*) path=\"${{path:+$path }}$word\" ;;\n               esac ;;\n        esac\n    done\n    COMPREPLY=($(compgen -W \"$(_jecode_words \"$path\")\" -- \"$cur\"))\n}}\ncomplete -o default -F _jecode jecode\n",
        case_rows("        "),
        value_rows("        "),
        free = free_pattern()
    )
}

fn zsh() -> String {
    format!(
        "#compdef jecode\n# jecode completion for zsh. Load with: source <(jecode completions zsh)\n\
_jecode_words() {{\n    case \"$1\" in\n{}        *) echo \"\" ;;\n    esac\n}}\n\
_jecode_values() {{\n    case \"$1\" in\n{}        *) return 1 ;;\n    esac\n}}\n\
_jecode() {{\n    local command_path=\"\" word i values\n    case \"${{words[CURRENT-1]}}\" in {free}) _files; return ;; esac\n    if values=$(_jecode_values \"${{words[CURRENT-1]}}\"); then\n        compadd -- ${{=values}}\n        return\n    fi\n    for ((i = 2; i < CURRENT; i++)); do\n        word=\"${{words[i]}}\"\n        case \"$word\" in\n            --) return ;;\n            -*) ;;\n            *) case \" $(_jecode_words \"$command_path\") \" in\n                   *\" $word \"*) command_path=\"${{command_path:+$command_path }}$word\" ;;\n               esac ;;\n        esac\n    done\n    compadd -- ${{=$(_jecode_words \"$command_path\")}}\n}}\nif [[ \"${{funcstack[1]}}\" == _jecode ]]; then _jecode \"$@\"; else compdef _jecode jecode; fi\n",
        case_rows("        "),
        value_rows("        "),
        free = free_pattern()
    )
}

fn fish() -> String {
    let mut script = String::from(
        "# jecode completion for fish. Load with: jecode completions fish | source\n\
function __jecode_path\n    set -l tokens (commandline -opc)\n    set -e tokens[1]\n    set -l path\n    for token in $tokens\n        if test \"$token\" = --\n            echo __literal\n            return\n        end\n        if string match -q -- '-*' $token\n            continue\n        end\n        if contains -- $token (__jecode_words (string join ' ' $path))\n            set path $path $token\n        end\n    end\n    string join ' ' $path\nend\n\
function __jecode_words\n    switch \"$argv[1]\"\n",
    );
    for (path, words) in rows() {
        script.push_str(&format!(
            "        case '{path}'\n            printf '%s\\n' {}\n",
            words.join(" ")
        ));
    }
    script.push_str("    end\nend\n");
    script.push_str("complete -c jecode -f -a '(__jecode_words (__jecode_path))'\n");
    for (option, values) in choices() {
        script.push_str(&format!(
            "complete -c jecode -l {option} -x -a '{}'\n",
            values.join(" ")
        ));
    }
    for option in free() {
        let short = option
            .short
            .map_or_else(String::new, |short| format!(" -s {short}"));
        script.push_str(&format!(
            "complete -c jecode{short} -l {} -r -F\n",
            option.long
        ));
    }
    script
}

fn powershell() -> String {
    let words = rows()
        .iter()
        .map(|(path, words)| format!("    '{path}' = @({})\n", quoted(words)))
        .collect::<String>();
    let values = choices()
        .iter()
        .map(|(option, values)| {
            let values: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
            format!("    '--{option}' = @({})\n", quoted(&values))
        })
        .collect::<String>();
    let free = quoted(&free_spellings());
    format!(
        "# jecode completion for PowerShell. Load with:\n# jecode completions powershell | Out-String | Invoke-Expression\n\
$jecodeWords = @{{\n{words}}}\n$jecodeValues = @{{\n{values}}}\n$jecodeFree = @({free})\n\
Register-ArgumentCompleter -Native -CommandName jecode, jecode.exe -ScriptBlock {{\n    param($wordToComplete, $commandAst, $cursorPosition)\n    $elements = @($commandAst.CommandElements | Select-Object -Skip 1 | ForEach-Object {{ $_.ToString() }})\n    if ($wordToComplete -ne '' -and $elements.Count -gt 0) {{ $elements = @($elements | Select-Object -SkipLast 1) }}\n    if ($elements.Count -gt 0 -and $jecodeFree -contains $elements[-1]) {{ return }}\n    $path = ''\n    foreach ($element in $elements) {{\n        if ($element -eq '--') {{ return }}\n        if ($element.StartsWith('-')) {{ continue }}\n        if ($jecodeWords[$path] -contains $element) {{ $path = ($path + ' ' + $element).Trim() }}\n    }}\n    $candidates = $jecodeWords[$path]\n    if ($elements.Count -gt 0 -and $jecodeValues.ContainsKey($elements[-1])) {{ $candidates = $jecodeValues[$elements[-1]] }}\n    $candidates | Where-Object {{ $_ -like \"$wordToComplete*\" }} | ForEach-Object {{\n        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_)\n    }}\n}}\n"
    )
}

fn quoted(words: &[String]) -> String {
    words
        .iter()
        .map(|word| format!("'{word}'"))
        .collect::<Vec<_>>()
        .join(", ")
}
