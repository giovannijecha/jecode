//! Bounded display of an exact change; the display never gates execution.
use super::change::Preview;

const PREVIEW_BYTES: usize = 48 * 1024;
const PREVIEW_LINES: usize = 400;
pub(super) fn preview(path: &str, before: Option<&str>, after: &str) -> Preview {
    let a: Vec<_> = before.unwrap_or("").split_inclusive('\n').collect();
    let b: Vec<_> = after.split_inclusive('\n').collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let old_end = a.len() - suffix;
    let new_end = b.len() - suffix;
    let start = prefix.saturating_sub(3);
    let tail = suffix.min(3);
    let mut full = format!(
        "  @@ -{},{} +{},{} @@\n",
        if a.is_empty() { 0 } else { start + 1 },
        old_end - start + tail,
        if b.is_empty() { 0 } else { start + 1 },
        new_end - start + tail
    );
    for line in &a[start..prefix] {
        append(&mut full, "  ", line);
    }
    for line in &a[prefix..old_end] {
        append(&mut full, "- ", line);
    }
    for line in &b[prefix..new_end] {
        append(&mut full, "+ ", line);
    }
    for line in &a[old_end..old_end + tail] {
        append(&mut full, "  ", line);
    }
    finish(
        path,
        before.is_none(),
        full,
        new_end - prefix,
        old_end - prefix,
    )
}

/// For a staged large original, the exact old/new arguments are the changed
/// content. Surrounding file bytes are deliberately omitted from this hunk.
pub(super) fn replacement_preview(path: &str, offset: u64, old: &str, new: &str) -> Preview {
    let mut full =
        format!("  @@ byte {offset}: exact replacement; unchanged file context omitted @@\n");
    for line in old.split_inclusive('\n') {
        append(&mut full, "- ", line);
    }
    for line in new.split_inclusive('\n') {
        append(&mut full, "+ ", line);
    }
    let old_lines: Vec<_> = old.split_inclusive('\n').collect();
    let new_lines: Vec<_> = new.split_inclusive('\n').collect();
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    finish(
        path,
        false,
        full,
        new_lines.len() - prefix - suffix,
        old_lines.len() - prefix - suffix,
    )
}

fn finish(path: &str, create: bool, full: String, added: usize, removed: usize) -> Preview {
    let mut shown = 0;
    let mut lines = 0;
    for line in full.split_inclusive('\n') {
        if lines == PREVIEW_LINES || shown + line.len() > PREVIEW_BYTES {
            break;
        }
        shown += line.len();
        lines += 1;
    }
    let omitted_bytes = full.len() - shown;
    let omitted_lines = full.lines().count() - lines;
    Preview {
        path: path.into(),
        create,
        diff: full[..shown].into(),
        added,
        removed,
        omitted_lines,
        omitted_bytes,
    }
}
fn append(out: &mut String, sign: &str, line: &str) {
    out.push_str(sign);
    out.push_str(
        &line
            .trim_end_matches('\n')
            .replace('\\', "\\\\")
            .replace('\t', "\\t")
            .replace('\r', "\\r"),
    );
    out.push('\n');
    if !line.ends_with('\n') {
        out.push_str("  \\ No newline at end of file\n");
    }
}
