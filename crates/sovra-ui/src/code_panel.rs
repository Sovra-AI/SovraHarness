//! Source-only writing panel fed by AG-UI snapshots and the saved artifact library.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
};
use sovra_core::artifacts::{ArtifactCommand, ArtifactKind};
use sovra_core::{AgUiWidget, Command, WorkspaceState};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::Shell;

pub(crate) fn render(
    state: &WorkspaceState,
    cx: &mut Context<Shell>,
) -> Option<gpui_kit::AnyElement> {
    let (language, content, artifact_id) = state
        .messages
        .iter()
        .rev()
        .flat_map(|message| message.widgets.iter().rev())
        .find_map(|widget| match widget {
            AgUiWidget::CodeWriting {
                language,
                content,
                artifact_id,
            } => Some((language, content, artifact_id)),
            _ => None,
        })?;
    let t = design::tokens(cx);
    let saved = artifact_id
        .as_deref()
        .and_then(|id| state.artifacts.get(id));
    let status = if state.is_generating {
        "Writing code…"
    } else if saved.is_some() {
        "Saved in Artifacts"
    } else {
        "Source preview · not saved yet"
    };
    let mut actions = h_flex().flex_shrink_0().flex_wrap().gap(t.space.small);
    if let Some(artifact) = saved {
        let id = artifact.id.clone();
        actions = actions.child(
            design::button("code-open-saved", "Open saved work", cx)
                .on_click(cx.listener(move |shell, _, _, cx| shell.open_artifact(&id, cx))),
        );
        if artifact.kind.info().opens_in_browser {
            let id = artifact.id.clone();
            actions = actions.child(
                design::primary_button("code-open-browser", "Open in browser", cx).on_click(
                    move |_, _, cx| {
                        let _ = cx
                            .global::<RuntimeClient>()
                            .0
                            .send(Command::Artifacts(ArtifactCommand::Open { id: id.clone() }));
                    },
                ),
            );
        }
        if artifact.kind == ArtifactKind::Webpage {
            let id = artifact.id.clone();
            actions = actions.child(
                design::button("code-run-website", "Run website", cx).on_click(move |_, _, cx| {
                    let _ = cx.global::<RuntimeClient>().0.send(Command::Artifacts(
                        ArtifactCommand::Deploy { id: id.clone() },
                    ));
                }),
            );
            if let Some(url) = state.artifacts.deployments.get(&artifact.id) {
                let url = url.clone();
                let id = artifact.id.clone();
                actions = actions
                    .child(
                        design::button("code-view-website", "View website", cx)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(
                        design::button("code-stop-website", "Stop website", cx).on_click(
                            move |_, _, cx| {
                                let _ = cx.global::<RuntimeClient>().0.send(Command::Artifacts(
                                    ArtifactCommand::StopDeployment { id: id.clone() },
                                ));
                            },
                        ),
                    );
            }
        }
    }
    Some(v_flex().id("code-writing-panel").test_support()
        .w_full().h_full().min_w_0().min_h_0().overflow_y_scroll()
        .sovra_surface(Surface::Chrome, cx).p(t.space.medium).gap(t.space.small)
        .child(h_flex().flex_shrink_0().flex_wrap().gap(t.space.small)
            .child(div().flex_1().min_w_0().sovra_text(TextRole::SectionTitle, cx).child("Your code"))
            .child(design::button("code-panel-close", "Back to chat", cx)
                .on_click(cx.listener(|shell, _, _, cx| shell.show_code_panel(false, cx)))))
        .child(div().flex_shrink_0().sovra_text(TextRole::Caption, cx).child(format!("{status} · {language}")))
        .child(actions)
        .child(div().id("code-writing-source").test_support().flex_1().min_h(t.layout.control_height).min_w_0()
            .overflow_y_scroll().overflow_x_scroll().bg(t.code_background).p(t.space.small)
            .font_family(cx.theme().mono_font_family.clone()).text_size(cx.theme().mono_font_size)
            .child(content.clone()))
        .child(div().flex_shrink_0().sovra_text(TextRole::Caption, cx)
            .child("Source preview (up to 32,768 characters). Websites run in your browser on this computer."))
        .into_any_element())
}
