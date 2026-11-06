//! Browser key events, independent of the host's keyboard layout.
use super::{actions, transport::Cdp};
use crate::cancel::Cancellation;
use crate::json::Value;
use std::time::{Duration, Instant};

const SHIFT: usize = 8;
const SHORTCUT: usize = 1 | 2 | 4;

pub(super) fn run(
    connection: &mut Cdp,
    action: &str,
    arguments: &Value,
    cancellation: &Cancellation,
    timeout: Duration,
) -> Result<(), String> {
    // Validate the whole request before focusing or sending any input.
    let keys = if action == "press" {
        let modifiers = match optional(arguments, "modifiers") {
            None => 0,
            Some(value) => value
                .as_usize()
                .filter(|value| *value <= 15)
                .ok_or("browser modifiers must be an integer from 0 to 15")?,
        };
        vec![Key::parse(required(arguments, "key")?, modifiers)?]
    } else {
        required(arguments, "text")?
            .replace("\r\n", "\n")
            .chars()
            .map(|character| Key::character(character, 0))
            .collect::<Result<Vec<_>, _>>()?
    };
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("Browser keyboard timeout is too large")?;
    if let Some(selector) = optional(arguments, "selector") {
        let selector = selector
            .as_str()
            .ok_or("browser selector must be a string")?;
        // A widget may hold focus inside itself or move it on focus; only an
        // element that leaves focus elsewhere and unchanged cannot take input.
        let expression = format!(
            "(()=>{{const e={};if(!e)return false;const b=document.activeElement;e.focus();const a=document.activeElement;return a!==b||e.contains(a)}})()",
            actions::element_expr(selector)
        );
        if actions::evaluate(
            connection,
            &expression,
            cancellation,
            remaining(deadline, cancellation)?,
        )? != Value::Bool(true)
        {
            return Err("Browser element is missing or cannot receive focus".into());
        }
    }
    let total = keys.len();
    for (completed, key) in keys.iter().enumerate() {
        for kind in ["keyDown", "keyUp"] {
            remaining(deadline, cancellation)
                .and_then(|timeout| {
                    connection.call(
                        "Input.dispatchKeyEvent",
                        key.event(kind),
                        cancellation,
                        timeout,
                    )
                })
                // Delivered keys stay applied, so a retry needs to know them.
                .map_err(|error| {
                    if cancellation.requested() {
                        return error;
                    }
                    let error = if kind == "keyUp" {
                        format!("keyUp did not complete; the current key may already have acted: {error}")
                    } else {
                        error
                    };
                    if completed == 0 {
                        error
                    } else {
                        format!("{completed} of {total} keys completed: {error}")
                    }
                })?;
        }
    }
    Ok(())
}

fn remaining(deadline: Instant, cancellation: &Cancellation) -> Result<Duration, String> {
    if cancellation.requested() {
        return Err("Operation cancelled".into());
    }
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or("Browser keyboard input timed out".into())
}

fn required<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("browser {name} is required"))
}

fn optional<'a>(arguments: &'a Value, name: &str) -> Option<&'a Value> {
    arguments
        .get(name)
        .filter(|value| !matches!(value, Value::Null))
}

struct Key {
    key: String,
    code: String,
    virtual_code: u16,
    modifiers: usize,
    text: Option<String>,
}

impl Key {
    fn parse(input: &str, mut modifiers: usize) -> Result<Self, String> {
        let mut key = input;
        // A literal plus is also a key, including in Ctrl++.
        while key != "+"
            && let Some((modifier, rest)) = key.split_once('+')
        {
            modifiers |= match modifier.to_ascii_lowercase().as_str() {
                "alt" | "option" => 1,
                "control" | "ctrl" => 2,
                "meta" | "command" | "cmd" => 4,
                "shift" => SHIFT,
                _ => {
                    return Err(format!(
                        "Unknown browser key modifier: {modifier}; use Alt, Ctrl, Meta or Shift"
                    ));
                }
            };
            key = rest;
        }
        if key.chars().count() == 1 {
            return Self::character(key.chars().next().unwrap(), modifiers);
        }
        // Names ignore case and accept the short arrow names of the process tool.
        let (name, virtual_code) = match key.to_ascii_lowercase().as_str() {
            "enter" | "return" => ("Enter", 13),
            "tab" => ("Tab", 9),
            "escape" | "esc" => ("Escape", 27),
            "backspace" => ("Backspace", 8),
            "delete" | "del" => ("Delete", 46),
            "insert" => ("Insert", 45),
            "home" => ("Home", 36),
            "end" => ("End", 35),
            "pageup" => ("PageUp", 33),
            "pagedown" => ("PageDown", 34),
            "arrowleft" | "left" => ("ArrowLeft", 37),
            "arrowup" | "up" => ("ArrowUp", 38),
            "arrowright" | "right" => ("ArrowRight", 39),
            "arrowdown" | "down" => ("ArrowDown", 40),
            "space" | "spacebar" => return Self::character(' ', modifiers),
            lower => {
                if let Some(number) = lower.strip_prefix('f').and_then(|s| s.parse::<u16>().ok())
                    && (1..=24).contains(&number)
                {
                    let name = format!("F{number}");
                    return Ok(Self::new(&name, &name, 111 + number, modifiers, None));
                }
                return Err(format!(
                    "Unknown browser key: {key}; use a character, a chord such as Ctrl+z, or one of Enter, Tab, Escape, Backspace, Delete, Insert, Home, End, PageUp, PageDown, ArrowLeft, ArrowUp, ArrowRight, ArrowDown, Space, F1-F24"
                ));
            }
        };
        Ok(Self::new(
            name,
            name,
            virtual_code,
            modifiers,
            (name == "Enter").then_some("\r"),
        ))
    }

