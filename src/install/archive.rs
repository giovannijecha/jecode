//! Reading npm's release tarball: gzip (RFC 1952) around DEFLATE (RFC 1951)
//! around a ustar archive. Only regular files are returned, by name; paths
//! are never used to write to disk.

use std::collections::BTreeMap;

const MAX_BITS: usize = 15;

struct Bits<'a> {
    data: &'a [u8],
    position: usize,
    buffer: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            buffer: 0,
            count: 0,
        }
    }

    fn need(&mut self, count: u32) -> Result<(), String> {
        while self.count < count {
            let byte = *self
                .data
                .get(self.position)
                .ok_or("Truncated compressed data")?;
            self.position += 1;
            self.buffer |= u64::from(byte) << self.count;
            self.count += 8;
        }
        Ok(())
    }

    fn bits(&mut self, count: u32) -> Result<u32, String> {
        if count == 0 {
            return Ok(0);
        }
        self.need(count)?;
        let value = (self.buffer & ((1u64 << count) - 1)) as u32;
        self.buffer >>= count;
        self.count -= count;
        Ok(value)
    }

    /// Drops the rest of the current byte; stored blocks start byte-aligned.
    fn align(&mut self) {
        let drop = self.count % 8;
        self.buffer >>= drop;
        self.count -= drop;
    }

    /// Bytes consumed, excluding whole bytes still buffered.
    fn consumed(&self) -> usize {
        self.position - (self.count / 8) as usize
    }
}

struct Huffman {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, String> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &length in lengths {
            counts[usize::from(length)] += 1;
        }
        let mut left = 1i32;
        for count in counts.iter().skip(1) {
            left = (left << 1) - i32::from(*count);
            if left < 0 {
                return Err("Invalid compressed data: oversubscribed code".into());
            }
        }
        let mut offsets = [0u16; MAX_BITS + 2];
        for length in 1..=MAX_BITS {
            offsets[length + 1] = offsets[length] + counts[length];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                let slot = &mut offsets[usize::from(length)];
                symbols[usize::from(*slot)] = symbol as u16;
                *slot += 1;
            }
        }
        counts[0] = 0;
        Ok(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for length in 1..=MAX_BITS {
            code |= bits.bits(1)? as i32;
            let count = i32::from(self.counts[length]);
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("Invalid compressed data: unknown code".into())
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Inflates a raw DEFLATE stream; returns the output and bytes consumed.
pub fn inflate(data: &[u8], limit: usize) -> Result<(Vec<u8>, usize), String> {
    let mut bits = Bits::new(data);
    let mut output = Vec::new();
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => stored(&mut bits, &mut output, limit)?,
            1 => {
                let mut lengths = [0u8; 288];
                lengths[..144].fill(8);
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                lengths[280..].fill(8);
                let literals = Huffman::new(&lengths)?;
                let distances = Huffman::new(&[5u8; 30])?;
                codes(&mut bits, &mut output, &literals, &distances, limit)?;
            }
            2 => {
                let (literals, distances) = dynamic(&mut bits)?;
                codes(&mut bits, &mut output, &literals, &distances, limit)?;
            }
            _ => return Err("Invalid compressed data: reserved block type".into()),
        }
        if last {
            break;
        }
    }
    Ok((output, bits.consumed()))
}

fn stored(bits: &mut Bits<'_>, output: &mut Vec<u8>, limit: usize) -> Result<(), String> {
    bits.align();
    let length = bits.bits(16)? as usize;
    let complement = bits.bits(16)? as usize;
    if length != !complement & 0xffff {
        return Err("Invalid compressed data: stored length".into());
    }
    let start = bits.consumed();
    debug_assert_eq!(bits.count, 0);
    let block = bits
        .data
        .get(start..start + length)
        .ok_or("Truncated compressed data")?;
    if output.len() + length > limit {
        return Err("Decompressed data exceeds its limit".into());
    }
    output.extend_from_slice(block);
    bits.position = start + length;
    Ok(())
}

fn dynamic(bits: &mut Bits<'_>) -> Result<(Huffman, Huffman), String> {
    let literal_count = bits.bits(5)? as usize + 257;
    let distance_count = bits.bits(5)? as usize + 1;
    let code_count = bits.bits(4)? as usize + 4;
    if literal_count > 286 || distance_count > 30 {
        return Err("Invalid compressed data: code counts".into());
    }
    let mut code_lengths = [0u8; 19];
    for &index in ORDER.iter().take(code_count) {
        code_lengths[index] = bits.bits(3)? as u8;
    }
    let lengths_code = Huffman::new(&code_lengths)?;
    let mut lengths = vec![0u8; literal_count + distance_count];
    let mut index = 0;
    while index < lengths.len() {
        let symbol = lengths_code.decode(bits)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                let previous = *index
                    .checked_sub(1)
                    .and_then(|previous| lengths.get(previous))
                    .ok_or("Invalid compressed data: repeat without length")?;
                (previous, 3 + bits.bits(2)? as usize)
            }
            17 => (0, 3 + bits.bits(3)? as usize),
            _ => (0, 11 + bits.bits(7)? as usize),
        };
        if index + repeat > lengths.len() {
            return Err("Invalid compressed data: too many lengths".into());
        }
        lengths[index..index + repeat].fill(value);
        index += repeat;
    }
    if lengths[256] == 0 {
        return Err("Invalid compressed data: no end-of-block code".into());
    }
    Ok((
        Huffman::new(&lengths[..literal_count])?,
        Huffman::new(&lengths[literal_count..])?,
    ))
}

