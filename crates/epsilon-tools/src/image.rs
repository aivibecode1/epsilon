//! Image bytes: magic-number MIME detection + base64 (pi `harness/tools/image.ts`).
//!
//! Detection mirrors pi exactly: JPEG (SOI, minus the `0xF7` fourth-byte
//! exclusion), still PNG (IHDR + no `acTL` before `IDAT`), GIF, WebP
//! (`RIFF....WEBP`), BMP (validated header). Anything else is `None` —
//! callers fall back to text/binary handling, never guess a MIME type.

/// Detect a directly-sendable image MIME type from magic bytes.
/// Returns `None` for non-images AND for formats providers reject inline
/// (animated PNG — pi excludes it because animation frames break the still
/// contract; BMP is detected but left to the caller's converter path).
pub fn detect_supported_image_mime(buffer: &[u8]) -> Option<&'static str> {
    if buffer.len() >= 4
        && buffer[0] == 0xFF
        && buffer[1] == 0xD8
        && buffer[2] == 0xFF
        && buffer[3] != 0xF7
    {
        return Some("image/jpeg");
    }
    if starts_with(buffer, &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        && is_png(buffer)
        && !is_animated_png(buffer)
    {
        return Some("image/png");
    }
    if starts_with_ascii(buffer, 0, "GIF") {
        return Some("image/gif");
    }
    if starts_with_ascii(buffer, 0, "RIFF") && starts_with_ascii(buffer, 8, "WEBP") {
        return Some("image/webp");
    }
    if starts_with_ascii(buffer, 0, "BM") && is_bmp(buffer) {
        return Some("image/bmp");
    }
    None
}

fn starts_with(buffer: &[u8], bytes: &[u8]) -> bool {
    buffer.len() >= bytes.len() && &buffer[..bytes.len()] == bytes
}

fn starts_with_ascii(buffer: &[u8], offset: usize, text: &str) -> bool {
    buffer.len() >= offset + text.len() && &buffer[offset..offset + text.len()] == text.as_bytes()
}

fn read_u32be(buffer: &[u8], offset: usize) -> u32 {
    (u32::from(buffer[offset])) * 0x1000000
        + (u32::from(buffer[offset + 1]) << 16)
        + (u32::from(buffer[offset + 2]) << 8)
        + u32::from(buffer[offset + 3])
}

fn read_u16le(buffer: &[u8], offset: usize) -> u16 {
    u16::from(buffer[offset]) + (u16::from(buffer[offset + 1]) << 8)
}

fn read_u32le(buffer: &[u8], offset: usize) -> u32 {
    u32::from(buffer[offset])
        + (u32::from(buffer[offset + 1]) << 8)
        + (u32::from(buffer[offset + 2]) << 16)
        + u32::from(buffer[offset + 3]) * 0x1000000
}

const PNG_SIGNATURE_LEN: usize = 8;

fn is_png(buffer: &[u8]) -> bool {
    buffer.len() >= 16
        && read_u32be(buffer, PNG_SIGNATURE_LEN) == 13
        && starts_with_ascii(buffer, 12, "IHDR")
}

/// Report whether `buffer` holds an animated PNG (`acTL` before `IDAT`).
///
/// `detect_supported_image_mime` deliberately returns `None` for these — animation
/// frames break the still-image contract providers accept — so the read tool checks
/// this separately to refuse honestly instead of falling through to binary handling.
pub fn is_animated_png(buffer: &[u8]) -> bool {
    if !starts_with(buffer, &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) || !is_png(buffer) {
        return false;
    }
    let mut offset = PNG_SIGNATURE_LEN;
    while offset + 8 <= buffer.len() {
        let chunk_len = read_u32be(buffer, offset) as usize;
        let chunk_type = offset + 4;
        if starts_with_ascii(buffer, chunk_type, "acTL") {
            return true;
        }
        if starts_with_ascii(buffer, chunk_type, "IDAT") {
            return false;
        }
        let next = offset + 8 + chunk_len + 4;
        if next <= offset || next > buffer.len() {
            return false;
        }
        offset = next;
    }
    false
}

fn is_bmp(buffer: &[u8]) -> bool {
    if buffer.len() < 26 {
        return false;
    }
    let declared_size = read_u32le(buffer, 2);
    let pixel_offset = read_u32le(buffer, 10);
    let dib_size = read_u32le(buffer, 14);
    if declared_size != 0 && declared_size < 26 {
        return false;
    }
    if pixel_offset < 14 + dib_size {
        return false;
    }
    if declared_size != 0 && pixel_offset > declared_size {
        return false;
    }
    let (planes, bpp) = if dib_size == 12 {
        if buffer.len() < 26 {
            return false;
        }
        (read_u16le(buffer, 22), read_u16le(buffer, 24))
    } else if (40..=124).contains(&dib_size) {
        if buffer.len() < 30 {
            return false;
        }
        (read_u16le(buffer, 26), read_u16le(buffer, 28))
    } else {
        return false;
    };
    planes == 1 && [1, 4, 8, 16, 24, 32].contains(&bpp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(animated: bool) -> Vec<u8> {
        // Minimal IHDR (len 13) + optional acTL + IDAT — enough for the walker.
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend([0, 0, 0, 13]);
        v.extend(b"IHDR");
        v.extend([0u8; 13 + 4]);
        if animated {
            v.extend([0, 0, 0, 8]);
            v.extend(b"acTL");
            v.extend([0u8; 8 + 4]);
        }
        v.extend([0, 0, 0, 0]);
        v.extend(b"IDAT");
        v
    }

    #[test]
    fn detects_still_png_rejects_animated() {
        assert_eq!(
            detect_supported_image_mime(&png_bytes(false)),
            Some("image/png")
        );
        assert_eq!(detect_supported_image_mime(&png_bytes(true)), None);
    }

    #[test]
    fn detects_jpeg_gif_webp_bmp_and_rejects_other() {
        assert_eq!(
            detect_supported_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some("image/jpeg")
        );
        // pi's 0xF7 fourth-byte exclusion.
        assert_eq!(detect_supported_image_mime(&[0xFF, 0xD8, 0xFF, 0xF7]), None);
        assert_eq!(detect_supported_image_mime(b"GIF89a..."), Some("image/gif"));
        let mut webp = b"RIFF....WEBP".to_vec();
        webp[4..8].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(detect_supported_image_mime(&webp), Some("image/webp"));
        // Minimal 24-bit BMP: BM + declared size 54 + pixel offset 54 + DIB 40
        // (offset == size is valid: no pixel bytes follow in the fixture).
        let mut bmp = vec![0u8; 54];
        bmp[0] = b'B';
        bmp[1] = b'M';
        bmp[2..6].copy_from_slice(&54u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14] = 40;
        bmp[26] = 1;
        bmp[28] = 24;
        assert_eq!(detect_supported_image_mime(&bmp), Some("image/bmp"));
        assert_eq!(detect_supported_image_mime(b"hello world"), None);
        assert_eq!(detect_supported_image_mime(&[]), None);
    }
}
