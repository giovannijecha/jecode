//! Content detection from file signatures. Extensions only refine text types;
//! they never make bytes count as an image or a PDF.

use super::Kind;

/// Raster types OpenRouter accepts as image parts.
pub const PROVIDER_IMAGES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

pub fn kind(media: &str) -> Kind {
    match media {
        "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/bmp" | "image/tiff" => {
            Kind::Image
        }
        "application/pdf" => Kind::Pdf,
        media if media.starts_with("text/") || media == "application/json" => Kind::Text,
        "image/svg+xml" => Kind::Text,
        _ => Kind::Binary,
    }
}

pub struct Detected {
    pub media: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Classifies a file from its first bytes and its name.
pub fn detect(head: &[u8], name: &str, complete: bool) -> Detected {
    let image = |media: &str, size: Option<(u32, u32)>| Detected {
        media: media.into(),
        width: size.map(|size| size.0),
        height: size.map(|size| size.1),
    };
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return image("image/png", png(head));
    }
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return image("image/jpeg", jpeg(head));
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return image(
            "image/gif",
            (head.len() >= 10).then(|| (le16(head, 6), le16(head, 8))),
        );
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return image("image/webp", webp(head));
    }
    if head.starts_with(b"BM") && head.len() >= 26 {
        let width = i32::from_le_bytes(head[18..22].try_into().unwrap());
        let height = i32::from_le_bytes(head[22..26].try_into().unwrap());
        return image(
            "image/bmp",
            Some((width.unsigned_abs(), height.unsigned_abs())),
        );
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        return image("image/tiff", None);
    }
    if head.starts_with(b"%PDF-") {
        return image("application/pdf", None);
    }
    let media = if text(head, complete) {
        text_media(name)
    } else {
        "application/octet-stream"
    };
    image(media, None)
}

fn text(head: &[u8], complete: bool) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A truncated read may end inside a character.
        Err(error) => !complete && error.error_len().is_none(),
    }
}

fn text_media(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "csv" => "text/csv",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" => "text/xml",
        "svg" => "image/svg+xml",
        _ => "text/plain",
    }
}

fn be16(bytes: &[u8], at: usize) -> u32 {
    u32::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]))
}

fn le16(bytes: &[u8], at: usize) -> u32 {
    u32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn le24(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], 0])
}

fn png(head: &[u8]) -> Option<(u32, u32)> {
    (head.len() >= 24 && &head[12..16] == b"IHDR").then(|| {
        (
            u32::from_be_bytes(head[16..20].try_into().unwrap()),
            u32::from_be_bytes(head[20..24].try_into().unwrap()),
        )
    })
}

fn jpeg(head: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < head.len() {
        if head[at] != 0xff {
            return None;
        }
        let marker = head[at + 1];
        if marker == 0xff {
            at += 1;
            continue;
        }
        if (0xd0..=0xd9).contains(&marker) || marker == 0x01 {
            at += 2;
            continue;
        }
        // Start-of-frame markers, excluding DHT, JPG and DAC.
        if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
            return Some((be16(head, at + 7), be16(head, at + 5)));
        }
        at += 2 + be16(head, at + 2) as usize;
    }
    None
}

fn webp(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 30 {
        return None;
    }
    match &head[12..16] {
        b"VP8 " => Some((le16(head, 26) & 0x3fff, le16(head, 28) & 0x3fff)),
        b"VP8L" => {
            let bits = u32::from_le_bytes(head[21..25].try_into().unwrap());
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le24(head, 24) + 1, le24(head, 27) + 1)),
        _ => None,
    }
}

/// Counts page objects. It is an estimate for context accounting; compressed
/// object streams can hide pages, so absence is not reported as zero.
pub fn pdf_pages(data: &[u8]) -> Option<u32> {
    let mut count = 0u32;
    let mut at = 0;
    while let Some(offset) = find(&data[at..], b"/Type") {
        at += offset + 5;
        let rest = &data[at..];
        let skip = rest
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        let rest = &rest[skip..];
        if rest.starts_with(b"/Page")
            && !rest.get(5).is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            count += 1;
        }
    }
    (count > 0).then_some(count)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_signatures_and_dimensions() {
        let png = crate::attachments::tests::png(3, 2);
        let detected = detect(&png, "x.bin", true);
        assert_eq!(
            (detected.media.as_str(), detected.width, detected.height),
            ("image/png", Some(3), Some(2))
        );
        let gif = b"GIF89a\x05\x00\x07\x00rest";
        assert_eq!(detect(gif, "a", true).width, Some(5));
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
        jpeg.extend([0xff, 0xc0, 0, 11, 8, 0, 20, 0, 30, 3, 0, 0]);
        let detected = detect(&jpeg, "a", true);
        assert_eq!((detected.width, detected.height), (Some(30), Some(20)));
        assert_eq!(detect(b"%PDF-1.7", "a.txt", true).media, "application/pdf");
        assert_eq!(detect(b"a,b\n1,2\n", "data.CSV", true).media, "text/csv");
        assert_eq!(
            detect(b"\x89PNx\0", "fake.png", true).media,
            "application/octet-stream"
        );
        assert_eq!(detect("é".as_bytes(), "a", true).media, "text/plain");
        assert_eq!(detect(&"é".as_bytes()[..1], "a", false).media, "text/plain");
        assert_eq!(
            detect(&"é".as_bytes()[..1], "a", true).media,
            "application/octet-stream"
        );
        assert_eq!(kind("image/svg+xml"), Kind::Text);
    }

    #[test]
    fn counts_pdf_pages_without_counting_page_trees() {
        let pdf = b"%PDF-1.4 /Type /Pages /Type /Page /Type/Page /Type /PageLabel";
        assert_eq!(pdf_pages(pdf), Some(2));
        assert_eq!(pdf_pages(b"%PDF-1.5 compressed"), None);
    }
}
