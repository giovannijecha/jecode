//! Linear, single-hunk diff for one exact replacement. Every changed line is shown.
use super::change::{ChangeError, Preview, fail};

pub(super) fn preview(
    path: &str,
    before: Option<&str>,
    after: &str,
) -> Result<Preview, ChangeError> {
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
    let mut diff = format!(
        "  @@ -{},{} +{},{} @@\n",
        if a.is_empty() { 0 } else { start + 1 },
        old_end - start + tail,
        if b.is_empty() { 0 } else { start + 1 },
        new_end - start + tail
    );
    for line in &a[start..prefix] {
        append(&mut diff, "  ", line);
    }
    for line in &a[prefix..old_end] {
        append(&mut diff, "- ", line);
    }
    for line in &b[prefix..new_end] {
        append(&mut diff, "+ ", line);
    }
    for line in &a[old_end..old_end + tail] {
        append(&mut diff, "  ", line);
    }
    // An approval never authorizes a truncated/hidden change. Narrow the model
    // request instead of adding a misleading ellipsis or unbounded TUI payload.
    if diff.len() > 48 * 1024 || diff.lines().count() > 400 {
        return fail(
            "complete diff exceeds the 48 KiB / 400-line preview limit; propose a smaller change",
        );
    }
    Ok(Preview {
        path: path.into(),
        create: before.is_none(),
        diff,
        added: new_end - prefix,
        removed: old_end - prefix,
    })
}
fn append(out: &mut String, sign: &str, line: &str) {
    out.push_str(sign);
    // Literal escapes prevent tabs, CRLF and absent final newlines from looking
    // like identical proposals. Unchanged source bytes are never normalized.
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