    fn character(character: char, mut modifiers: usize) -> Result<Self, String> {
        match character {
            '\n' | '\r' => return Self::parse("Enter", modifiers),
            '\t' => return Self::parse("Tab", modifiers),
            _ if character.is_control() => {
                return Err("Browser text contains an unsupported control character".into());
            }
            _ => {}
        }
        if character.is_ascii_alphabetic() {
            // Conventional Ctrl+Z means undo; uppercase alone implies Shift.
            if character.is_ascii_uppercase() && modifiers & SHORTCUT == 0 {
                modifiers |= SHIFT;
            }
            let upper = character.to_ascii_uppercase();
            let key = if modifiers & SHIFT != 0 {
                upper
            } else {
                upper.to_ascii_lowercase()
            };
            return Ok(Self::new(
                &key.to_string(),
                &format!("Key{upper}"),
                upper as u16,
                modifiers,
                Some(&key.to_string()),
            ));
        }
        for (number, shifted) in ")!@#$%^&*(".chars().enumerate() {
            let digit = char::from(b'0' + number as u8);
            if character == digit || character == shifted {
                if character == shifted {
                    modifiers |= SHIFT;
                }
                let key = if modifiers & SHIFT != 0 {
                    shifted
                } else {
                    digit
                };
                return Ok(Self::new(
                    &key.to_string(),
                    &format!("Digit{digit}"),
                    digit as u16,
                    modifiers,
                    Some(&key.to_string()),
                ));
            }
        }
        for (plain, shifted, code, virtual_code) in [
            ('`', '~', "Backquote", 192),
            ('-', '_', "Minus", 189),
            ('=', '+', "Equal", 187),
            ('[', '{', "BracketLeft", 219),
            (']', '}', "BracketRight", 221),
            ('\\', '|', "Backslash", 220),
            (';', ':', "Semicolon", 186),
            ('\'', '"', "Quote", 222),
            (',', '<', "Comma", 188),
            ('.', '>', "Period", 190),
            ('/', '?', "Slash", 191),
        ] {
            if character == plain || character == shifted {
                if character == shifted {
                    modifiers |= SHIFT;
                }
                let key = if modifiers & SHIFT != 0 {
                    shifted
                } else {
                    plain
                };
                return Ok(Self::new(
                    &key.to_string(),
                    code,
                    virtual_code,
                    modifiers,
                    Some(&key.to_string()),
                ));
            }
        }
        // Unicode text has no universal physical key. Preserve its DOM key and
        // text, leaving code unspecified rather than inventing a keyboard key.
        let (code, virtual_code) = if character == ' ' {
            ("Space", 32)
        } else {
            ("", 0)
        };
        Ok(Self::new(
            &character.to_string(),
            code,
            virtual_code,
            modifiers,
            Some(&character.to_string()),
        ))
    }

    fn new(key: &str, code: &str, virtual_code: u16, modifiers: usize, text: Option<&str>) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            virtual_code,
            modifiers,
            text: text.map(str::to_string),
        }
    }

    fn event(&self, kind: &str) -> Value {
        let mut event = Value::object([
            ("type", Value::string(kind)),
            ("key", Value::string(&self.key)),
            ("code", Value::string(&self.code)),
            ("windowsVirtualKeyCode", Value::number(self.virtual_code)),
            ("modifiers", Value::number(self.modifiers)),
        ]);
        if kind == "keyDown"
            && let Some(text) = &self.text
            && let Value::Object(fields) = &mut event
        {
            fields.insert("unmodifiedText".into(), Value::string(text));
            if self.modifiers & SHORTCUT == 0 {
                fields.insert("text".into(), Value::string(text));
            }
        }
        // macOS normally resolves these through AppKit, which CDP bypasses.
        // Commands remain default actions: a page can cancel the key event.
        #[cfg(target_os = "macos")]
        if kind == "keyDown"
            && let Some(command) = match (self.modifiers, self.key.as_str()) {
                (4, "a") => Some("SelectAll"),
                (4, "c") => Some("Copy"),
                (4, "x") => Some("Cut"),
                (4, "v") => Some("Paste"),
                (4, "z") => Some("Undo"),
                (12, "Z") => Some("Redo"),
                _ => None,
            }
            && let Value::Object(fields) = &mut event
        {
            fields.insert(
                "commands".into(),
                Value::Array(vec![Value::string(command)]),
            );
        }
        event
    }
}
