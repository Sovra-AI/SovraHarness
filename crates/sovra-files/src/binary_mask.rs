//! Read only bounded, two-tone pixels. Preserve the full pixel grid at its native
//! size. Ordinary photos and recordings use the existing metadata path.
use image::{ImageDecoder as _, ImageReader};
use sovra_core::knowledge_analysis::mask::Mask;
use std::io::Cursor;

pub(super) fn decode(mime: &str, bytes: &[u8]) -> Option<Mask> {
    if !matches!(
        mime,
        "image/png"
            | "image/bmp"
            | "image/x-ms-bmp"
            | "image/tiff"
            | "image/gif"
            | "image/x-portable-bitmap"
            | "image/svg+xml"
    ) || bytes.len() > 4 * 1024 * 1024
    {
        return None;
    }
    let svg;
    let bytes = if mime == "image/svg+xml" {
        svg = crate::vector::render(bytes).ok()?;
        svg.as_slice()
    } else {
        bytes
    };
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(16 * 1024 * 1024);
    limits.max_image_height = Some(4096);
    limits.max_image_width = Some(4096);
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > 1_048_576 {
        return None;
    }
    let raster = image::DynamicImage::from_decoder(decoder)
        .ok()?
        .into_rgba16();
    let pixels = raster
        .pixels()
        .map(|p| match p.0 {
            [_, _, _, 0] | [65535, 65535, 65535, 65535] => Some(false),
            [0, 0, 0, 65535] => Some(true),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Mask::new(width as usize, height as usize, pixels).ok()
}
