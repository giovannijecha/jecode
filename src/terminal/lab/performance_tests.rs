//! Opt-in release-mode measurements of the actual presentation layer. These
//! synthetic runs are evidence, not a shared-CI timing gate or terminal I/O.
use super::activity_view::{Activity, Phase};
use super::caps::{Caps, ColorDepth};
use super::composer::Footer;
use super::model::{Block, Detail, Status, Tool};
use super::render::Renderer;
use super::view::{self, Layout, Screen};
use std::fmt::Write;
use std::hint::black_box;
use std::time::{Duration, Instant};

const PAINTS: usize = 60;
const REPLAYS: usize = 5;
const REPLY: &str = "The renderer keeps a **pure** `state -> frame` path and repaints only the tail, so a long session costs the same per paint as a short one — in principle.";

fn run(turns: usize, width: usize, height: usize, caps: &Caps) -> String {
    let blocks = transcript(turns);
    let footer = Footer {
        cwd: "~/Codex/jecode".into(),
        model: "opus-5.5".into(),
        effort: "high".into(),
    };
    let size = (width, height);
    let mut renderer = Renderer::default();
    let mut layout = Layout::default();
    let first = layout.frame_bounded(&screen(&blocks, &footer, "", false), width, height, caps, 0);
    let rows = first.len();
    renderer.draw(first, size, caps.color);

    // Streaming: each paint grows the live reply by a few characters.
    let (mut build, mut draw, mut bytes) = (Vec::new(), Vec::new(), 0);
    for paint in 1..=PAINTS {
        let live = &REPLY[..floor(REPLY, paint * REPLY.len() / PAINTS)];
        let now = paint as u64 * 16;
        let start = Instant::now();
        let frame = layout.frame_bounded(
            &screen(&blocks, &footer, live, false),
            width,
            height,
            caps,
            now,
        );
        let built = Instant::now();
        let out = black_box(renderer.draw(frame, size, caps.color));
        build.push(built - start);
        draw.push(built.elapsed());
        bytes += out.len();
    }

    let mut report = String::new();
    let _ = writeln!(
        report,
        "{turns} turns · {rows} rows · {width}x{height} · budget 16ms/paint"
    );
    line(&mut report, "stream frame", &mut build);
    line(&mut report, "stream diff", &mut draw);
    let _ = writeln!(report, "  stream bytes    {} per paint", bytes / PAINTS);
    let narrow = (width.saturating_sub(20), height.saturating_sub(5));
    let preview_start = Instant::now();
    let frame = view::preview(
        &screen(&blocks, &footer, REPLY, false),
        narrow.0,
        narrow.1,
        caps,
        0,
    );
    let preview_built = preview_start.elapsed();
    let preview_bytes = black_box(renderer.preview(frame, narrow, caps.color)).len();
    let preview_painted = preview_start.elapsed() - preview_built;
    let settled_start = Instant::now();
    let frame = layout.frame_bounded(
        &screen(&blocks, &footer, REPLY, false),
        narrow.0,
        narrow.1,
        caps,
        0,
    );
    let settled_built = settled_start.elapsed();
    let settled_bytes = black_box(renderer.draw(frame, narrow, caps.color)).len();
    let settled_painted = settled_start.elapsed() - settled_built;
    let _ = writeln!(
        report,
        "  resize preview  frame {:.2}ms · bytes {:.2}ms · {} KiB",
        preview_built.as_secs_f64() * 1000.0,
        preview_painted.as_secs_f64() * 1000.0,
        preview_bytes / 1024
    );
    let _ = writeln!(
        report,
        "  resize replay   frame {:.2}ms · bytes {:.2}ms · {} KiB",
        settled_built.as_secs_f64() * 1000.0,
        settled_painted.as_secs_f64() * 1000.0,
        settled_bytes / 1024
    );
    for (label, expanded) in [("replay", false), ("replay expanded", true)] {
        let (mut times, mut size_bytes) = (Vec::new(), 0);
        for _ in 0..REPLAYS {
            let start = Instant::now();
            let frame = Layout::default().frame_bounded(
                &screen(&blocks, &footer, REPLY, expanded),
                width,
                height,
                caps,
                0,
            );
            renderer.invalidate();
            size_bytes = black_box(renderer.draw(frame, size, caps.color)).len();
            times.push(start.elapsed());
        }
        line(&mut report, label, &mut times);
        let _ = writeln!(report, "  {label:<15} {} KiB", size_bytes / 1024);
    }
    if turns == 200 {
        let long = REPLY.repeat(500);
        let long_output = (0..2_000)
            .map(|n| format!("line {n}: checked output\n"))
            .collect();
        let long_blocks = vec![
            Block::User("Inspect a long response".into()),
            Block::Tools(vec![Tool {
                verb: "Run".into(),
                subject: "long-command".into(),
                summary: "2000 lines".into(),
                status: Status::Done,
                elapsed_ms: 1200,
                detail: Detail::Output(long_output),
            }]),
        ];
        let start = Instant::now();
        let frame = Layout::default().frame_bounded(
            &screen(&long_blocks, &footer, &long, true),
            width,
            height,
            caps,
            0,
        );
        let built = start.elapsed();
        renderer.invalidate();
        let output = black_box(renderer.draw(frame, size, caps.color));
        let _ = writeln!(
            report,
            "  long reply+output frame {:.2}ms · bytes {:.2}ms · {} KiB",
            built.as_secs_f64() * 1000.0,
            (start.elapsed() - built).as_secs_f64() * 1000.0,
            output.len() / 1024
        );
    }
    report
}

