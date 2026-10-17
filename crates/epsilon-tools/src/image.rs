//! Image bytes: magic-number MIME detection + resize toward model limits + base64
//! (pi `harness/tools/image.ts`, `utils/image-resize-core.ts`).
//!
//! Detection mirrors pi exactly: JPEG (SOI, minus the `0xF7` fourth-byte
//! exclusion), still PNG (IHDR + no `acTL` before `IDAT`), GIF, WebP
//! (`RIFF....WEBP`), BMP (validated header). Anything else is `None` —
//! callers fall back to text/binary handling, never guess a MIME type.
//!
//! Resize (pi `processImage` + `resizeImageInProcess`, simplified): images over
//! the caps are downscaled aspect-fit and re-encoded as PNG. No model metadata
//! reaches the tool layer (`ToolContext` carries no model), so the caps are
//! always pi's fallback defaults — see [`default_resize_options`].
//!
//! ponytail: the decoder is the `image` crate already compiled into this
//! binary via arboard (same 0.25.10 in `Cargo.lock` — declaring it adds no new
//! source, only the name). Only PNG/JPEG round-trip: GIF/WebP pass through at
//! original size (re-encoding an animated GIF would silently drop frames, and
//! WebP has no decoder under the enabled features). No JPEG-quality ladder —
//! PNG-only output, shrinking dimensions until the byte cap fits. Upgrade
//! trigger: a real read whose resized PNG is uglier or larger than a JPEG
//! ladder would give.

/// Caps an inline image must fit (pi `ImageResizeOptions`,
/// `image-resize-core.ts:8-14`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageResizeOptions {
    /// Long-edge cap in pixels (pi default: 2000).
    pub max_width: u32,
    /// Short-edge cap in pixels (pi default: 2000).
    pub max_height: u32,
    /// Base64 payload cap in bytes (pi default: 4.5 MiB).
    pub max_bytes: usize,
}

/// The fallback caps: pi's `DEFAULT_OPTIONS` (`image-resize-core.ts:24-29`).
///
/// This is also the *only* caps the read tool can use — `ToolContext` carries
/// no model handle, so per-model `inputLimits.images.resize` (pi
/// `read.ts:117`) has nothing to read here. When a model-metadata seam reaches
/// this layer, the read branch should prefer it and keep this as the fallback.
pub fn default_resize_options() -> ImageResizeOptions {
    ImageResizeOptions {
        max_width: 2000,
        max_height: 2000,
        max_bytes: 4_718_592, // 4.5 MiB of base64 payload, below Anthropic's 5 MB limit
    }
}

/// Image bytes fitted to the caps: resized or passed through untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizedImage {
    /// Bytes to attach (re-encoded PNG when resized, the input otherwise).
    pub bytes: Vec<u8>,
    /// MIME of `bytes` (`image/png` after a resize, the input MIME otherwise).
    pub mime: String,
    /// Pixel size before the fit.
    pub original_width: u32,
    /// Pixel size before the fit.
    pub original_height: u32,
    /// Pixel size of `bytes`.
    pub width: u32,
    /// Pixel size of `bytes`.
    pub height: u32,
    /// Whether the bytes changed at all (dimensions or format).
    pub was_resized: bool,
}

impl ResizedImage {
    /// The coordinate-mapping note for the model (pi `formatDimensionNote`,
    /// `image-resize.ts:116-123`). `None` when nothing changed.
    pub fn dimension_note(&self) -> Option<String> {
        if !self.was_resized {
            return None;
        }
        let scale = self.original_width as f64 / self.width as f64;
        Some(format!(
            "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {:.2} to map to original image.]",
            self.original_width, self.original_height, self.width, self.height, scale
        ))
    }
}

