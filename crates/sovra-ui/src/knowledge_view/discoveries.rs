//! What Sovra found in indexed files: a short card on the landing page, numbers
//! and categories at the top of the section, and the other findings one kind at a time.
use super::parts::{finding_words, heading, pill, pills, stat};
use super::*;
use sovra_core::{
    file_index::{automatic::DiscoveryOverview, ontology::group_digits},
    knowledge_analysis::FindingKind,
};

fn files(report: &DiscoveryOverview) -> u64 {
    report.kinds.iter().map(|v| v.1).sum()
}

fn count_of(report: &DiscoveryOverview, kind: FindingKind) -> u64 {
    report
        .counts
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(0, |v| v.1)
}

fn summary(report: &DiscoveryOverview) -> String {
    let files = files(report);
    let findings = report.counts.iter().map(|v| v.1).sum::<u64>();
    format!(
        "{} {} sorted · {} {}",
        group_digits(files),
        if files == 1 { "file" } else { "files" },
        group_digits(findings),
        if findings == 1 { "finding" } else { "findings" }
    )
}

/// The most common file kinds first. A large folder can hold a dozen kinds;
/// past `limit` the rest are counted in one "N more" pill.
fn kind_pills(report: &DiscoveryOverview, limit: usize) -> Vec<(String, Option<u64>)> {
    let mut kinds = report.kinds.clone();
    kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
    let rest = kinds.len().saturating_sub(limit);
    let mut pills: Vec<_> = kinds
        .iter()
        .take(limit)
        .map(|(kind, n)| (kind.count_phrase(*n), None))
        .collect();
    if rest > 0 {
        pills.push((
            format!("{rest} more {}", if rest == 1 { "kind" } else { "kinds" }),
            None,
        ));
    }
    pills
}

/// The card under search on the landing page.
pub(super) fn overview(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let state = &shell.file_index.state;
    let report = state.discoveries.as_ref()?;
    let t = design::tokens(cx);
    let mut panel = v_flex()
        .id("knowledge-auto-overview")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .p(t.space.medium)
        .sovra_surface(Surface::Subtle, cx);
    let mut top = h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Found in your files"),
        );
    let status = match report {
        Err(error) => format!("Sovra will try reading again. {error}"),
        Ok(report) if report.working => format!("{} · Still reading…", summary(report)),
        Ok(report) if report.kinds.is_empty() => {
            "Connect a folder and Sovra sorts its files and notes how things in them connect."
                .into()
        }
        Ok(report) => summary(report),
    };
    if let Ok(report) = report
        && !report.kinds.is_empty()
    {
        top = top.child(
            design::button("knowledge-auto-open", "See relationships", cx)
                .small()
                .icon(IconName::ChevronRight)
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.select_knowledge_section(KnowledgeSection::Relationships, cx);
                })),
        );
    }
    panel = panel.child(top).child(
        div()
            .id("knowledge-auto-status")
            .test_support()
            .aria_label(status.clone())
            .sovra_text(TextRole::Body, cx)
            .child(status),
    );
    if let Ok(report) = report
        && !report.kinds.is_empty()
    {
        panel = panel.child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.compact)
                .children(
                    kind_pills(report, 5)
                        .into_iter()
                        .map(|(text, n)| pill(text, n, cx)),
                ),
        );
    }
    Some(panel)
}

/// Numbers and categories at the top of the section.
pub(super) fn details(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let report = shell.file_index.state.discoveries.as_ref()?.as_ref().ok()?;
    if report.kinds.is_empty() {
        return None;
    }
    let t = design::tokens(cx);
    let connections = count_of(report, FindingKind::Relation);
    let other = report
        .counts
        .iter()
        .filter(|(k, _)| *k != FindingKind::Relation)
        .map(|v| v.1)
        .sum::<u64>();
    let mut panel = v_flex()
        .id("knowledge-auto-details")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .child(
            div()
                .sovra_text(TextRole::Body, cx)
                .text_color(t.text_secondary)
                .child(if report.working {
                    "Sovra is still reading your folders. Everything here links back to the line it came from."
                } else {
                    "Sovra read your folders on this computer, sorted the files, and noted how things in them connect. Everything here links back to the line it came from."
                }),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .gap(t.space.small)
                .child(stat(IconName::FileText, files(report), "files sorted", cx))
                .child(stat(IconName::Workflow, connections, "connections", cx))
                .child(stat(
                    IconName::Bookmark,
                    report.subjects.len() as u64,
                    "kinds of things",
                    cx,
                ))
                .child(stat(IconName::Sparkles, other, "other findings", cx)),
        )
        .child(pills(
            "knowledge-auto-kinds",
            "Your files are",
            kind_pills(report, usize::MAX),
            cx,
        ));
    if !report.areas.is_empty() {
        panel = panel.child(pills(
            "knowledge-auto-areas",
            "Parts of life they cover",
            report
                .areas
                .iter()
                .map(|(a, n)| (a.label().to_owned(), Some(*n)))
                .collect(),
            cx,
        ));
    }
    if !report.subjects.is_empty() {
        panel = panel.child(pills(
            "knowledge-auto-subjects",
            "Things they mention",
            report
                .subjects
                .iter()
                .map(|(k, n)| (k.display_name(), Some(*n)))
                .collect(),
            cx,
        ));
    }
    Some(panel)
}

