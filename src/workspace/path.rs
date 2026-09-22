use super::Error;

/// Portable names avoid Windows streams, devices, short aliases and path ambiguity.
pub fn relative(path: &str) -> Result<String, Error> {
    if path == "." {
        return Ok(path.into());
    }
    if path.is_empty() || path.len() > 1024 || path.split('/').count() > 32 {
        return Err(Error::Path);
    }
    for name in path.split('/') {
        if name.is_empty()
            || name == ".."
            || name.ends_with(['.', ' '])
            || name
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|~".contains(c))
        {
            return Err(Error::Path);
        }
        let lower = name.to_ascii_lowercase();
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
                "target"
                    | "node_modules"
                    | "dist"
                    | "vendor"
                    | "credentials.json"
                    | "auth.json"
                    | "tokens.json"
                    | "id_rsa"
                    | "id_ed25519"
            )
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        {
            return Err(Error::Excluded);
        }
    }
    Ok(path.into())
}
