//! The GitHub card on the Connections tab.
//!
//! One button connects. GitHub's page opens in the browser with a short code that
//! is already on the clipboard; the person pastes it, presses Authorize, and this
//! card turns to "Connected" by itself. Copies of Sovra without a GitHub app, and
//! anyone who prefers it, get three plain steps to make and paste a key instead.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, button::ButtonVariants as _, h_flex,
    input::InputState, v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    WeakEntity, Window, div,
};
use sovra_core::Command;
use sovra_core::github::{
    GitHubAccount, GitHubCommand, GitHubConnection, GitHubKey, GitHubState, KEY_PAGE_URL,
    MANAGE_ACCESS_URL,
};

use crate::RuntimeClient;
use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::google_card::TryAsking;
use crate::model_connection_editor::field;
use crate::shell::Shell;

/// Questions that show what a connected GitHub account is for.
const EXAMPLES: [&str; 3] = [
    "Which of my GitHub projects changed most recently?",
    "Are there any pull requests waiting for my review?",
    "Summarise the open issues in my newest project.",
];

/// Reads the GitHub state the shell folds; keeps only what is on screen.
pub(crate) struct GitHubCard {
    shell: WeakEntity<Shell>,
    /// The Disconnect button asks once more before forgetting the access.
    confirm_disconnect: bool,
    /// The make-a-key steps, for copies without GitHub sign-in or by choice.
    show_key_form: bool,
    key: Entity<InputState>,
}

impl gpui_kit::EventEmitter<TryAsking> for GitHubCard {}

impl GitHubCard {
    pub(crate) fn new(
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            shell,
            confirm_disconnect: false,
            show_key_form: false,
            key: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Paste the key here")
                    .masked(true)
            }),
        }
    }

    fn use_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = GitHubKey::new(self.key.read(cx).value().to_string()) else {
            return;
        };
        // Cleared before the runtime answers, so the key is never left on screen.
        self.key.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        // Keep the setup steps available if checking fails. The CheckingKey
        // state temporarily replaces them, and successful connection closes them.
        send(GitHubCommand::UseKey(key), cx);
        cx.notify();
    }

    /// Connect: GitHub's page when this copy can, else the key steps.
    fn connect(&mut self, device_sign_in: bool, cx: &mut Context<Self>) {
        if device_sign_in {
            send(GitHubCommand::Connect, cx);
        } else {
            self.show_key_form = true;
            cx.notify();
        }
    }
}

impl Render for GitHubCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let github = self
            .shell
            .upgrade()
            .map(|shell| shell.read(cx).workspace().github.clone())
            .unwrap_or_default();
        if !matches!(github.state, GitHubState::Connected(_)) {
            self.confirm_disconnect = false;
        }
        if matches!(
            github.state,
            GitHubState::Connected(_) | GitHubState::Connecting { .. }
        ) {
            self.show_key_form = false;
        }
        render(self, &github, cx)
    }
}

fn send(command: GitHubCommand, cx: &mut Context<GitHubCard>) {
    let _ = cx
        .global::<RuntimeClient>()
        .0
        .send(Command::GitHub(command));
}

fn render(
    view: &GitHubCard,
    github: &GitHubConnection,
    cx: &mut Context<GitHubCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let device = github.device_sign_in;
    let body = match &github.state {
        GitHubState::Loading => caption("Checking your GitHub connection…", cx).into_any_element(),
        GitHubState::NotConnected | GitHubState::NeedsSignIn(_) if view.show_key_form => {
            key_steps(view, device, cx).into_any_element()
        }
        GitHubState::NotConnected => not_connected(device, cx).into_any_element(),
        GitHubState::Connecting { code, url } => connecting(code, url, cx).into_any_element(),
        GitHubState::CheckingKey => caption("Checking your key with GitHub…", cx)
            .id("github-checking")
            .test_support()
            .into_any_element(),
        GitHubState::Connected(account) => {
            connected(account, view.confirm_disconnect, cx).into_any_element()
        }
        GitHubState::NeedsSignIn(account) => v_flex()
            .gap(t.space.medium)
            .child(identity(account, cx))
            .child(
                h_flex()
                    .gap(t.space.small)
                    .items_start()
                    .child(Icon::new(IconName::TriangleAlert).text_color(t.accent.amber))
                    .child(paragraph(
                        "GitHub stopped accepting Sovra's access, maybe because it was removed \
                         on GitHub or the key ran out. Connect again to carry on.",
                        cx,
                    )),
            )
            .child(
                h_flex().flex_wrap().child(
                    design::primary_button("github-reconnect", "Connect again", cx)
                        .icon(Icon::new(IconName::Plug))
                        .on_click(cx.listener(move |view, _, _, cx| view.connect(device, cx))),
                ),
            )
            .into_any_element(),
    };
    v_flex()
        .id("github-card")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .p(t.space.large)
        .sovra_surface(Surface::Glass, cx)
        .child(header(&github.state, cx))
        .children(github.notice.clone().map(|notice| {
            div()
                .id("github-notice")
                .test_support()
                .min_w_0()
                .sovra_text(TextRole::Body, cx)
                .text_color(cx.theme().foreground)
                .child(notice)
        }))
        .child(body)
}

