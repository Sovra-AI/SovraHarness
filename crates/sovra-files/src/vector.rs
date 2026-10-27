//! Bounded SVG rasterization with external resources disabled.

use resvg::{tiny_skia, usvg};
use sovra_core::files::{FileError, MAX_DOCUMENT_BYTES};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};

static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();

pub(super) fn render(bytes: &[u8]) -> Result<Vec<u8>, FileError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(failed());
    }
    let fonts = FONTS.get_or_init(|| {
        let mut database = usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    });
    let image_count = Arc::new(AtomicUsize::new(0));
    let embedded_count = image_count.clone();
    let options = usvg::Options {
        fontdb: fonts.clone(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            // Normalize embedded rasters under the same allocation limits as uploads.
            // Nested SVG images are deliberately not recursively resolved.
            resolve_data: Box::new(move |_, bytes, _| {
                if embedded_count.fetch_add(1, Ordering::Relaxed) >= 4 {
                    return None;
                }
                crate::normalize_image(&bytes)
                    .ok()
                    .map(|(data, _)| usvg::ImageKind::PNG(Arc::new(data)))
            }),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|_| failed())?;
    if image_count.load(Ordering::Relaxed) > 4 {
        return Err(failed());
    }
    let scale = (2048.0 / tree.size().width().max(tree.size().height())).min(1.0);
    let size = usvg::Size::from_wh(tree.size().width() * scale, tree.size().height() * scale)
        .ok_or_else(failed)?
        .to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height()).ok_or_else(failed)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|_| failed())
}

fn failed() -> FileError {
    FileError("This drawing could not be rendered within the picture limits.".into())
}
