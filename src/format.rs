/// Image formats supported by `ime`, identified by magic bytes only, never by
/// file extension (see `03-business-logic.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_PREFIX: [u8; 3] = [0xFF, 0xD8, 0xFF];

/// Detect the image format from the leading bytes. Returns `None` when the
/// magic bytes do not match PNG, JPEG, or WebP.
pub fn detect(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.len() >= PNG_SIGNATURE.len() && bytes[..PNG_SIGNATURE.len()] == PNG_SIGNATURE {
        return Some(ImageFormat::Png);
    }
    if bytes.len() >= JPEG_PREFIX.len() && bytes[..JPEG_PREFIX.len()] == JPEG_PREFIX {
        return Some(ImageFormat::Jpeg);
    }
    if bytes.len() >= 12 && bytes[..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        return Some(ImageFormat::Webp);
    }
    None
}
