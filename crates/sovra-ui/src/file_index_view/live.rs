//! What a person sees while Sovra reads a folder — and the short summary afterwards.
//!
//! All wording and numbers come from `sovra_core::file_index::live::ScanPicture`; this
//! module only lays them out. Layout rules (`docs/styling.md`): sizes from tokens, chips
//! wrap, text truncates, and every count pairs an icon with words — never colour alone.
//! The headline is a polite status region so screen readers hear phase changes without
//! being interrupted by every count.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _, h_flex, progress::Progress, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::file_index::IndexedRoot;
use sovra_core::file_index::live::{ScanPicture, found_sentence};
use sovra_core::file_index::ontology::{FileKind, LifeArea, group_digits};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

/// How many kinds to show before "and more": enough to be interesting, few enough to scan.
const MAX_CHIPS: usize = 8;

/// The live panel for a folder that is being read right now.
pub(super) fn reading_panel(
    root: &IndexedRoot,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let picture = ScanPicture::of(&root.progress);
    let id = &root.id;
    let mut panel = v_flex()
        .id(SharedString::from(format!("index-live-{id}")))
        .test_support()
        .min_w_0()
        .gap(t.space.small)
        .child(
            div()
                .id(SharedString::from(format!("index-live-headline-{id}")))
                .test_support()
                .role(Role::Status)
                .aria_label(picture.headline)
                .sovra_text(TextRole::SectionTitle, cx)
                .child(picture.headline),
        )
        .child(
            Progress::new(SharedString::from(format!("index-live-bar-{id}")))
                .accessibility_label(format!("{} — {}", picture.headline, picture.count_line))
                // Indeterminate while the total is still being counted (honest, per HIG).
                .loading(picture.percent.is_none())
                .value(picture.percent.unwrap_or_default()),
        );
    let mut counts = h_flex()
        .flex_wrap()
        .min_w_0()
        .gap(t.space.small)
        .sovra_text(TextRole::Body, cx)
        .child(
            div()
                .id(SharedString::from(format!("index-live-count-{id}")))
                .test_support()
                .child(picture.count_line.clone()),
        );
    if let Some(time_left) = picture.time_left.clone() {
        counts = counts.child(
            div()
                .id(SharedString::from(format!("index-live-eta-{id}")))
                .test_support()
                .text_color(t.text_secondary)
                .child(time_left),
        );
    }
    panel = panel.child(counts);
    if let Some(name) = picture.now_reading.clone() {
        panel = panel.child(
            div()
                .id(SharedString::from(format!("index-live-now-{id}")))
                .test_support()
                .min_w_0()
                .text_ellipsis()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .aria_label(format!("Now reading {name}"))
                .child(format!("Now reading: {name}")),
        );
    }
    if !picture.found.is_empty() {
        panel = panel.child(section(
            &format!("index-live-found-{id}"),
            "What I've found so far",
            picture
                .found
                .iter()
                .map(|(kind, count)| Chip::kind(*kind, *count))
                .collect(),
            cx,
        ));
    }
    if !picture.areas.is_empty() {
        panel = panel.child(section(
            &format!("index-live-areas-{id}"),
            "Your files seem to be about",
            picture
                .areas
                .iter()
                .map(|(area, count)| Chip::area(*area, *count))
                .collect(),
            cx,
        ));
    }
    if let Some(problems) = picture.problems {
        let report_id = root.id.clone();
        panel = panel.child(
            h_flex()
                .flex_wrap()
                .min_w_0()
                .gap(t.space.small)
                .items_center()
                .child(Icon::new(IconName::TriangleAlert).small())
                // Text wraps; the button label stays short so it fits at 200% text.
                .child(
                    div()
                        .id(SharedString::from(format!("index-live-problem-text-{id}")))
                        .test_support()
                        .min_w_0()
                        .sovra_text(TextRole::Body, cx)
                        .child(problems),
                )
                .child(
                    design::button(
                        SharedString::from(format!("index-live-problems-{id}")),
                        "See why",
                        cx,
                    )
                    .on_click(cx.listener(move |shell, _, window, cx| {
                        shell.open_scan_report(&report_id, window, cx);
                    })),
                ),
        );
    }
    panel
}

/// After a scan: "Found 320 photos, 85 documents and 12 spreadsheets." plus chips.
pub(super) fn found_summary(
    root: &IndexedRoot,
    cx: &mut Context<Shell>,
) -> Option<impl IntoElement + use<>> {
    let sentence = found_sentence(&root.progress)?;
    let picture = ScanPicture::of(&root.progress);
    let t = design::tokens(cx);
    let id = &root.id;
    Some(
        v_flex()
            .min_w_0()
            .gap(t.space.compact)
            .child(
                div()
                    .id(SharedString::from(format!("index-found-sentence-{id}")))
                    .test_support()
                    .sovra_text(TextRole::Body, cx)
                    .child(sentence),
            )
            .child(chips(
                &format!("index-found-{id}"),
                picture
                    .found
                    .iter()
                    .map(|(kind, count)| Chip::kind(*kind, *count))
                    .collect(),
                cx,
            )),
    )
}

