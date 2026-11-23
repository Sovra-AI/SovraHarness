//! Actual discoveries on the landing page; the vocabulary is an optional reference.
use super::*;
use sovra_core::{
    file_index::{automatic::DiscoveryOverview, ontology::group_digits},
    knowledge_analysis::FindingKind,
};

fn summary(report: &DiscoveryOverview) -> String {
    let files = report.kinds.iter().map(|v| v.1).sum::<u64>();
    let findings = report.counts.iter().map(|v| v.1).sum::<u64>();
    format!(
        "{} {} classified · {} {}",
        group_digits(files),
        if files == 1 { "file" } else { "files" },
        group_digits(findings),
        if findings == 1 { "finding" } else { "findings" }
    )
}

fn labels(report: &DiscoveryOverview) -> String {
    report
        .kinds
        .iter()
        .map(|(kind, n)| kind.count_phrase(*n))
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(super) fn overview(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let state = &shell.file_index.state;
    let report = state.discoveries.as_ref()?;
    let t = design::tokens(cx);
    let mut panel = v_flex()
        .id("knowledge-auto-overview")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.compact)
        .p(t.space.medium)
        .sovra_surface(Surface::Subtle, cx)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Found in your files"),
        );
    match report {
        Err(error) => {
            panel = panel.child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child(format!("Automatic analysis will retry. {error}")),
            );
        }
        Ok(report) => {
            let status = if report.working {
                format!("{} · Analyzing saved files…", summary(report))
            } else if report.kinds.is_empty() {
                "Sovra classifies files and finds connections as folders are indexed.".into()
            } else {
                summary(report)
            };
            panel = panel.child(
                div()
                    .id("knowledge-auto-status")
                    .test_support()
                    .aria_label(status.clone())
                    .sovra_text(TextRole::Body, cx)
                    .child(status),
            );
            if !report.kinds.is_empty() {
                panel = panel.child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(labels(report)),
                );
            }
            if let Some(example) = report
                .examples
                .iter()
                .find(|e| e.kind == FindingKind::Relation)
                .or(report.examples.first())
            {
                panel = panel
                    .child(
                        div()
                            .sovra_text(TextRole::Body, cx)
                            .child(example.detail.clone()),
                    )
                    .child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child(format!("{} · line {}", example.path, example.line)),
                    );
            }
        }
    }
    Some(panel)
}

pub(super) fn details(shell: &Shell, cx: &mut Context<Shell>) -> Option<impl IntoElement + use<>> {
    let report = shell.file_index.state.discoveries.as_ref()?.as_ref().ok()?;
    if report.kinds.is_empty() {
        return None;
    }
    let t = design::tokens(cx);
    let mut panel = v_flex()
        .id("knowledge-auto-details")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Automatic classification"),
        )
        .child(div().sovra_text(TextRole::Body, cx).child(labels(report)));
    if !report.areas.is_empty() {
        panel = panel.child(div().sovra_text(TextRole::Body, cx).child(format!(
                "Areas of life · {}",
                report
                    .areas
                    .iter()
                    .map(|(a, n)| format!("{} ({n})", a.label()))
                    .collect::<Vec<_>>()
                    .join(" · ")
            )));
    }
    for (title, groups) in [
        (
            "Subjects found",
            report
                .subjects
                .iter()
                .map(|(k, n)| format!("{} ({n})", k.label()))
                .collect::<Vec<_>>(),
        ),
        (
            "Relationships found",
            report
                .predicates
                .iter()
                .map(|(k, n)| format!("{} ({n})", k.label()))
                .collect(),
        ),
    ] {
        if !groups.is_empty() {
            panel = panel.child(
                div()
                    .sovra_text(TextRole::Body, cx)
                    .child(format!("{title} · {}", groups.join(" · "))),
            );
        }
    }
    for (group, (kind, count)) in report.counts.iter().enumerate() {
        let kind = *kind;
        panel = panel.child(
            div()
                .sovra_text(TextRole::SectionTitle, cx)
                .child(format!("{} ({count})", kind.label())),
        );
        let browser = &shell.file_index.state.finding_browser;
        let expanded = browser.kind == Some(kind);
        let samples = report
            .examples
            .iter()
            .filter(|item| item.kind == kind)
            .collect::<Vec<_>>();
        let rows = if expanded {
            panel = panel.child(
                design::button(("knowledge-auto-collapse", group), "Back to examples", cx)
                    .small()
                    .self_start()
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.file_index.state.finding_browser.close();
                        cx.notify();
                    })),
            );
            match &browser.page {
                Some(Ok(page)) => page.items.iter().collect(),
                Some(Err(error)) => {
                    panel = panel.child(
                        div()
                            .sovra_text(TextRole::Body, cx)
                            .child(error.to_string()),
                    );
                    Vec::new()
                }
                None => {
                    panel = panel.child(
                        div()
                            .sovra_text(TextRole::Caption, cx)
                            .child("Reading findings…"),
                    );
                    Vec::new()
                }
            }
        } else {
            if *count > samples.len() as u64 {
                panel = panel.child(
                    design::button(
                        ("knowledge-auto-all", group),
                        format!("Browse all {count} findings"),
                        cx,
                    )
                    .small()
                    .self_start()
                    .on_click(cx.listener(move |shell, _, _, cx| {
                        let command = shell.file_index.state.finding_browser.open(kind, None);
                        shell.index_command(command, cx);
                    })),
                );
            }
            samples
        };
        for (i, item) in rows.into_iter().enumerate() {
            let index = group * 100 + i;
            let reference = item.reference.clone();
            let browser = &shell.file_index.state.browser;
            let open = shell.file_index.discovery_source.as_ref() == Some(&reference)
                && browser.preview_reference.as_ref() == Some(&reference);
            panel = panel
                .child(
                    div()
                        .sovra_text(TextRole::Body, cx)
                        .child(item.detail.clone()),
                )
                .child(
                    div()
                        .sovra_text(TextRole::Caption, cx)
                        .child(format!("{} · line {}", item.path, item.line)),
                )
                .child(
                    design::button(
                        ("knowledge-auto-source", index),
                        if open { "Close source" } else { "Read source" },
                        cx,
                    )
                    .small()
                    .self_start()
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
                );
            if open {
                let text = browser
                    .preview
                    .as_ref()
                    .map(|p| sovra_core::file_index::passage_wording(&p.text))
                    .or(browser.preview_error.as_deref())
                    .unwrap_or("Reading saved passage…");
                panel = panel.child(
                    div()
                        .id(("knowledge-auto-wording", index))
                        .test_support()
                        .aria_label(text.to_owned())
                        .sovra_text(TextRole::Reading, cx)
                        .child(text.to_owned()),
                );
            }
        }
        if expanded
            && let Some(Ok(page)) = &browser.page
            && let Some(before) = page.next
        {
            panel = panel.child(
                design::button(("knowledge-auto-next", group), "Next findings", cx)
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
    }
    Some(panel)
}
