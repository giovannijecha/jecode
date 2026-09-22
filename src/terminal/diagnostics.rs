//! Explicit, bounded, numeric-only local resize diagnostics for the preview.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    time::Instant,
};
pub struct Trace {
    file: Option<File>,
    start: Instant,
    remaining: usize,
    samples: usize,
}
impl Trace {
    pub fn open() -> io::Result<Self> {
        let file = std::env::var_os("JECODE_TUI_TRACE")
            .map(|path| OpenOptions::new().write(true).create_new(true).open(path))
            .transpose()?;
        Ok(Self {
            file,
            start: Instant::now(),
            remaining: 4096,
            samples: 0,
        })
    }
    pub fn changed(&mut self) {
        self.samples = 4;
    }
    pub fn active(&self) -> bool {
        self.file.is_some() && self.samples > 0 && self.remaining > 0
    }
    pub fn record(
        &mut self,
        stage: &str,
        size: (usize, usize),
        rows: usize,
        bytes: usize,
        position: &str,
    ) -> io::Result<()> {
        if !self.active() {
            return Ok(());
        }
        if let Some(file) = &mut self.file {
            writeln!(
                file,
                "{} {stage} columns={} rows={} frame_rows={rows} bytes={bytes} {position}",
                self.start.elapsed().as_millis(),
                size.0,
                size.1
            )?;
            file.flush()?;
        }
        self.remaining -= 1;
        if stage == "after" {
            self.samples = self.samples.saturating_sub(1);
        }
        Ok(())
    }
}