/// Median and worst of a set of timings, in ms.
fn line(report: &mut String, label: &str, times: &mut [Duration]) {
    times.sort();
    let ms = |time: Duration| time.as_secs_f64() * 1_000.0;
    let _ = writeln!(
        report,
        "  {label:<15} median {:>7.2}ms · worst {:>7.2}ms",
        ms(times[times.len() / 2]),
        ms(times[times.len() - 1])
    );
}

fn floor(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Each turn: a prompt, three tools (40-line output, 30-line diff) and a
/// reply with prose, a list and a code block.
fn transcript(turns: usize) -> Vec<Block> {
    let tool = |verb: &str, subject: &str, detail: Detail| Tool {
        verb: verb.into(),
        subject: subject.into(),
        summary: String::new(),
        status: Status::Done,
        elapsed_ms: 1_200,
        detail,
    };
    let output = (1..=40)
        .map(|n| format!("test terminal::module_{n}::tests::case_{n} ... ok"))
        .collect::<Vec<_>>()
        .join("\n");
    let diff = (1..=30)
        .map(|n| match n % 3 {
            0 => format!("-    let old_{n} = value;"),
            1 => format!("+    let new_{n} = value.clone();"),
            _ => format!("     context line {n}"),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let reply = format!(
        "{REPLY}\n\n- first point with `code`\n- second point\n\n```rust\nfn main() {{\n    println!(\"hi\");\n}}\n```"
    );
    let mut blocks = Vec::new();
    for turn in 0..turns {
        blocks.push(Block::User(format!("turn {turn}: fix the failing test")));
        blocks.push(Block::Tools(vec![
            tool(
                "Read",
                "src/terminal/render.rs",
                Detail::Output(String::new()),
            ),
            tool("Run", "cargo test", Detail::Output(output.clone())),
            tool("Edit", "src/terminal/view.rs", Detail::Diff(diff.clone())),
        ]));
        blocks.push(Block::Assistant(reply.clone()));
    }
    blocks
}

/// A streaming turn at the end of the transcript.
fn screen<'a>(
    blocks: &'a [Block],
    footer: &'a Footer,
    live: &'a str,
    expanded: bool,
) -> Screen<'a> {
    Screen {
        blocks,
        live: Some(live),
        activity: Some(Activity {
            phase: Phase::Streaming,
            elapsed_ms: 1_000,
            tokens: 0,
        }),
        notice: None,
        draft: "",
        cursor: 0,
        suggestion: 0,
        footer,
        picker: None,
        expanded,
        queued: &[],
    }
}

#[test]
#[ignore = "manual release-mode presentation probe, not a timing gate"]
fn long_transcript_probe() {
    let caps = Caps {
        color: ColorDepth::TrueColor,
        ascii: false,
        reduced_motion: true,
    };
    for turns in [200, 1_000] {
        println!("{}", run(turns, 100, 30, &caps));
    }
}
