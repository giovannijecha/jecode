//! Select original, completed conversation text independently of terminal rendering.
mod targets;
pub use targets::Target;

pub fn response(archive: &crate::export::Archive) -> Result<Vec<Target>, String> {
    let source = archive
        .messages
        .lock()
        .unwrap()
        .iter()
        .rev()
        .filter(|message| {
            message.get("role").and_then(crate::json::Value::as_str) == Some("assistant")
        })
        .find_map(|message| {
            message
                .get("content")
                .and_then(crate::json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
        })
        .ok_or("No completed assistant response to copy")?;
    // Reapply the current credential mask, including after an account change.
    Ok(targets::extract(&archive.redactor.text(&source)))
}

pub fn preview(value: &str) -> String {
    let line = value
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let mut characters = line.trim().chars();
    let mut preview: String = characters
        .by_ref()
        .take(72)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if characters.next().is_some() {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests;
