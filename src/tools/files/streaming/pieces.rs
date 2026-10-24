//! A sequential edit view: original file ranges and borrowed replacement text.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Copy)]
enum Piece<'a> {
    Original { offset: u64, length: u64 },
    Inserted(&'a [u8]),
}

impl<'a> Piece<'a> {
    fn len(self) -> u64 {
        match self {
            Self::Original { length, .. } => length,
            Self::Inserted(bytes) => bytes.len() as u64,
        }
    }

    fn slice(self, start: u64, length: u64) -> Self {
        match self {
            Self::Original { offset, .. } => Self::Original {
                offset: offset + start,
                length,
            },
            Self::Inserted(bytes) => {
                Self::Inserted(&bytes[start as usize..(start + length) as usize])
            }
        }
    }
}

pub(super) struct View<'a> {
    pieces: Vec<Piece<'a>>,
    length: u64,
}

impl<'a> View<'a> {
    pub(super) fn new(length: u64) -> Self {
        Self {
            pieces: if length == 0 {
                Vec::new()
            } else {
                vec![Piece::Original { offset: 0, length }]
            },
            length,
        }
    }

    pub(super) fn len(&self) -> u64 {
        self.length
    }

    pub(super) fn reader<'b>(&'b self, original: &'b mut File) -> Reader<'a, 'b> {
        Reader {
            original,
            pieces: &self.pieces,
            index: 0,
            consumed: 0,
        }
    }

    pub(super) fn replace(
        &mut self,
        position: u64,
        old_length: u64,
        replacement: &'a [u8],
    ) -> Result<(), String> {
        let length = self
            .length
            .checked_sub(old_length)
            .and_then(|length| length.checked_add(replacement.len() as u64))
            .ok_or("The replacement exceeds the filesystem length range")?;
        let mut pieces = Vec::new();
        self.append_range(&mut pieces, 0, position);
        if !replacement.is_empty() {
            pieces.push(Piece::Inserted(replacement));
        }
        self.append_range(&mut pieces, position + old_length, self.length);
        self.pieces = pieces;
        self.length = length;
        Ok(())
    }

    fn append_range(&self, output: &mut Vec<Piece<'a>>, start: u64, end: u64) {
        let mut offset = 0;
        for &piece in &self.pieces {
            let next = offset + piece.len();
            let from = start.max(offset);
            let to = end.min(next);
            if from < to {
                output.push(piece.slice(from - offset, to - from));
            }
            if next >= end {
                break;
            }
            offset = next;
        }
    }
}

pub(super) struct Reader<'a, 'b> {
    original: &'b mut File,
    pieces: &'b [Piece<'a>],
    index: usize,
    consumed: u64,
}

impl Read for Reader<'_, '_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let Some(&piece) = self.pieces.get(self.index) else {
            return Ok(0);
        };
        let count = (piece.len() - self.consumed).min(output.len() as u64) as usize;
        if count == 0 {
            return Ok(0);
        }
        let count = match piece {
            Piece::Original { offset, .. } => {
                if self.consumed == 0 {
                    self.original.seek(SeekFrom::Start(offset))?;
                }
                let count = self.original.read(&mut output[..count])?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "The original file became shorter while preparing the edit",
                    ));
                }
                count
            }
            Piece::Inserted(bytes) => {
                let start = self.consumed as usize;
                output[..count].copy_from_slice(&bytes[start..start + count]);
                count
            }
        };
        self.consumed += count as u64;
        if self.consumed == piece.len() {
            self.index += 1;
            self.consumed = 0;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests;