fn caption(text: impl Into<gpui_kit::SharedString>, cx: &Context<GitHubCard>) -> gpui_kit::Div {
    div()
        .min_w_0()
        .sovra_text(TextRole::Caption, cx)
        .text_color(design::tokens(cx).text_secondary)
        .child(text.into())
}

fn paragraph(text: impl Into<gpui_kit::SharedString>, cx: &Context<GitHubCard>) -> gpui_kit::Div {
    div()
        .min_w_0()
        .sovra_text(TextRole::Body, cx)
        .child(text.into())
}

fn header(state: &GitHubState, cx: &Context<GitHubCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let connected = matches!(state, GitHubState::Connected(_));
    h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap(t.space.small)
        .items_center()
        .child(Icon::new(IconName::Code).size(t.layout.icon_medium))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("GitHub"),
        )
        .child(
            h_flex()
                .id("github-status")
                .test_support()
                .gap(t.space.compact)
                .items_center()
                .px(t.space.small)
                .rounded(t.radius.pill)
                .bg(t.status_pill_background)
                .sovra_text(TextRole::Caption, cx)
                .children(connected.then(|| {
                    Icon::new(IconName::CircleCheck)
                        .size(t.layout.icon_small)
                        .text_color(t.accent.green)
                }))
                .child(state.status_label()),
        )
}

fn not_connected(device: bool, cx: &mut Context<GitHubCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .gap(t.space.medium)
        .child(paragraph(
            "Connect GitHub and Sovra can look through your projects, read the code, issues \
             and pull requests, and tell you whether the checks passed. It only opens an issue \
             or writes a comment when you ask.",
            cx,
        ))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .items_center()
                .child(
                    design::primary_button("github-connect", "Connect GitHub", cx)
                        .icon(Icon::new(IconName::Plug))
                        .on_click(cx.listener(move |view, _, _, cx| view.connect(device, cx))),
                )
                .children(device.then(|| {
                    design::button("github-use-key", "Use a GitHub key instead", cx)
                        .ghost()
                        .small()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.show_key_form = true;
                            cx.notify();
                        }))
                })),
        )
        .child(caption(
            if device {
                "You'll approve Sovra on GitHub's own page. Sovra never sees your password, and \
                 it never changes your code."
            } else {
                "GitHub will make a key for Sovra. Sovra never sees your password, and it never \
                 changes your code."
            },
            cx,
        ))
}

/// One numbered step: a number, then what to do.
fn step(number: &str, text: &'static str, cx: &Context<GitHubCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    h_flex()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .items_start()
        .child(
            div()
                .flex_shrink_0()
                .sovra_text(TextRole::SectionTitle, cx)
                .text_color(cx.theme().link)
                .child(number.to_owned()),
        )
        .child(paragraph(text, cx).flex_1())
}

fn key_steps(
    view: &GitHubCard,
    device: bool,
    cx: &mut Context<GitHubCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let empty = view.key.read(cx).value().trim().is_empty();
    v_flex()
        .id("github-key-steps")
        .test_support()
        .gap(t.space.medium)
        .child(step(
            "1.",
            "Press Open GitHub. GitHub's page opens with everything already filled in.",
            cx,
        ))
        .child(
            h_flex().flex_wrap().child(
                design::button("github-open-key-page", "Open GitHub", cx)
                    .icon(Icon::new(IconName::ExternalLink))
                    .on_click(|_, _, cx| cx.open_url(KEY_PAGE_URL)),
            ),
        )
        .child(step(
            "2.",
            "Scroll to the bottom of that page and press Generate token. Copy the key GitHub \
             shows you. It starts with ghp_.",
            cx,
        ))
        .child(step("3.", "Paste the key below and press Connect.", cx))
        .child(field(
            "github-key",
            "Your GitHub key",
            &view.key,
            "Sovra keeps it safely on this computer and does not show it again.",
            cx,
        ))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::primary_button("github-use-key-connect", "Connect", cx)
                        .disabled(empty)
                        .on_click(cx.listener(|view, _, window, cx| view.use_key(window, cx))),
                )
                .children(device.then(|| {
                    design::button("github-key-back", "Back", cx)
                        .ghost()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.show_key_form = false;
                            cx.notify();
                        }))
                })),
        )
}

