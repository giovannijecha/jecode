//! Paginated UTF-8 reads keep only the requested output window in memory.
use super::{Budget, Error, Workspace, platform};
use std::io::Read;

pub struct Page {
    pub text: String,
    pub count: usize,
    pub truncated: bool,
    pub long_line: bool,
}
impl Workspace {
    pub fn read_page(
        &self,
        path: &str,
        start: usize,
        limit: usize,
        budget: &Budget<'_>,
    ) -> Result<Page, Error> {
        budget.check()?;
        if start == 0 || limit == 0 {
            return Err(Error::Path);
        }
        let path = self.resolve(path)?;
        let mut opened = self.open_location(&path, false)?;
        let before = opened.file.metadata().map_err(|_| Error::Unavailable)?;
        if !before.is_file() {
            return Err(Error::Unavailable);
        }
        let identity = platform::identity(&opened.file).map_err(|_| Error::Unavailable)?;
        let mut validator = Utf8::default();
        let mut buffer = [0; 16 * 1024];
        let mut text = Vec::new();
        let mut line = Vec::new();
        let mut line_no = 1usize;
        let mut total_lines = 0usize;
        let mut count = 0usize;
        let mut blocked = false;
        let mut long_line = false;
        let mut last = None;
        let mut size = 0u64;
        loop {
            budget.check()?;
            let n = opened
                .file
                .read(&mut buffer)
                .map_err(|_| Error::Unavailable)?;
            if n == 0 {
                break;
            }
            size = size.checked_add(n as u64).ok_or(Error::Size)?;
            validator.push(&buffer[..n])?;
            for &byte in &buffer[..n] {
                last = Some(byte);
                if line_no >= start && count < limit && !blocked {
                    if line.len() < 8192 {
                        line.push(byte);
                    } else {
                        blocked = true;
                        long_line = count == 0;
                        line.clear();
                    }
                }
                if byte == b'\n' {
                    total_lines = total_lines.saturating_add(1);
                    if line_no >= start && count < limit && !blocked {
                        if text.len() + line.len() <= 8192 {
                            text.extend_from_slice(&line);
                            count += 1;
                        } else {
                            blocked = true;
                        }
                    }
                    line.clear();
                    line_no = line_no.saturating_add(1);
                }
            }
        }
        if last.is_some_and(|byte| byte != b'\n') {
            total_lines = total_lines.saturating_add(1);
            if line_no >= start && count < limit && !blocked && text.len() + line.len() <= 8192 {
                text.extend_from_slice(&line);
                count += 1;
            }
        }
        validator.finish()?;
        let after = opened.file.metadata().map_err(|_| Error::Unavailable)?;
        budget.check()?;
        if size != before.len()
            || after.len() != before.len()
            || before.modified().ok() != after.modified().ok()
            || platform::identity(&opened.file).map_err(|_| Error::Unavailable)? != identity
        {
            return Err(Error::Changed);
        }
        Ok(Page {
            text: String::from_utf8(text).map_err(|_| Error::Text)?,
            count,
            truncated: total_lines >= start.saturating_add(count),
            long_line,
        })
    }
}
#[derive(Default)]
struct Utf8 {
    pending: Vec<u8>,
}
impl Utf8 {
    fn push(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let mut chunk = std::mem::take(&mut self.pending);
        chunk.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&chunk) {
            Ok(_) => chunk.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => return Err(Error::Text),
        };
        if std::str::from_utf8(&chunk[..valid])
            .unwrap()
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(Error::Text);
        }
        self.pending.extend_from_slice(&chunk[valid..]);
        Ok(())
    }
    fn finish(&self) -> Result<(), Error> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(Error::Text)
        }
    }
}
