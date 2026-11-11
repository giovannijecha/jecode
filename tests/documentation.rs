use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const REPOSITORY_BLOB: &str = "https://github.com/giovannijecha/jecode/blob/main/";

fn public_markdown(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_file()
            && path.extension().is_some_and(|extension| extension == "md")
            && public_page(&path)
        {
            files.push(path);
        }
    }
    files.push(root.join("npm/README.md"));
    collect_markdown(&root.join(".github"), &mut files);
    collect_markdown(&root.join("docs"), &mut files);
    files.sort();
    files
}

fn collect_markdown(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let path = entry.path();
        if kind.is_dir() {
            collect_markdown(&path, files);
        } else if kind.is_file()
            && path.extension().is_some_and(|extension| extension == "md")
            && public_page(&path)
        {
            files.push(path);
        }
    }
}

fn public_page(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_ascii_uppercase();
    !matches!(
        name.as_str(),
        "AGENTS.MD" | "CLAUDE.MD" | "CODEX_HARNESS_STUDY.MD"
    ) && !name.starts_with("HARNESS")
}

fn visible_lines(
    markdown: &str,
    mask_inline_code: bool,
) -> impl Iterator<Item = (usize, String)> + '_ {
    let mut fence: Option<(u8, usize)> = None;
    let mut code_ticks = 0;
    markdown
        .lines()
        .enumerate()
        .filter_map(move |(index, line)| {
            let trimmed = line.trim_start();
            let bytes = trimmed.as_bytes();
            let marker = bytes
                .first()
                .copied()
                .filter(|byte| matches!(byte, b'`' | b'~'));
            let run = marker.map_or(0, |byte| {
                bytes.iter().take_while(|item| **item == byte).count()
            });
            if let Some((byte, minimum)) = fence {
                if marker == Some(byte) && run >= minimum {
                    fence = None;
                }
                return None;
            }
            if let Some(byte) = marker.filter(|_| run >= 3) {
                fence = Some((byte, run));
                return None;
            }
            if !mask_inline_code {
                return Some((index + 1, line.to_owned()));
            }
            // A code span never crosses a blank line, so a stray backtick cannot
            // hide the links of later paragraphs.
            if line.trim().is_empty() {
                code_ticks = 0;
            }

            let mut masked = line.as_bytes().to_vec();
            let mut position = 0;
            while position < masked.len() {
                if masked[position] == b'`' {
                    let start = position;
                    while position < masked.len() && masked[position] == b'`' {
                        position += 1;
                    }
                    let count = position - start;
                    if code_ticks == 0 {
                        code_ticks = count;
                    } else if code_ticks == count {
                        code_ticks = 0;
                    }
                    masked[start..position].fill(b' ');
                } else {
                    if code_ticks != 0 {
                        masked[position] = b' ';
                    }
                    position += 1;
                }
            }
            Some((index + 1, String::from_utf8(masked).unwrap()))
        })
}

fn links(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut links = Vec::new();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] != b']' || bytes[index + 1] != b'(' {
            index += 1;
            continue;
        }
        let start = index + 2;
        let mut cursor = start;
        let mut depth = 1;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\\' => cursor = (cursor + 2).min(bytes.len()),
                b'(' => {
                    depth += 1;
                    cursor += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    cursor += 1;
                }
                _ => cursor += 1,
            }
        }
        if depth == 0 {
            let destination = line[start..cursor].trim();
            let destination = if destination.starts_with('<') {
                destination
                    .strip_prefix('<')
                    .and_then(|rest| rest.split_once('>'))
                    .map(|(url, _)| url)
            } else {
                destination.split_ascii_whitespace().next()
            };
            if let Some(destination) = destination.filter(|value| !value.is_empty()) {
                links.push(destination.to_owned());
            }
            index = cursor + 1;
        } else {
            index = start;
        }
    }
    links
}

fn heading_slug(heading: &str) -> String {
    let heading = heading.trim().trim_end_matches('#').trim();
    let mut slug = String::new();
    for character in heading.chars() {
        if character.is_alphanumeric() || matches!(character, '-' | '_') {
            slug.extend(character.to_lowercase());
        } else if character.is_whitespace() {
            slug.push('-');
        }
    }
    slug
}

fn anchors(markdown: &str) -> HashSet<String> {
    let mut anchors = HashSet::new();
    for (_, line) in visible_lines(markdown, false) {
        let trimmed = line.trim_start();
        let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
        if !(1..=6).contains(&level) || trimmed.as_bytes().get(level) != Some(&b' ') {
            continue;
        }
        let slug = heading_slug(&trimmed[level + 1..]);
        let mut anchor = slug.clone();
        let mut suffix = 0;
        while anchors.contains(&anchor) {
            suffix += 1;
            anchor = format!("{slug}-{suffix}");
        }
        anchors.insert(anchor);
    }
    anchors
}

