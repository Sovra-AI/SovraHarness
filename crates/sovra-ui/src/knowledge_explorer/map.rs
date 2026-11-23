//! Browse the connections in one Explore response and follow an edge to its source line.
use super::*;
use gpui_kit::component::ActiveTheme as _;
use sovra_core::knowledge_analysis::SourceLine;

#[derive(Default)]
pub(super) struct EvidenceMap {
    pub(super) list: bool,
    connection: Option<usize>,
    pub(super) selected: Option<SourceLine>,
}

impl EvidenceMap {
    pub(super) fn for_report(report: &Exploration) -> Self {
        Self {
            connection: (!report.shared.is_empty()).then_some(0),
            ..Self::default()
        }
    }
}

fn origin(citation: &Citation) -> &'static str {
    match citation {
        Citation::File { .. } => "File",
        Citation::User { .. } => "Personal record",
        Citation::Artifact { .. } => "Artifact",
    }
}

fn source_stamp(citation: &Citation) -> String {
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    let (label, seconds) = match citation {
        Citation::File {
            reference,
            observed,
        } => (
            format!("File version {} · indexed", reference.version_id),
            i64::try_from(*observed).ok(),
        ),
        Citation::User { recorded, .. } => {
            ("Recorded".into(), Some(recorded.millis().div_euclid(1000)))
        }
        Citation::Artifact { updated, .. } => (
            "Current version saved".into(),
            Some(updated.millis().div_euclid(1000)),
        ),
    };
    let date = seconds
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
        .and_then(|d| d.format(&Rfc3339).ok())
        .unwrap_or_else(|| "date unavailable".into());
    format!("{label} · {date}")
}