/// Everything else Sovra noticed, one kind at a time so the page stays short.
pub(super) fn findings(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let report = shell.file_index.state.discoveries.as_ref()?.as_ref().ok()?;
    if report.counts.is_empty() {
        return None;
    }
    let t = design::tokens(cx);
    let browser = &shell.file_index.state.finding_browser;
    let mut chips = h_flex().w_full().min_w_0().flex_wrap().gap(t.space.compact);
    for (group, (kind, count)) in report.counts.iter().enumerate() {
        let kind = *kind;
        let (name, icon, _) = finding_words(kind);
        let open = browser.kind == Some(kind);
        chips = chips.child(
            design::button(
                ("knowledge-auto-kind", group),
                format!("{name} · {}", group_digits(*count)),
                cx,
            )
            .small()
            .icon(icon)
            .selected(open)
            .max_w_full()
            .on_click(cx.listener(move |shell, _, _, cx| {
                if open {
                    shell.file_index.state.finding_browser.close();
                    shell.file_index.discovery_source = None;
                    cx.notify();
                } else {
                    let command = shell.file_index.state.finding_browser.open(kind, None);
                    shell.index_command(command, cx);
                }
            })),
        );
    }
    let mut panel = v_flex()
        .id("knowledge-auto-findings")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .child(heading(
            "More that Sovra noticed",
            "Pick a kind to see each finding and the file it came from.",
            cx,
        ))
        .child(chips);
    let Some(kind) = browser.kind else {
        return Some(panel);
    };
    panel = panel.child(
        div()
            .sovra_text(TextRole::Caption, cx)
            .text_color(t.text_secondary)
            .child(finding_words(kind).2),
    );
    let rows = match &browser.page {
        Some(Ok(page)) => page.items.iter().collect::<Vec<_>>(),
        Some(Err(error)) => {
            return Some(
                panel.child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child(error.to_string()),
                ),
            );
        }
        None => {
            return Some(
                panel.child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child("Reading findings…"),
                ),
            );
        }
    };
    let mut list = v_flex()
        .w_full()
        .min_w_0()
        .rounded(t.radius.card)
        .border_1()
        .border_color(t.material_border);
    for (index, item) in rows.into_iter().enumerate() {
        let reference = item.reference.clone();
        let source = &shell.file_index.state.browser;
        let open = shell.file_index.discovery_source.as_ref() == Some(&reference)
            && source.preview_reference.as_ref() == Some(&reference);
        let mut row = v_flex()
            .w_full()
            .min_w_0()
            .gap(t.space.compact)
            .px(t.space.medium)
            .py(t.space.small);
        if index > 0 {
            row = row.border_t_1().border_color(t.material_border);
        }
        row = row.child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(t.space.small)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(t.space.hair)
                        .child(
                            div()
                                .min_w_0()
                                .sovra_text(TextRole::Body, cx)
                                .child(item.detail.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .sovra_text(TextRole::Caption, cx)
                                .text_color(t.text_secondary)
                                .child(format!("{} · line {}", item.path, item.line)),
                        ),
                )
                .child(
                    design::button(
                        ("knowledge-auto-source", index),
                        if open { "Hide source" } else { "Source" },
                        cx,
                    )
                    .ghost()
                    .small()
                    .flex_shrink_0()
                    .icon(IconName::FileText)
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        if open {
                            shell.file_index.discovery_source = None;
                        } else {
                            shell.file_index.discovery_source = Some(reference.clone());
                            let command = shell.file_index.state.browser.preview(reference.clone());
                            shell.index_command(command, cx);
                        }
                        cx.notify();
                    })),
                ),
        );
        if open {
            let text = source
                .preview
                .as_ref()
                .map(|p| sovra_core::file_index::passage_wording(&p.text))
                .or(source.preview_error.as_deref())
                .unwrap_or("Reading saved passage…");
            row = row.child(
                div()
                    .id(("knowledge-auto-wording", index))
                    .test_support()
                    .aria_label(text.to_owned())
                    .w_full()
                    .min_w_0()
                    .p(t.space.small)
                    .sovra_surface(Surface::Subtle, cx)
                    .sovra_text(TextRole::Reading, cx)
                    .child(text.to_owned()),
            );
        }
        list = list.child(row);
    }
    panel = panel.child(list);
    if let Some(Ok(page)) = &browser.page
        && let Some(before) = page.next
    {
        panel = panel.child(
            design::button("knowledge-auto-next", "Show more", cx)
                .small()
                .self_start()
                .on_click(cx.listener(move |shell, _, _, cx| {
                    let command = shell
                        .file_index
                        .state
                        .finding_browser
                        .open(kind, Some(before));
                    shell.index_command(command, cx);
                })),
        );
    }
    Some(panel)
}