fn percent_decode(value: &str) -> Result<String, String> {
    let mut decoded = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(pair) = bytes.get(index + 1..index + 3) else {
                return Err(format!("invalid percent escape in {value}"));
            };
            let hex = std::str::from_utf8(pair)
                .map_err(|_| format!("invalid percent escape in {value}"))?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|_| format!("invalid percent escape in {value}"))?;
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| format!("invalid UTF-8 escape in {value}"))
}

fn local_target(
    source: &Path,
    url: &str,
    root: &Path,
) -> Result<Option<(PathBuf, String)>, String> {
    let (path, relative) = if let Some(path) = url.strip_prefix(REPOSITORY_BLOB) {
        (path, false)
    } else if url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("mailto:")
        || url.starts_with("//")
    {
        return Ok(None);
    } else {
        (url, true)
    };
    let (path, fragment) = path.split_once('#').unwrap_or((path, ""));
    let path = path.split_once('?').map_or(path, |(before, _)| before);
    let path = percent_decode(path)?;
    let fragment = percent_decode(fragment)?;
    if path.is_empty() {
        return Ok(Some((source.to_path_buf(), fragment)));
    }
    let base = if relative && !path.starts_with('/') {
        source.parent().unwrap()
    } else {
        root
    };
    let mut result = PathBuf::new();
    for component in base.join(path.trim_start_matches('/')).components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            Component::RootDir => result.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::Normal(name) => result.push(name),
        }
    }
    if !result.starts_with(root) {
        return Err(format!("link leaves repository: {url}"));
    }
    Ok(Some((result, fragment)))
}

fn exact_case_exists(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut directory = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return false;
        };
        let Ok(entries) = fs::read_dir(&directory) else {
            return false;
        };
        if !entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() == name)
        {
            return false;
        }
        directory.push(name);
    }
    directory.exists()
}

#[test]
fn public_documentation_links_and_index() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pages = public_markdown(&root);
    let index = root.join("docs/README.md");
    let index_text = fs::read_to_string(&index).unwrap();
    let indexed = visible_lines(&index_text, true)
        .flat_map(|(_, line)| links(&line))
        .filter_map(|url| local_target(&index, &url, &root).ok().flatten())
        .map(|(path, _)| path)
        .collect::<HashSet<_>>();

    let mut problems = Vec::new();
    for page in pages
        .iter()
        .filter(|page| page.starts_with(root.join("docs")) && **page != index)
    {
        if !indexed.contains(page) {
            problems.push(format!(
                "{}: missing direct link in docs/README.md",
                page.display()
            ));
        }
    }
    let mut anchor_cache = HashMap::<PathBuf, HashSet<String>>::new();
    for page in pages {
        let markdown = fs::read_to_string(&page).unwrap();
        for (line_number, line) in visible_lines(&markdown, true) {
            for url in links(&line) {
                let target = match local_target(&page, &url, &root) {
                    Ok(Some(target)) => target,
                    Ok(None) => continue,
                    Err(error) => {
                        problems.push(format!("{}:{line_number}: {error}", page.display()));
                        continue;
                    }
                };
                let (path, fragment) = target;
                if !exact_case_exists(&root, &path) {
                    problems.push(format!(
                        "{}:{line_number}: missing or wrong-case target: {url}",
                        page.display()
                    ));
                } else if !fragment.is_empty()
                    && path.extension().is_some_and(|extension| extension == "md")
                {
                    let anchors = anchor_cache
                        .entry(path.clone())
                        .or_insert_with(|| anchors(&fs::read_to_string(&path).unwrap()));
                    if !anchors.contains(&fragment) {
                        problems.push(format!(
                            "{}:{line_number}: missing heading #{fragment} in {}",
                            page.display(),
                            path.display()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "documentation links:\n{}",
        problems.join("\n")
    );
}

#[test]
fn parser_ignores_code_and_tracks_nested_destinations() {
    let markdown = "```md\n[false](missing.md)\n```\n`[false](missing.md)` [real](folder/(draft).md#part)\n## Part\n## Part\n";
    let lines = visible_lines(markdown, true).collect::<Vec<_>>();
    assert_eq!(links(&lines[0].1), ["folder/(draft).md#part"]);
    let stray = "A stray ` tick\n\n[after](after.md)\n";
    let lines = visible_lines(stray, true).collect::<Vec<_>>();
    assert_eq!(links(&lines[2].1), ["after.md"]);
    assert_eq!(
        anchors(markdown),
        HashSet::from(["part".into(), "part-1".into()])
    );
    assert_eq!(
        anchors("## Part\n## Part-1\n## Part\n"),
        HashSet::from(["part".into(), "part-1".into(), "part-2".into()])
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("docs/page.md");
    assert_eq!(
        local_target(&source, "#part", root).unwrap(),
        Some((source, "part".into()))
    );
}
