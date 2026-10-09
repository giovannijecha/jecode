use super::{Target, extract};

fn parts(source: &str) -> Vec<Target> {
    let targets = extract(source);
    assert_eq!(targets[0].name, "Whole response");
    assert_eq!(targets[0].text, source);
    targets.into_iter().skip(1).collect()
}

#[test]
fn whole_response_is_exact_even_without_blocks() {
    assert!(parts("  café\r\nlast  ").is_empty());
    assert!(parts("").is_empty());
}

#[test]
fn fenced_code_preserves_whitespace_and_line_endings() {
    let blocks = parts("before\r\n```rust extra info\r\n  let x = \"é\";  \r\n\r\n```\r\nafter");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].name, "Code block 1 · rust");
    assert_eq!(blocks[0].text, "  let x = \"é\";  \r\n\r\n");
}

#[test]
fn longer_fences_and_shorter_literal_fences() {
    let blocks = parts("~~~~~text\n~~~\ncode\n~~~~\n~~~~~~\n````\n```\nvalue\n````");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "~~~\ncode\n~~~~\n");
    assert_eq!(blocks[1].text, "```\nvalue\n");
}

#[test]
fn unclosed_code_is_copyable_and_empty_blocks_are_not_offered() {
    let blocks = parts("```\n```\n~~~\ntail");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].name, "Code block 2");
    assert_eq!(blocks[0].text, "tail");
    assert!(parts(">").is_empty());
    let blocks = parts("> ```\n> ```");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].name, "Blockquote 1");
    assert_eq!(blocks[0].text, "```\n```");
}

#[test]
fn quoted_markdown_and_nested_quote_levels_are_preserved() {
    let blocks = parts("> **Bold**  \r\n> > nested\r\n>  indented\r\n\r\n> second\r\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].name, "Blockquote 1");
    assert_eq!(blocks[0].text, "**Bold**  \r\n> nested\r\n indented\r\n");
    assert_eq!(blocks[1].name, "Blockquote 2");
    assert_eq!(blocks[1].text, "second\r\n");
}

#[test]
fn quoted_fences_follow_their_quote_in_source_order() {
    let blocks = parts(
        "> Intro\n> ```rust\n>   let n = 1;\n> ```\n> > ~~~json\n> > {\"ok\":true}\n> > ~~~\n```txt\noutside\n```",
    );
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[0].name, "Blockquote 1");
    assert_eq!(
        blocks[0].text,
        "Intro\n```rust\n  let n = 1;\n```\n> ~~~json\n> {\"ok\":true}\n> ~~~\n"
    );
    assert_eq!(blocks[1].name, "Code block 1 · rust");
    assert_eq!(blocks[1].text, "  let n = 1;\n");
    assert_eq!(blocks[2].name, "Code block 2 · json");
    assert_eq!(blocks[2].text, "{\"ok\":true}\n");
    assert_eq!(blocks[3].name, "Code block 3 · txt");
    assert_eq!(blocks[3].text, "outside\n");
}

#[test]
fn quoted_code_keeps_mixed_line_endings_and_trailing_spaces() {
    let blocks = parts("> ~~~\r\n> α  \n> β\r\n> ~~~\r\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "~~~\r\nα  \nβ\r\n~~~\r\n");
    assert_eq!(blocks[1].text, "α  \nβ\r\n");
}

#[test]
fn quote_markers_inside_root_code_are_literal() {
    let blocks = parts("```\n> not a quote\n```\n> actual");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "> not a quote\n");
    assert_eq!(blocks[1].text, "actual");
}

#[test]
fn nested_quote_fence_stops_when_its_container_ends() {
    let blocks = parts("> > ```\n> > nested\n> outside\n> ```\n> root\n> ```");
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[1].text, "nested\n");
    assert_eq!(blocks[2].text, "root\n");
}

#[test]
fn large_body_is_not_truncated() {
    let body = "λ  \r\n".repeat(20_000);
    let source = format!("```text\r\n{body}```");
    let blocks = parts(&source);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, body);
}