fn connecting(code: &str, url: &str, cx: &mut Context<GitHubCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let open_url = url.to_owned();
    let copy_code = code.to_owned();
    v_flex()
        .gap(t.space.medium)
        .child(paragraph(
            "Your web browser has opened GitHub. Type or paste this code there:",
            cx,
        ))
        .child(
            div()
                .id("github-code")
                .test_support()
                .aria_label(format!("Your code is {code}"))
                .self_start()
                .px(t.space.large)
                .py(t.space.small)
                .rounded(t.radius.card)
                .sovra_surface(Surface::Elevated, cx)
                .sovra_text(TextRole::PageTitle, cx)
                .child(code.to_owned()),
        )
        .child(caption(
            "Sovra has already copied it for you. Paste it, press Continue, then press \
             Authorize. This page updates by itself when you're done.",
            cx,
        ))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("github-copy-code", "Copy code", cx)
                        .icon(Icon::new(IconName::Copy))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_code.clone()));
                        }),
                )
                .child(
                    design::button("github-open-again", "Open GitHub again", cx)
                        .icon(Icon::new(IconName::ExternalLink))
                        .on_click(move |_, _, cx| cx.open_url(&open_url)),
                )
                .child(
                    design::button("github-cancel", "Cancel", cx)
                        .ghost()
                        .on_click(cx.listener(|_, _, _, cx| send(GitHubCommand::Cancel, cx))),
                ),
        )
}

fn identity(account: &GitHubAccount, cx: &Context<GitHubCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    h_flex()
        .id("github-account")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.small)
        .items_center()
        .child(
            div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size(t.layout.profile_avatar)
                .rounded_full()
                .bg(theme.primary)
                .text_color(theme.primary_foreground)
                .sovra_text(TextRole::SectionTitle, cx)
                .child(account.initial()),
        )
        .child(
            v_flex()
                .min_w_0()
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .text_ellipsis()
                        .child(account.display_name().to_owned()),
                )
                .child(caption(format!("@{}", account.login), cx).text_ellipsis()),
        )
}

fn connected(
    account: &GitHubAccount,
    confirm_disconnect: bool,
    cx: &mut Context<GitHubCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let examples: Vec<_> = EXAMPLES
        .iter()
        .enumerate()
        .map(|(index, prompt)| {
            design::button(("github-try", index), format!("“{prompt}”"), cx)
                .small()
                .ghost()
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(TryAsking {
                        prompt: (*prompt).to_owned(),
                    });
                }))
                .into_any_element()
        })
        .collect();
    let disconnect = if confirm_disconnect {
        v_flex()
            .id("github-disconnect-confirm")
            .test_support()
            .gap(t.space.small)
            .child(paragraph(
                "Disconnect GitHub? Sovra will forget its access on this computer. To remove \
                 Sovra from GitHub completely, also open GitHub's settings.",
                cx,
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(t.space.small)
                    .child(
                        design::button("github-disconnect-yes", "Disconnect", cx)
                            .danger()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.confirm_disconnect = false;
                                send(GitHubCommand::Disconnect, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        design::button("github-disconnect-no", "Keep connected", cx).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.confirm_disconnect = false;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        design::button("github-settings", "Open GitHub's settings", cx)
                            .ghost()
                            .icon(Icon::new(IconName::ExternalLink))
                            .on_click(|_, _, cx| cx.open_url(MANAGE_ACCESS_URL)),
                    ),
            )
            .into_any_element()
    } else {
        design::button("github-disconnect", "Disconnect GitHub", cx)
            .ghost()
            .on_click(cx.listener(|view, _, _, cx| {
                view.confirm_disconnect = true;
                cx.notify();
            }))
            .into_any_element()
    };
    v_flex()
        .gap(t.space.medium)
        .child(identity(account, cx))
        .child(paragraph(
            "Sovra can now look at your GitHub projects whenever you ask in Chat.",
            cx,
        ))
        .child(
            v_flex()
                .gap(t.space.compact)
                .child(caption("Try asking Sovra:", cx))
                .child(h_flex().flex_wrap().gap(t.space.compact).children(examples)),
        )
        .child(h_flex().flex_wrap().child(disconnect))
}