struct Chip {
    key: String,
    icon: IconName,
    label: &'static str,
    count: u64,
}

impl Chip {
    fn kind(kind: FileKind, count: u64) -> Self {
        Self {
            key: format!("{kind:?}").to_lowercase(),
            icon: kind_icon(kind),
            label: kind.label(),
            count,
        }
    }

    fn area(area: LifeArea, count: u64) -> Self {
        Self {
            key: format!("{area:?}").to_lowercase(),
            icon: area_icon(area),
            label: area.label(),
            count,
        }
    }
}

fn section(
    id: &str,
    title: &'static str,
    items: Vec<Chip>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .min_w_0()
        .gap(t.space.compact)
        .child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(title),
        )
        .child(chips(id, items, cx))
}

fn chips(id: &str, items: Vec<Chip>, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let hidden = items.len().saturating_sub(MAX_CHIPS);
    let mut row = h_flex().flex_wrap().min_w_0().gap(t.space.compact);
    for chip in items.into_iter().take(MAX_CHIPS) {
        let count = group_digits(chip.count);
        row = row.child(
            h_flex()
                .id(SharedString::from(format!("{id}-{}", chip.key)))
                .test_support()
                .aria_label(format!("{}: {count}", chip.label))
                .min_w_0()
                .gap(t.space.compact)
                .items_center()
                .px(t.space.small)
                .py(t.space.hair)
                .rounded(t.radius.pill)
                .sovra_surface(Surface::Elevated, cx)
                .sovra_text(TextRole::Caption, cx)
                .child(Icon::new(chip.icon).xsmall())
                .child(div().min_w_0().text_ellipsis().child(chip.label))
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(count),
                ),
        );
    }
    if hidden > 0 {
        row = row.child(
            div()
                .sovra_text(TextRole::Caption, cx)
                .text_color(t.text_secondary)
                .child(format!("and {hidden} more")),
        );
    }
    row
}

/// One icon per kind, data-driven like `artifacts_view::kind_icon`.
fn kind_icon(kind: FileKind) -> IconName {
    match kind {
        FileKind::Document => IconName::FileText,
        FileKind::Spreadsheet => IconName::Table,
        FileKind::Presentation => IconName::Presentation,
        FileKind::Photo => IconName::Image,
        FileKind::Video => IconName::Video,
        FileKind::Audio => IconName::Music,
        FileKind::Email => IconName::Mail,
        FileKind::Bill => IconName::Receipt,
        FileKind::Form => IconName::ClipboardCheck,
        FileKind::Book => IconName::BookOpen,
        FileKind::Code => IconName::Code,
        FileKind::Data => IconName::Database,
        FileKind::Archive => IconName::Archive,
        FileKind::App => IconName::AppWindow,
        FileKind::Other => IconName::File,
    }
}

