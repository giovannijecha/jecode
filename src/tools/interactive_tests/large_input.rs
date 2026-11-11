//! Terminal input past the former 64 KiB per call reaches the program whole,
//! in order and once.
use super::*;

#[test]
fn terminal_input_beyond_64_kib_arrives_in_order_once() {
    let root = Directory::new();
    let tools = tools(&root);
    let started = execute(&tools, fixture(), &["count"], &[]);
    let id = text(&started, "process_id").to_owned();
    let mut output = text(&started, "stdout").to_owned();
    collect_until(&tools, &id, &mut output, "COUNTING");
    // Short lines keep each under any terminal line-editing buffer.
    let lines: Vec<String> = (0..3000)
        .map(|index| format!("line {index:05} {}", "abcdefghij".repeat(index % 7 + 1)))
        .collect();
    let mut input = lines.join("\r");
    input.push_str("\rEND\r");
    assert!(input.len() > 2 * 64 * 1024, "{}", input.len());
    action(&tools, &id, "write", &[("text", Value::string(&input))]);
    let (mut bytes, mut hash) = (0u64, 0xcbf29ce484222325u64);
    for byte in lines.iter().flat_map(|line| line.bytes().chain([b'\n'])) {
        bytes += 1;
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    let expected = format!("COUNT:{}:{bytes}:{hash}", lines.len());
    let started = Instant::now();
    while !output.contains("COUNT:") {
        assert!(started.elapsed() < Duration::from_secs(60), "no count yet");
        let result = action(&tools, &id, "poll", &[]);
        output.push_str(text(&result, "stdout"));
    }
    let shown = &output[output.find("COUNT:").unwrap()..];
    assert!(shown.starts_with(&expected), "{shown:.200} != {expected}");
}