/// Shrink `bytes` to fit `options`, or hand them back untouched.
///
/// Never refuses: undecodable bytes, formats without a decoder (GIF/WebP),
/// and images already inside the caps all return the input byte-identical —
/// the read branch attached them as-is before this port and still does.
pub fn resize_to_limits(bytes: &[u8], mime: &str, options: &ImageResizeOptions) -> ResizedImage {
    let passthrough = || ResizedImage {
        bytes: bytes.to_vec(),
        mime: mime.to_string(),
        original_width: 0,
        original_height: 0,
        width: 0,
        height: 0,
        was_resized: false,
    };
    // Still formats the in-tree decoder round-trips losslessly. Anything else
    // (notably GIF: multi-frame, and WebP: no decoder) passes through.
    if mime != "image/png" && mime != "image/jpeg" {
        return passthrough();
    }
    let Ok(img) = image::load_from_memory(bytes) else {
        return passthrough();
    };
    let (original_width, original_height) = (img.width(), img.height());
    // Pi compares the base64 payload size (`resizeImageInProcess`:65), because
    // that is what the provider counts.
    let encoded_len = bytes.len().div_ceil(3) * 4;
    if original_width <= options.max_width
        && original_height <= options.max_height
        && encoded_len < options.max_bytes
    {
        return passthrough();
    }
    // Aspect-fit to the dimension caps (pi `image-resize-core.ts:99-106`;
    // `Math.round` there, so round here too — never truncate).
    let mut target_width = original_width;
    let mut target_height = original_height;
    if target_width > options.max_width {
        target_height = ((u64::from(target_height) * u64::from(options.max_width)
            + u64::from(target_width) / 2)
            / u64::from(target_width)) as u32;
        target_width = options.max_width;
    }
    if target_height > options.max_height {
        target_width = ((u64::from(target_width) * u64::from(options.max_height)
            + u64::from(target_height) / 2)
            / u64::from(target_height)) as u32;
        target_height = options.max_height;
    }
    // Shrink by 3/4 steps until the PNG re-encode fits (pi `:146-154` without
    // the JPEG-quality ladder — PNG only, see the module docs). A 1x1 PNG is
    // ~100 bytes, so the loop always lands; the last candidate wins.
    let mut current_width = target_width.max(1);
    let mut current_height = target_height.max(1);
    loop {
        // Exact dimensions: the fit above already preserves the aspect ratio,
        // so this is a scale, not a stretch (`resize` would re-fit and drift
        // by a pixel against the reported size).
        let small = img.resize_exact(
            current_width,
            current_height,
            image::imageops::FilterType::Lanczos3,
        );
        let mut encoded = Vec::new();
        let png = image::codecs::png::PngEncoder::new(&mut encoded);
        let rgba = small.to_rgba8();
        if image::ImageEncoder::write_image(
            png,
            &rgba,
            current_width,
            current_height,
            image::ExtendedColorType::Rgba8,
        )
        .is_ok()
            && encoded.len().div_ceil(3) * 4 < options.max_bytes
        {
            return ResizedImage {
                bytes: encoded,
                mime: "image/png".to_string(),
                original_width,
                original_height,
                width: current_width,
                height: current_height,
                was_resized: true,
            };
        }
        if current_width == 1 && current_height == 1 {
            // Unreachable in practice (a 1x1 PNG always fits) but never refuse:
            // hand back the smallest candidate rather than an error.
            return ResizedImage {
                bytes: encoded,
                mime: "image/png".to_string(),
                original_width,
                original_height,
                width: current_width,
                height: current_height,
                was_resized: true,
            };
        }
        current_width = (current_width / 4 * 3).max(1);
        current_height = (current_height / 4 * 3).max(1);
    }
}

/// PDF magic for the read-as-attachment branch (opencode `util/media.ts:20`).
///
/// `%PDF-` opens every versioned PDF (`%PDF-1.4` … `%PDF-2.0`); the read tool
/// checks this alongside the image branch, so a PDF attaches the same way an
/// image does instead of paging as text.
pub fn is_pdf(buffer: &[u8]) -> bool {
    buffer.starts_with(b"%PDF-")
}

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

    // ---- resize-toward-limits (RED-first) --------------------------------------

    /// A real encodable PNG, `w` x `h`, deterministic noise so it is not trivially tiny.
    fn encodable_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(w, h, |x, y| {
            image::Rgb([(x % 251) as u8, (y % 251) as u8, 77u8])
        }));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    #[test]
    fn oversized_png_shrinks_toward_limits() {
        // 3000px wide is over the 2000px fallback cap: dimensions must shrink,
        // aspect preserved, and the payload must decode to the reported size.
        let bytes = encodable_png(3000, 100);
        let out = resize_to_limits(&bytes, "image/png", &default_resize_options());
        assert!(out.was_resized, "a 3000px-wide PNG must shrink");
        assert_eq!((out.width, out.height), (2000, 67), "aspect-fit to the cap");
        assert_eq!(out.mime, "image/png");
        let decoded = image::load_from_memory(&out.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2000, 67));
    }

    #[test]
    fn unknown_limits_use_the_fallback() {
        // No model metadata reaches the tool layer, so these pi
        // `image-resize-core.ts:24-29` defaults ARE the limits. Pin the values.
        let opts = default_resize_options();
        assert_eq!(opts.max_width, 2000);
        assert_eq!(opts.max_height, 2000);
        assert_eq!(opts.max_bytes, 4_718_592, "4.5 MiB of base64 payload");
    }

    #[test]
    fn small_image_passes_through_untouched() {
        // Within every limit: byte-identical, no resize note, never refused.
        let bytes = encodable_png(10, 10);
        let out = resize_to_limits(&bytes, "image/png", &default_resize_options());
        assert!(!out.was_resized);
        assert_eq!(out.bytes, bytes);
        assert!(out.dimension_note().is_none());
    }

    #[test]
    fn undecodable_bytes_pass_through_instead_of_refusing() {
        // A corrupt "PNG" must read exactly as it does today — never an error.
        let bytes = b"not an image at all".to_vec();
        let out = resize_to_limits(&bytes, "image/png", &default_resize_options());
        assert!(!out.was_resized);
        assert_eq!(out.bytes, bytes);
    }

    #[test]
    fn formats_without_a_decoder_pass_through() {
        // GIF/WebP have no decoder in-tree (animated GIF would silently lose
        // frames); they ride at original size rather than being re-encoded.
        let bytes = b"GIF89a...".to_vec();
        let out = resize_to_limits(&bytes, "image/gif", &default_resize_options());
        assert!(!out.was_resized);
        assert_eq!(out.bytes, bytes);
    }
}