fn codes(
    bits: &mut Bits<'_>,
    output: &mut Vec<u8>,
    literals: &Huffman,
    distances: &Huffman,
    limit: usize,
) -> Result<(), String> {
    loop {
        let symbol = literals.decode(bits)?;
        match symbol {
            0..=255 => {
                if output.len() >= limit {
                    return Err("Decompressed data exceeds its limit".into());
                }
                output.push(symbol as u8);
            }
            256 => return Ok(()),
            _ => {
                let index = usize::from(symbol - 257);
                if index >= 29 {
                    return Err("Invalid compressed data: length code".into());
                }
                let length = usize::from(LENGTH_BASE[index])
                    + bits.bits(u32::from(LENGTH_EXTRA[index]))? as usize;
                let code = usize::from(distances.decode(bits)?);
                if code >= 30 {
                    return Err("Invalid compressed data: distance code".into());
                }
                let distance = usize::from(DISTANCE_BASE[code])
                    + bits.bits(u32::from(DISTANCE_EXTRA[code]))? as usize;
                if distance > output.len() {
                    return Err("Invalid compressed data: distance too far".into());
                }
                if output.len() + length > limit {
                    return Err("Decompressed data exceeds its limit".into());
                }
                let start = output.len() - distance;
                for offset in 0..length {
                    let byte = output[start + offset];
                    output.push(byte);
                }
            }
        }
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let mut value = index as u32;
        for _ in 0..8 {
            value = if value & 1 == 1 {
                0xedb8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
        }
        *entry = value;
    }
    !data.iter().fold(!0u32, |crc, &byte| {
        table[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8)
    })
}

pub fn gunzip(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return Err("Not a gzip archive".into());
    }
    let flags = data[3];
    let mut position = 10;
    let byte_at = |position: usize| data.get(position).copied().ok_or("Truncated gzip header");
    if flags & 4 != 0 {
        let length = usize::from(byte_at(position)?) | usize::from(byte_at(position + 1)?) << 8;
        position += 2 + length;
    }
    for flag in [8u8, 16] {
        if flags & flag != 0 {
            while byte_at(position)? != 0 {
                position += 1;
            }
            position += 1;
        }
    }
    if flags & 2 != 0 {
        position += 2;
    }
    let body = data.get(position..).ok_or("Truncated gzip header")?;
    let (output, used) = inflate(body, limit)?;
    let trailer = body.get(used..used + 8).ok_or("Truncated gzip trailer")?;
    let crc = u32::from_le_bytes(trailer[..4].try_into().unwrap());
    let size = u32::from_le_bytes(trailer[4..].try_into().unwrap());
    if crc != crc32(&output) || size != output.len() as u32 {
        return Err("The gzip archive failed its checksum".into());
    }
    Ok(output)
}

/// Regular files of a ustar archive by full name. Links, devices and
/// repeated or traversing names are rejected rather than skipped.
pub fn untar(tar: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let mut offset = 0;
    while offset + 512 <= tar.len() && tar[offset] != 0 {
        let header = &tar[offset..offset + 512];
        let text = |start: usize, length: usize| {
            let field = &header[start..start + length];
            let end = field.iter().position(|byte| *byte == 0).unwrap_or(length);
            String::from_utf8_lossy(&field[..end]).into_owned()
        };
        let stored: u32 = text(148, 8)
            .trim()
            .chars()
            .try_fold(0u32, |sum, digit| {
                digit.to_digit(8).map(|digit| sum * 8 + digit)
            })
            .ok_or("Invalid tar header checksum")?;
        let actual: u32 = header
            .iter()
            .enumerate()
            .map(|(index, byte)| {
                if (148..156).contains(&index) {
                    32
                } else {
                    u32::from(*byte)
                }
            })
            .sum();
        if stored != actual {
            return Err("Invalid tar header checksum".into());
        }
        let size = text(124, 12)
            .trim()
            .chars()
            .try_fold(0usize, |sum, digit| {
                digit.to_digit(8).map(|digit| sum * 8 + digit as usize)
            })
            .ok_or("Unsupported tar entry size")?;
        let prefix = text(345, 155);
        let name = text(0, 100);
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let start = offset + 512;
        let end = start.checked_add(size).ok_or("Invalid tar entry size")?;
        if end > tar.len() {
            return Err("Truncated tar archive".into());
        }
        match header[156] {
            b'0' | 0 => {
                if name.contains('\\')
                    || name.starts_with('/')
                    || name.split('/').any(|part| part == ".." || part == ".")
                    || files.contains_key(&name)
                {
                    return Err(format!("Invalid or repeated archive path: {name}"));
                }
                files.insert(name, tar[start..end].to_vec());
            }
            b'5' | b'x' | b'g' => {}
            _ => return Err(format!("Unsupported archive entry: {name}")),
        }
        offset = start + size.div_ceil(512) * 512;
    }
    Ok(files)
}

#[cfg(test)]
pub(crate) mod tests;
