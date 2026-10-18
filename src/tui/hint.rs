use super::text;

/// The catalog gives each control group a few complete wordings. Keep the
/// first one that fits, falling back to its shortest key-only wording.
pub fn choose(variants: &[&str], columns: usize) -> String {
    variants
        .iter()
        .find(|value| text::cells(value) <= columns)
        .or_else(|| variants.last())
        .copied()
        .unwrap_or("")
        .to_owned()
}