pub(super) fn render(
    map: &EvidenceMap,
    report: &Exploration,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let mut modes = h_flex().flex_wrap().gap(t.space.compact);
    for (id, label, list) in [
        ("explore-map", "Evidence map", false),
        ("explore-list", "Source list", true),
    ] {
        modes = modes.child(
            design::button(id, label, cx)
                .selected(map.list == list)
                .on_click(cx.listener(move |shell, _, _, cx| {
                    shell.explorer.map.list = list;
                    cx.notify();
                })),
        );
    }
    let count = report.readings.len();
    let links = report.shared.len();
    let coverage = format!(
        "This search returned {count} {} and {links} shared {}.",
        if count == 1 { "excerpt" } else { "excerpts" },
        if links == 1 {
            "connection"
        } else {
            "connections"
        }
    );
    let mut panel = v_flex()
        .min_w_0()
        .w_full()
        .gap(t.space.small)
        .child(modes)
        .child(
            div()
                .id("explore-map-count")
                .test_support()
                .aria_label(coverage.clone())
                .sovra_text(TextRole::Caption, cx)
                .child(coverage),
        );
    if map.list {
        return panel;
    }

    let choose = |shell: &mut Shell, index, cx: &mut Context<Shell>| {
        shell.explorer.map.connection = index;
        shell.explorer.map.selected = None;
        shell.explorer.opened = None;
        cx.notify();
    };
    panel = panel.child(
        h_flex().flex_wrap().child(
            design::button("explore-map-all", "All matching sources", cx)
                .selected(map.connection.is_none())
                .on_click(cx.listener(move |shell, _, _, cx| choose(shell, None, cx))),
        ),
    );
    if !report.shared.is_empty() {
        let mut connections = v_flex()
            .min_w_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .pl(t.space.medium)
            .gap(t.space.compact);
        for (index, connection) in report.shared.iter().enumerate() {
            connections = connections.child(
                h_flex().min_w_0().flex_wrap().child(
                    design::button(("explore-connection", index), connection.label.clone(), cx)
                        .selected(map.connection == Some(index))
                        .on_click(
                            cx.listener(move |shell, _, _, cx| choose(shell, Some(index), cx)),
                        ),
                ),
            );
        }
        panel = panel.child(connections);
    }
    let connection = map.connection.and_then(|i| report.shared.get(i));
    let mut branch = v_flex()
        .min_w_0()
        .gap(t.space.compact)
        .pl(t.space.medium)
        .border_l_1()
        .border_color(cx.theme().border);
    let lines = connection.map_or_else(
        || {
            (0..count)
                .map(|reading| SourceLine { reading, line: 1 })
                .collect::<Vec<_>>()
        },
        |link| link.sources.clone(),
    );
    let heading = connection.map_or_else(
        || "Sources matching your search".into(),
        |link| format!("{} · source lines", link.label),
    );
    branch = branch.child(div().sovra_text(TextRole::SectionTitle, cx).child(heading));
    for (node, line) in lines.iter().copied().enumerate() {
        let excerpt = &report.readings[line.reading].excerpt;
        let absolute = excerpt
            .first_line
            .saturating_add(line.line.saturating_sub(1));
        let label = format!(
            "{} · {} · line {absolute}",
            origin(&excerpt.citation),
            excerpt.title
        );
        branch = branch.child(
            h_flex().min_w_0().flex_wrap().child(
                design::button(("explore-map-source", node), label, cx)
                    .selected(map.selected == Some(line))
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        shell.explorer.map.selected = Some(line);
                        shell.explorer.opened = None;
                        cx.notify();
                    })),
            ),
        );
    }
    panel = panel.child(branch);
    if let Some(line) = map.selected {
        let excerpt = &report.readings[line.reading].excerpt;
        let quote = excerpt
            .wording
            .lines()
            .nth(line.line.saturating_sub(1) as usize)
            .unwrap_or("");
        panel = panel.child(
            v_flex()
                .min_w_0()
                .sovra_surface(Surface::Matte, cx)
                .p(t.space.medium)
                .gap(t.space.compact)
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(source_stamp(&excerpt.citation)),
                )
                .child(
                    div()
                        .id("explore-line-wording")
                        .test_support()
                        .aria_label(quote.to_owned())
                        .sovra_text(TextRole::Body, cx)
                        .child(quote.to_owned()),
                ),
        );
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::Root;
    use gpui_kit::{TestAppContext, px, size, test::TestWindowExt as _};
    use sovra_core::{NavTab, Timestamp, knowledge_analysis::Excerpt};
    use sovra_runtime::Client;

    #[gpui_kit::test]
    fn map_follows_exact_lines_and_all_sources_includes_an_unlinked_excerpt(
        cx: &mut TestAppContext,
    ) {
        let (client, _commands, _events) = Client::loopback();
        cx.update(|cx| crate::init(client, cx));
        let sources = [
            ("Note", "Juniper\nJuniper weighs 0.5 kg."),
            ("Table", "value,unit\n500,g"),
            ("Other", "Juniper is green."),
        ]
        .into_iter()
        .map(|(id, text)| Excerpt::artifact(id.into(), Timestamp(1000), id.into(), text, 0))
        .collect();
        let report =
            Exploration::from_excerpts(sources, &AnalysisOptions::default(), vec![]).unwrap();
        let window = cx.open_window(size(px(1400.), px(1000.)), |window, cx| {
            let shell = cx.new(|cx| {
                let mut shell = Shell::new(0, window, cx);
                shell.select_tab(NavTab::Knowledge, cx);
                shell.explorer.pending_request = Some(0);
                shell.explorer.observe(&Event::Explored {
                    request_id: 0,
                    result: Ok(report),
                });
                shell
            });
            Root::new(shell, window, cx)
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("explore-query").visible());
            assert_eq!(
                window.find("explore-map-count").label(),
                Some("This search returned 3 excerpts and 1 shared connection.")
            );
            for (i, quote) in ["Juniper weighs 0.5 kg.", "500,g"].into_iter().enumerate() {
                window.click(("explore-map-source", i), cx);
                window.render_frame(cx);
                assert_eq!(window.find("explore-line-wording").label(), Some(quote));
            }
            window.click("explore-map-all", cx);
            window.render_frame(cx);
            assert!(window.try_find("explore-line-wording").is_none());
            window.click(("explore-map-source", 2usize), cx);
            window.render_frame(cx);
            assert_eq!(
                window.find("explore-line-wording").label(),
                Some("Juniper is green.")
            );
            window.click("explore-list", cx);
            window.render_frame(cx);
            for i in 0..3usize {
                assert!(window.try_find(("explore-source", i)).is_some());
            }
            window.click("knowledge-overview", cx);
            window.render_frame(cx);
            assert!(window.try_find("explore-map-count").is_none());
            window.click("explore-show-results", cx);
            window.render_frame(cx);
            assert!(window.try_find(("explore-source", 2usize)).is_some());
        })
        .unwrap();
    }
}