fn area_icon(area: LifeArea) -> IconName {
    match area {
        LifeArea::Money => IconName::Wallet,
        LifeArea::Health => IconName::HeartPulse,
        LifeArea::Home => IconName::House,
        LifeArea::Work => IconName::Briefcase,
        LifeArea::Family => IconName::Users,
        LifeArea::Travel => IconName::Plane,
        LifeArea::Learning => IconName::GraduationCap,
        LifeArea::Hobbies => IconName::Palette,
        LifeArea::Legal => IconName::Scale,
        LifeArea::Shopping => IconName::ShoppingBag,
        LifeArea::Vehicles => IconName::Car,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{click_control, root};
    use crate::design;
    use crate::shell::Shell;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, px, size};
    use sovra_core::file_index::ontology::{FileKind, LifeArea};
    use sovra_core::file_index::{IndexCommand, IndexEvent, IndexStatus, IndexedRoot};
    use sovra_core::models::InterfaceScale;
    use sovra_core::{Command, NavTab};
    use sovra_runtime::Client;

    fn counting() -> IndexedRoot {
        let mut root = root();
        root.status = IndexStatus::Scanning;
        root.issues.clear();
        root.progress.discovered = 3_214;
        root
    }

    fn reading() -> IndexedRoot {
        let mut root = counting();
        let p = &mut root.progress;
        p.walk_complete = true;
        p.discovered = 5_000;
        p.handled = 1_204;
        p.files_per_minute = 1_000;
        p.failed = 12;
        p.current = Some(format!(
            "Photos/{}/IMG_0001.jpg",
            "very-long-album".repeat(20)
        ));
        for (kind, n) in [
            (FileKind::Photo, 900),
            (FileKind::Document, 200),
            (FileKind::Bill, 60),
            (FileKind::Spreadsheet, 20),
            (FileKind::Email, 9),
            (FileKind::Video, 5),
            (FileKind::Audio, 4),
            (FileKind::Book, 3),
            (FileKind::Code, 2),
            (FileKind::Other, 1),
        ] {
            p.kinds.insert(kind, n);
        }
        p.areas.insert(LifeArea::Money, 60);
        p.areas.insert(LifeArea::Travel, 400);
        root
    }

    #[gpui_kit::test]
    fn counting_then_reading_shows_found_kinds_and_a_way_to_see_problems(cx: &mut TestAppContext) {
        let (client, commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.expect("shell view");
        cx.update_window(window.into(), |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell
                    .file_index
                    .state
                    .apply(&IndexEvent::Roots(vec![counting()]));
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
            });
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.try_find("index-live-headline-chosen").is_some());
            assert!(window.try_find("index-live-bar-chosen").is_some());
            assert!(
                window.try_find("index-live-eta-chosen").is_none(),
                "no estimate while counting"
            );
            assert!(window.try_find("index-live-found-chosen-photo").is_none());

            shell.update(cx, |shell, _| {
                shell
                    .file_index
                    .state
                    .apply(&IndexEvent::Progress(reading()));
            });
            window.render_frame(cx);
            window.render_frame(cx);
            for id in [
                "index-live-eta-chosen",
                "index-live-now-chosen",
                "index-live-found-chosen-photo",
                "index-live-found-chosen-bill",
                "index-live-areas-chosen-travel",
            ] {
                assert!(window.try_find(id).is_some(), "{id}");
            }
            assert!(
                window.try_find("index-live-found-chosen-other").is_none(),
                "only the most common kinds are listed"
            );
            click_control(window, cx, "index-live-problems-chosen");
        })
        .expect("window");
        assert!(
            std::iter::from_fn(|| commands.try_recv().ok()).any(|command| matches!(command,
            Command::FileIndex(IndexCommand::Report { query, .. }) if query.root_id == "chosen"))
        );
    }

    #[gpui_kit::test]
    fn finished_folders_summarise_what_was_found(cx: &mut TestAppContext) {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let mut shell = None;
        let window = cx.open_window(size(px(1280.), px(1000.)), |window, cx| {
            let view = cx.new(|cx| Shell::new(0, window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        let shell = shell.expect("shell view");
        cx.update_window(window.into(), |_, window, cx| {
            let mut done = reading();
            done.status = IndexStatus::Ready;
            shell.update(cx, |shell, cx| {
                shell.file_index.state.apply(&IndexEvent::Roots(vec![done]));
                shell.select_tab(NavTab::Knowledge, cx);
                shell.select_knowledge_section(crate::knowledge_view::KnowledgeSection::Files, cx);
            });
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.try_find("index-live-chosen").is_none());
            assert!(window.try_find("index-capture-status-chosen").is_some());
            assert!(window.try_find("index-found-sentence-chosen").is_some());
            assert!(window.try_find("index-found-chosen-photo").is_some());
        })
        .expect("window");
    }

    #[gpui_kit::test]
    fn the_live_panel_fits_minimum_wide_and_double_text_sizes(cx: &mut TestAppContext) {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        for (width, height, scale) in [(640., 420., 100), (2560., 1440., 100), (640., 420., 200)] {
            let mut shell = None;
            let window = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Shell::new(0, window, cx));
                shell = Some(view.clone());
                Root::new(view, window, cx)
            });
            let shell = shell.expect("shell view");
            cx.update_window(window.into(), |_, window, cx| {
                design::set_interface_scale(InterfaceScale::from_percent(scale), cx);
                shell.update(cx, |shell, cx| {
                    shell
                        .file_index
                        .state
                        .apply(&IndexEvent::Roots(vec![reading()]));
                    shell.select_tab(NavTab::Knowledge, cx);
                    shell.select_knowledge_section(
                        crate::knowledge_view::KnowledgeSection::Files,
                        cx,
                    );
                });
                for mode in [ThemeMode::Dark, ThemeMode::Light] {
                    Theme::change(mode, Some(window), cx);
                    window.render_frame(cx);
                    window.render_frame(cx);
                    for id in [
                        "index-live-chosen",
                        "index-live-headline-chosen",
                        "index-live-bar-chosen",
                        "index-live-count-chosen",
                        "index-live-now-chosen",
                        "index-live-found-chosen-photo",
                        "index-live-found-chosen-spreadsheet",
                        "index-live-areas-chosen-money",
                        "index-live-problem-text-chosen",
                        "index-live-problems-chosen",
                    ] {
                        let bounds = window.find(id).bounds();
                        assert!(
                            bounds.left() >= px(0.) && bounds.right() <= px(width),
                            "{id} at {width}px/{scale}%: {bounds:?}"
                        );
                        assert!(bounds.size.height > px(0.), "{id}: {bounds:?}");
                    }
                }
            })
            .expect("window");
        }
    }
}
