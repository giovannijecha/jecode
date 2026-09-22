//! Opt-in synthetic layout + VT serialization measurement; no terminal or provider.
use super::{
    model::{Block, Model},
    render::Renderer,
    view::Layout,
};
use std::{hint::black_box, time::Instant};

fn sample(stream: bool) -> (u128, u64, usize) {
    let mut model = Model::new(Instant::now());
    let paragraph = "A **bounded** transcript keeps `source` intact and renders useful progress.\n\n```rust\nlet value = 42; // sample\n```\n";
    for n in 0..32 {
        model.blocks.push(Block {
            speaker: "Demo",
            text: format!("# Block {n}\n{}", paragraph.repeat(8)),
        });
    }
    model.blocks.push(Block {
        speaker: "Demo",
        text: String::new(),
    });
    let mut layout = Layout::default();
    let mut renderer = Renderer::default();
    black_box(renderer.draw(layout.frame(&model, 100, 30), (100, 30), true));
    let mut checksum = 0u64;
    let mut bytes = 0;
    let start = Instant::now();
    for n in 0..80 {
        model.editor.text = format!("draft {n}");
        model.editor.cursor = model.editor.text.len();
        if stream {
            model
                .blocks
                .last_mut()
                .unwrap()
                .text
                .push_str(["New **", "streamed", " text**.", "\n"][n % 4]);
        }
        let output = renderer.draw(layout.frame(&model, 100, 30), (100, 30), true);
        bytes += output.len();
        for byte in output.bytes() {
            checksum = checksum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        black_box(output);
    }
    (start.elapsed().as_micros(), checksum, bytes)
}

#[test]
#[ignore = "manual release-mode performance probe, not a timing gate"]
fn layout_probe() {
    for stream in [false, true] {
        let (_, checksum, bytes) = sample(stream);
        let mut times = Vec::new();
        for _ in 0..7 {
            let (micros, actual_checksum, actual_bytes) = sample(stream);
            assert_eq!((actual_checksum, actual_bytes), (checksum, bytes));
            times.push(micros);
        }
        println!(
            "stream={stream} frames=80 samples_us={times:?} checksum={checksum} bytes={bytes}"
        );
    }
}
