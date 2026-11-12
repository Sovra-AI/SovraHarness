//! Bounded UI paragraphs: break RTL text before shaping each row.
//!
//! GPUI 0.3.9 wraps an already shaped line, which can paint RTL rows outside
//! their bounds. Keep the native shaper, but use logical-order line breaks.

use gpui_bidi::paragraph::{RtlLayout, RtlText};
use gpui_kit::{
    Div, InteractiveElement as _, IntoElement as _, ParentElement as _, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, div,
};

/// Plain, non-editable paragraph content. The parent supplies typography and
/// color; this element owns wrapping and exposes the original accessible text.
pub fn paragraph(text: impl Into<SharedString>) -> Stateful<Div> {
    paragraph_with_layout(text.into()).0
}

fn paragraph_with_layout(text: SharedString) -> (Stateful<Div>, Option<RtlLayout>) {
    let bidi = unicode_bidi::BidiInfo::new(&text, None);
    let block = div().id("paragraph").w_full().min_w_0();
    if !bidi.has_rtl() {
        return (block.child(text), None);
    }
    let rtl = bidi
        .paragraphs
        .first()
        .is_some_and(|paragraph| paragraph.level.is_rtl());
    let content = RtlText::new(text.clone()).with_base_rtl(rtl);
    let layout = content.layout().clone();
    (
        block
            .role(Role::Label)
            .aria_label(text)
            .child(content.into_any_element()),
        Some(layout),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{self, SovraStyled as _, TextRole};
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, IntoElement, Pixels, Render, TestAppContext, TestSupportExt as _,
        Window, px, size,
    };
    use sovra_core::models::InterfaceScale;
    use sovra_runtime::Client;

    const SAMPLE: &str = "اسمك يكفي للبدء. أضف بريداً وكلمة مرور فقط إذا أردت تسجيل الدخول بهما لاحقاً. تبقى كلمة المرور على هذا الحاسوب.";

    struct Sample {
        layout: Option<RtlLayout>,
    }

    impl Render for Sample {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (paragraph, layout) = paragraph_with_layout(SAMPLE.into());
            self.layout = layout;
            div().w_full().p(design::tokens(cx).space.large).child(
                paragraph
                    .id("rtl-paragraph")
                    .test_support()
                    .max_w(design::tokens(cx).layout.dialog_width)
                    .sovra_text(TextRole::Body, cx),
            )
        }
    }

    #[gpui_kit::test]
    fn arabic_rows_stay_inside_the_paragraph_and_in_reading_order(cx: &mut TestAppContext) {
        let (client, _, _) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                });
                let mut sample = None;
                let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                    let view = cx.new(|_| Sample { layout: None });
                    sample = Some(view.clone());
                    Root::new(view, window, cx)
                });
                let sample = sample.expect("sample created");
                cx.update_window(window.into(), |_, window, cx| {
                    window.render_frame(cx);
                    let element = window.find("rtl-paragraph");
                    assert_eq!(element.label(), Some(SAMPLE));
                    let bounds = element.bounds();
                    let layout = sample.read(cx).layout.as_ref().expect("RTL layout");
                    // The wrapper deliberately hangs one trailing space beyond
                    // a row. Include its measured advance in the layout envelope;
                    // native screenshots additionally verify the painted glyphs.
                    let space = SAMPLE.find(' ').expect("sample has spaces");
                    let hanging = layout
                        .rects_for_range(space..space + 1)
                        .into_iter()
                        .map(|rect| rect.size.width)
                        .fold(px(0.), Pixels::max);
                    let mut offset = 0;
                    for word in SAMPLE.split_whitespace() {
                        let start = offset + SAMPLE[offset..].find(word).expect("next word");
                        offset = start + word.len();
                        for rect in layout.rects_for_range(start..offset) {
                            assert!(
                                rect.left() >= bounds.left() - hanging - px(1.)
                                    && rect.right() <= bounds.right() + px(1.),
                                "{word}: {rect:?} outside {bounds:?}"
                            );
                        }
                    }
                    let first = layout.position_for_index(0).expect("first word");
                    let last = layout
                        .position_for_index(SAMPLE.len() - 1)
                        .expect("last word");
                    assert!(first.y < last.y, "paragraph must read top to bottom");
                })
                .expect("window exists");
            }
        }
    }
}
