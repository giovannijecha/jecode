use super::{App, Field, Panel};
use crate::{
    json::Value,
    tui::selector::{Purpose, Selector},
};
use std::path::PathBuf;

impl App {
    pub(super) fn show_lsp_field(&mut self, field: Field) {
        let Some(profile) = &self.lsp.draft else {
            self.show_lsp_form();
            return;
        };
        let (title, value) = match field {
            Field::Name => ("Name", profile.name.clone()),
            Field::Executable => ("Executable", profile.executable.clone()),
            Field::Languages => ("Languages", super::menus::associations(profile)),
            Field::Arguments => ("Arguments", display_arguments(&profile.arguments)),
            Field::Environment => ("Environment", String::new()),
            Field::RootMarkers => ("Root markers", profile.root_markers.join(", ")),
            Field::RootOverride => (
                "Root override",
                profile
                    .root_override
                    .as_ref()
                    .map_or(String::new(), |path| path.display().to_string()),
            ),
            Field::Initialization => (
                "Initialization options",
                if profile.initialization_options == Value::Null {
                    String::new()
                } else {
                    profile.initialization_options.encode()
                },
            ),
            Field::Settings => ("Settings", profile.settings.encode()),
            Field::Timeout => ("Request timeout", profile.request_timeout_ms.to_string()),
        };
        let parent = if matches!(field, Field::Name | Field::Executable | Field::Languages) {
            "Language server"
        } else {
            "Advanced"
        };
        let mut menu = Selector::lsp(Panel::Field(field), title, Some(parent), "", vec![]);
        menu.editor.replace(value);
        self.state.selector = Some(menu);
    }

    pub(in crate::tui) fn save_lsp_field(&mut self) {
        let Some(menu) = &self.state.selector else {
            return;
        };
        let Purpose::Lsp(Panel::Field(field)) = menu.purpose else {
            return;
        };
        let value = menu.editor.text.clone();
        let trimmed = value.trim();
        let Some(profile) = &mut self.lsp.draft else {
            return;
        };
        match field {
            Field::Name => profile.name = trimmed.to_owned(),
            Field::Executable => profile.executable = trimmed.to_owned(),
            Field::Languages => match parse_languages(trimmed) {
                Ok((languages, extensions)) => {
                    profile.languages = languages;
                    profile.extensions = extensions;
                }
                Err(error) => {
                    self.warn(&error);
                    return;
                }
            },
            Field::Arguments => match parse_arguments(trimmed) {
                Ok(arguments) => profile.arguments = arguments,
                Err(error) => {
                    self.warn(&error);
                    return;
                }
            },
            Field::Environment => {
                let Some((name, value)) = value.split_once('=') else {
                    self.warn("Use NAME=value · NAME= removes a variable");
                    return;
                };
                if name.is_empty() || name.contains(['=', '\0']) {
                    self.warn("Enter a valid environment variable name");
                    return;
                }
                if value.is_empty() {
                    profile.environment.remove(name);
                } else {
                    if crate::redact::sensitive(name, value) {
                        self.archive.redactor.add_secret(value.into());
                    }
                    profile.environment.insert(name.into(), value.into());
                }
            }
            Field::RootMarkers => profile.root_markers = csv(trimmed),
            Field::RootOverride => {
                profile.root_override = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
            }
            Field::Initialization => match parse_json(trimmed, Value::Null) {
                Ok(json) => profile.initialization_options = json,
                Err(error) => {
                    self.warn(&error);
                    return;
                }
            },
            Field::Settings => match parse_json(trimmed, Value::object([])) {
                Ok(json) => profile.settings = json,
                Err(error) => {
                    self.warn(&error);
                    return;
                }
            },
            Field::Timeout => match trimmed.parse::<u64>() {
                Ok(ms @ 100..=120_000) => profile.request_timeout_ms = ms,
                _ => {
                    self.warn("Request timeout must be 100–120000 ms");
                    return;
                }
            },
        }
        if matches!(field, Field::Name | Field::Executable | Field::Languages) {
            self.show_lsp_form();
        } else {
            self.show_lsp_advanced();
        }
    }
}

fn csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_languages(
    value: &str,
) -> Result<(Vec<String>, std::collections::BTreeMap<String, String>), String> {
    let mut languages = Vec::new();
    let mut extensions = std::collections::BTreeMap::new();
    for group in value
        .split(';')
        .map(str::trim)
        .filter(|group| !group.is_empty())
    {
        let Some((language, suffixes)) = group.split_once(':') else {
            return Err("Use language: .ext .ext; language: .ext".into());
        };
        let language = language.trim();
        if language.is_empty()
            || language.chars().any(char::is_whitespace)
            || languages.iter().any(|saved| saved == language)
        {
            return Err("Each language needs a distinct ID".into());
        }
        let mut count = 0;
        for suffix in suffixes.split_whitespace() {
            let Some(extension) = suffix.strip_prefix('.') else {
                return Err("File extensions must start with a dot".into());
            };
            if extension.is_empty()
                || extension.contains('.')
                || extensions
                    .insert(extension.to_owned(), language.to_owned())
                    .is_some()
            {
                return Err("Enter each nonempty extension once".into());
            }
            count += 1;
        }
        if count == 0 {
            return Err("Each language needs at least one file extension".into());
        }
        languages.push(language.to_owned());
    }
    if languages.is_empty() {
        return Err("Enter at least one language and file extension".into());
    }
    Ok((languages, extensions))
}

fn parse_json(value: &str, empty: Value) -> Result<Value, String> {
    if value.is_empty() {
        Ok(empty)
    } else {
        crate::json::parse(value)
    }
}

fn display_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if argument.is_empty()
                || argument.chars().any(char::is_whitespace)
                || argument.contains(['"', '\''])
            {
                format!(
                    "\"{}\"",
                    argument.replace('\\', "\\\\").replace('"', "\\\"")
                )
            } else {
                argument.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quotes group arguments; unquoted backslashes remain literal for Windows paths.
fn parse_arguments(value: &str) -> Result<Vec<String>, String> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, '"' | '\'') => {
                quote = Some(ch);
                started = true;
            }
            (Some(mark), ch) if ch == mark => quote = None,
            (Some('"'), '\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                current.push(chars.next().unwrap())
            }
            (None, ch) if ch.is_whitespace() => {
                if started {
                    output.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("Close the quoted argument".into());
    }
    if started {
        output.push(current);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_argument_spaces_and_windows_paths_round_trip() {
        let arguments = vec![
            "--config=C:\\My Project\\settings.json".into(),
            "two words".into(),
            String::new(),
        ];
        assert_eq!(
            parse_arguments(&display_arguments(&arguments)),
            Ok(arguments)
        );
    }

    #[test]
    fn language_editor_keeps_c_and_cpp_ids_distinct() {
        let (languages, extensions) = parse_languages("c: .c .h; cpp: .cc .cpp .hpp").unwrap();
        assert_eq!(languages, vec!["c", "cpp"]);
        assert_eq!(extensions.get("h").map(String::as_str), Some("c"));
        assert_eq!(extensions.get("hpp").map(String::as_str), Some("cpp"));
        assert!(parse_languages("c: .c; cpp: .c").is_err());
    }
}
