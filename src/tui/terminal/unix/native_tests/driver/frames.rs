//! Completed synchronized TUI frames for native PTY assertions.

use crate::tui::test_screen::Screen;

const BEGIN: &[u8] = b"\x1b[?2026h";
const END: &[u8] = b"\x1b[?2026l";

pub(super) struct Frames {
    pending: Vec<u8>,
    pub(super) screen: Screen,
    pub(super) complete: usize,
}

impl Frames {
    pub(super) fn new() -> Self {
        let mut screen = Screen::new(80, 24);
        screen.enter();
        Self {
            pending: Vec::new(),
            screen,
            complete: 0,
        }
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        loop {
            let Some(start) = self.pending.windows(BEGIN.len()).position(|s| s == BEGIN) else {
                let discard = self.pending.len().saturating_sub(BEGIN.len() - 1);
                self.pending.drain(..discard);
                return;
            };
            self.pending.drain(..start);
            let body = BEGIN.len();
            let Some(end) = self.pending[body..]
                .windows(END.len())
                .position(|s| s == END)
            else {
                return;
            };
            let end = body + end;
            let frame =
                std::str::from_utf8(&self.pending[body..end]).expect("complete TUI frame is UTF-8");
            self.screen.feed(frame);
            self.complete += 1;
            self.pending.drain(..end + END.len());
        }
    }
}

#[test]
fn screen_observes_only_complete_frames_across_split_utf8_and_delimiters() {
    let mut frames = Frames::new();
    let output = "prompt\x1b[?2026h\x1b[Hα ready\x1b[?2026l".as_bytes();
    let chunks: Vec<_> = output.chunks(3).collect();
    for chunk in &chunks[..chunks.len() - 1] {
        frames.push(chunk);
        assert_eq!(frames.complete, 0);
    }
    frames.push(chunks.last().unwrap());
    assert_eq!(frames.complete, 1);
    assert!(frames.screen.visible().contains("α ready"));
}
