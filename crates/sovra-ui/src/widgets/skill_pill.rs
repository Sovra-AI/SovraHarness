//! Presentation of the explicit shortcut at the start of a chat message.
use crate::design::{self, SovraStyled as _, TextRole};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex};
use gpui_kit::{
    App, Div, ElementId, InteractiveElement as _, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::skills::{Skill, SkillCatalog};
use std::ops::Range;

/// Recognise a known shortcut even before its required arguments are filled in.
pub(crate) fn leading_skill<'a>(
    catalog: &'a SkillCatalog,
    text: &str,
) -> Option<(&'a Skill, Range<usize>)> {
    let trimmed = text.trim_start();
    let tail = trimmed.strip_prefix('/')?;
    let name = tail
        .split_once(char::is_whitespace)
        .map_or(tail, |(name, _)| name);
    let start = text.len() - trimmed.len();
    Some((catalog.find(name)?, start..start + 1 + name.len()))
}

pub(crate) fn pill(
    id: impl Into<ElementId>,
    title: &str,
    icon: IconName,
    cx: &App,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let t = design::tokens(cx);
    h_flex()
        .id(id)
        .test_support()
        .aria_label(format!("Skill: {title}"))
        .min_w_0()
        .max_w_full()
        .items_center()
        .gap(t.space.compact)
        .px(t.space.small)
        .rounded(t.radius.pill)
        .bg(t.nav_active_background)
        .sovra_text(TextRole::Caption, cx)
        .child(Icon::new(icon).size(t.layout.icon_small).flex_shrink_0())
        .child(div().min_w_0().text_ellipsis().child(title.to_owned()))
}
