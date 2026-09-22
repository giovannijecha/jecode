use super::Error;
use std::path::{Component, Path};

/// Syntax only; the workspace owner applies the access policy before opening.
pub fn input(path: &str) -> Result<String, Error> {
    if path.is_empty() || path.len() > 4096 || path.chars().any(hidden) {
        return Err(Error::Path);
    }
    let parsed = Path::new(path);
    if parsed.has_root() && !parsed.is_absolute() {
        return Err(Error::Path);
    }
    let mut count = 0;
    for part in parsed.components() {
        count += 1;
        match part {
            Component::Normal(value) => name(value.to_str().ok_or(Error::Path)?, false)?,
            #[cfg(windows)]
            Component::Prefix(value)
                if parsed.is_absolute()
                    && matches!(
                        value.kind(),
                        std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
                    ) => {}
            Component::Prefix(_) => return Err(Error::Path),
            _ => {}
        }
    }
    if count == 0 || count > 128 {
        return Err(Error::Path);
    }
    Ok(path.into())
}

pub(super) fn name(value: &str, generated: bool) -> Result<(), Error> {
    if value.is_empty()
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|c| hidden(c) || "\\:*?\"<>|~".contains(c))
    {
        return Err(Error::Path);
    }
    let lower = value.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    if matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit()
    {
        return Err(Error::Path);
    }
    if lower.starts_with('.')
        || matches!(
            lower.as_str(),
            "credentials.json" | "auth.json" | "tokens.json" | "id_rsa" | "id_ed25519"
        )
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|s| lower.ends_with(s))
        || generated
            && matches!(
                lower.as_str(),
                "target" | "node_modules" | "dist" | "vendor"
            )
    {
        return Err(Error::Excluded);
    }
    Ok(())
}
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Portable names avoid Windows streams, devices, short aliases and path ambiguity.
pub fn relative(path: &str) -> Result<String, Error> {
    if path == "." {
        return Ok(path.into());
    }
    if path.is_empty() || path.len() > 1024 || path.split('/').count() > 32 {
        return Err(Error::Path);
    }
    for part in path.split('/') {
        if part == "." || part == ".." {
            return Err(Error::Path);
        }
        name(part, true)?;
    }
    Ok(path.into())
}
