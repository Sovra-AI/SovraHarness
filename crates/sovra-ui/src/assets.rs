//! Compile-time assets; installed builds never depend on the working directory.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

pub(crate) const SOVRA_LOGO: &str = "branding/sovra-logo.svg";
const LOGO_BYTES: &[u8] = include_bytes!("../assets/branding/sovra-logo.svg");

#[derive(Debug)]
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == SOVRA_LOGO {
            return Ok(Some(Cow::Borrowed(LOGO_BYTES)));
        }
        // AllAssets: the views use Lucide icons beyond the default subset.
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::AllAssets.list(path)?;
        if SOVRA_LOGO.starts_with(path) {
            paths.push(SOVRA_LOGO.into());
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn bundled_logo_renders_in_colour_and_keeps_component_icons(cx: &mut gpui_kit::TestAppContext) {
        let bytes = Assets.load(SOVRA_LOGO).unwrap().unwrap();
        cx.update(|cx| {
            let image = cx.svg_renderer().render_single_frame(&bytes, 0.05).unwrap();
            let pixels = image.as_bytes(0).unwrap();
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[3] > 0 && p[0] != p[1])
            );
        });
        assert!(Assets.load("icons/check.svg").unwrap().is_some());
        assert!(Assets.load("branding/missing.svg").is_err());
        assert_eq!(
            Assets.list("branding/").unwrap(),
            vec![SharedString::from(SOVRA_LOGO)]
        );
    }
}
