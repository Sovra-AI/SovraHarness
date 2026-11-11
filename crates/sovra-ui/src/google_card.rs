//! The Google card on the Connections tab.
//!
//! One button connects; Google's own page in the browser does the rest. Once
//! connected, every service is a row with a switch, in the order of
//! `sovra_core::google::SERVICES`, so a new service needs only an icon here.
//! A switched-on service Google wasn't allowed to use asks Google again.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, button::ButtonVariants as _, h_flex,
    input::InputState, switch::Switch, v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, WeakEntity, Window, div,
};
use sovra_core::Command;
use sovra_core::google::{
    ClientSecret, DESKTOP_CLIENT_NEEDED, GOOGLE_NOT_INCLUDED, GoogleAccount, GoogleCommand,
    GoogleConnection, GoogleService, GoogleState, SERVICES, ServiceInfo,
};

use crate::RuntimeClient;
use crate::design::{self, FitsPane as _, SovraStyled as _, Surface, TextRole};
use crate::model_connection_editor::field;
use crate::shell::Shell;

/// Ask Sovra this in Chat (an example under a connected account).
#[derive(Debug, Clone)]
pub(crate) struct TryAsking {
    pub(crate) prompt: String,
}

/// Reads the Google state the shell folds; keeps only what is on screen.
pub(crate) struct GoogleCard {
    shell: WeakEntity<Shell>,
    /// The Disconnect button asks once more before signing out.
    confirm_disconnect: bool,
    /// The bring-your-own-client form stays folded so most people never see it.
    show_client_form: bool,
    client_id: Entity<InputState>,
    client_secret: Entity<InputState>,
}

impl EventEmitter<TryAsking> for GoogleCard {}

impl GoogleCard {
    pub(crate) fn new(
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            shell,
            confirm_disconnect: false,
            show_client_form: false,
            client_id: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Ends with .apps.googleusercontent.com")
            }),
            client_secret: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Only if your project has one")
                    .masked(true)
            }),
        }
    }

    fn save_client(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client_id = self.client_id.read(cx).value().trim().to_owned();
        if client_id.is_empty() {
            return;
        }
        let secret_text = self.client_secret.read(cx).value().trim().to_owned();
        let client_secret = (!secret_text.is_empty()).then(|| ClientSecret::new(secret_text));
        // The field is cleared before the runtime answers, so the secret is not
        // left on screen or shown again after a successful save.
        self.client_secret.update(cx, |input, cx| {
            input.set_value(String::new(), window, cx);
        });
        send(
            GoogleCommand::SaveClient {
                client_id,
                client_secret,
            },
            cx,
        );
    }
}

impl Render for GoogleCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let google = self
            .shell
            .upgrade()
            .map(|shell| shell.read(cx).workspace().google.clone())
            .unwrap_or_default();
        if !matches!(google.state, GoogleState::Connected(_)) {
            self.confirm_disconnect = false;
        }
        render(self, &google, cx)
    }
}

fn service_icon(service: GoogleService) -> IconName {
    match service {
        GoogleService::Mail => IconName::Mail,
        GoogleService::Calendar => IconName::Calendar,
        GoogleService::Files => IconName::FolderOpen,
        GoogleService::Contacts => IconName::Contact,
        GoogleService::Tasks => IconName::ListTodo,
    }
}

/// Questions that show what a connected service is for.
fn example(service: GoogleService) -> &'static str {
    match service {
        GoogleService::Mail => "Do I have any new emails I should answer?",
        GoogleService::Calendar => "What's on my calendar this week?",
        GoogleService::Files => "Find my latest document about the holiday.",
        GoogleService::Contacts => "What is Anna's phone number?",
        GoogleService::Tasks => "Add “buy milk” to my to-dos.",
    }
}

fn send(command: GoogleCommand, cx: &mut Context<GoogleCard>) {
    let _ = cx
        .global::<RuntimeClient>()
        .0
        .send(Command::Google(command));
}

fn render(
    view: &GoogleCard,
    google: &GoogleConnection,
    cx: &mut Context<GoogleCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let body = match &google.state {
        GoogleState::Loading => caption("Checking your Google connection…", cx).into_any_element(),
        GoogleState::Unavailable { .. } => client_setup(view, cx).into_any_element(),
        GoogleState::NotConnected => not_connected(cx).into_any_element(),
        GoogleState::Connecting { url, adding } => connecting(url, *adding, cx).into_any_element(),
        GoogleState::Connected(account) => {
            connected(account, view.confirm_disconnect, cx).into_any_element()
        }
        GoogleState::NeedsSignIn(account) => v_flex()
            .gap(t.space.medium)
            .child(identity(account, cx))
            .child(
                h_flex()
                    .gap(t.space.small)
                    .items_start()
                    .child(Icon::new(IconName::TriangleAlert).text_color(t.accent.amber))
                    .child(paragraph(
                        "Google needs you to sign in again before Sovra can keep helping. \
                         This happens now and then, for your safety.",
                        cx,
                    )),
            )
            .child(
                design::primary_button("google-reconnect", "Sign in again", cx)
                    .icon(Icon::new(IconName::Plug))
                    .on_click(cx.listener(|_, _, _, cx| send(GoogleCommand::Connect, cx))),
            )
            .into_any_element(),
    };
    v_flex()
        .id("google-card")
        .test_support()
        .w_full()
        .min_w_0()
        .gap(t.space.medium)
        .p(t.space.large)
        .sovra_surface(Surface::Glass, cx)
        .child(header(&google.state, cx))
        .children(google.notice.clone().map(|notice| {
            div()
                .id("google-notice")
                .test_support()
                .sovra_text(TextRole::Body, cx)
                .text_color(cx.theme().foreground)
                .child(notice)
        }))
        .child(body)
}

fn caption(text: impl Into<gpui_kit::SharedString>, cx: &Context<GoogleCard>) -> gpui_kit::Div {
    div()
        .min_w_0()
        .sovra_text(TextRole::Caption, cx)
        .text_color(design::tokens(cx).text_secondary)
        .child(text.into())
}

fn paragraph(text: impl Into<gpui_kit::SharedString>, cx: &Context<GoogleCard>) -> gpui_kit::Div {
    div()
        .min_w_0()
        .sovra_text(TextRole::Body, cx)
        .child(text.into())
}

fn header(state: &GoogleState, cx: &Context<GoogleCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let connected = matches!(state, GoogleState::Connected(_));
    h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap(t.space.small)
        .items_center()
        .child(Icon::new(IconName::Globe).size(t.layout.icon_medium))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .sovra_text(TextRole::SectionTitle, cx)
                .child("Google"),
        )
        .child(
            h_flex()
                .id("google-status")
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

fn client_setup(view: &GoogleCard, cx: &mut Context<GoogleCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let lead = v_flex().gap(t.space.medium).child(
        div()
            .id("google-not-included")
            .aria_label(GOOGLE_NOT_INCLUDED)
            .test_support()
            .min_w_0()
            .sovra_text(TextRole::Body, cx)
            .child(GOOGLE_NOT_INCLUDED),
    );
    if !view.show_client_form {
        return lead.child(
            h_flex().flex_wrap().child(
                design::button("google-own-client", "Use my own Google Cloud client", cx)
                    .ghost()
                    .small()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_client_form = true;
                        cx.notify();
                    })),
            ),
        );
    }
    let id_empty = view.client_id.read(cx).value().trim().is_empty();
    lead.child(
        div()
            .id("google-client-help")
            .aria_label(DESKTOP_CLIENT_NEEDED)
            .test_support()
            .min_w_0()
            .sovra_text(TextRole::Body, cx)
            .child(DESKTOP_CLIENT_NEEDED),
    )
    .child(field(
        "google-client-id",
        "Desktop client id",
        &view.client_id,
        "Copy it from your Google Cloud project. It ends with .apps.googleusercontent.com.",
        cx,
    ))
    .child(field(
        "google-client-secret",
        "Secret, if your project has one",
        &view.client_secret,
        "Optional. Sovra keeps it on this computer and does not show it again.",
        cx,
    ))
    .child(
        h_flex().flex_wrap().child(
            design::primary_button("google-save-client", "Save", cx)
                .disabled(id_empty)
                .on_click(cx.listener(|view, _, window, cx| view.save_client(window, cx))),
        ),
    )
}

fn not_connected(cx: &mut Context<GoogleCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    v_flex()
        .gap(t.space.medium)
        .child(paragraph(
            "Connect your Google account and Sovra can read your email, see your calendar, \
             find your files and contacts, and keep your to-do list — whenever you ask.",
            cx,
        ))
        .child(
            v_flex()
                .gap(t.space.compact)
                .children(SERVICES.iter().map(|info| {
                    h_flex()
                        .gap(t.space.small)
                        .items_center()
                        .min_w_0()
                        .child(Icon::new(service_icon(info.service)).text_color(t.text_secondary))
                        .child(caption(format!("{} — {}", info.label, info.product), cx))
                })),
        )
        .child(
            h_flex().child(
                design::primary_button("google-connect", "Connect Google", cx)
                    .icon(Icon::new(IconName::Plug))
                    .on_click(cx.listener(|_, _, _, cx| send(GoogleCommand::Connect, cx))),
            ),
        )
        .child(caption(
            "You'll sign in on Google's own page in your web browser. Sovra never sees your \
             password, and it never sends an email by itself.",
            cx,
        ))
}

fn connecting(
    url: &str,
    adding: Option<GoogleService>,
    cx: &mut Context<GoogleCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let what = adding.map_or_else(
        || "Your web browser has opened Google's sign-in page.".to_owned(),
        |service| {
            format!(
                "Your web browser has opened Google's page so you can allow {}.",
                sovra_core::google::service_info(service).product
            )
        },
    );
    let open_url = url.to_owned();
    let copy_url = url.to_owned();
    v_flex()
        .gap(t.space.medium)
        .child(paragraph(what, cx))
        .child(caption(
            "Choose your account, tick the boxes for what Sovra may use, then press Continue. \
             Come back here when it says you're connected.",
            cx,
        ))
        .child(
            h_flex()
                .flex_wrap()
                .gap(t.space.small)
                .child(
                    design::button("google-open-again", "Open the page again", cx)
                        .icon(Icon::new(IconName::ExternalLink))
                        .on_click(move |_, _, cx| cx.open_url(&open_url)),
                )
                .child(
                    design::button("google-copy-link", "Copy link", cx)
                        .icon(Icon::new(IconName::Copy))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_url.clone()));
                        }),
                )
                .child(
                    design::button("google-cancel", "Cancel", cx)
                        .ghost()
                        .on_click(cx.listener(|_, _, _, cx| send(GoogleCommand::Cancel, cx))),
                ),
        )
}

fn identity(account: &GoogleAccount, cx: &Context<GoogleCard>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let theme = cx.theme();
    let name = if account.name.is_empty() {
        account.email.clone()
    } else {
        account.name.clone()
    };
    h_flex()
        .id("google-account")
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
                        .child(name),
                )
                .child(caption(account.email.clone(), cx).text_ellipsis()),
        )
}

fn service_row(
    info: &ServiceInfo,
    account: &GoogleAccount,
    index: usize,
    cx: &mut Context<GoogleCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let service = info.service;
    let on = account.is_active(service);
    let allowed = account.granted.contains(&service);
    let status = match (on, allowed) {
        (true, _) => "On",
        (false, true) => "Off",
        (false, false) => "Off — turning it on asks Google",
    };
    h_flex()
        .id(("google-service", index))
        .test_support()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap(t.space.medium)
        .items_center()
        .py(t.space.small)
        .child(
            Icon::new(service_icon(service))
                .size(t.layout.icon_medium)
                .text_color(if on {
                    cx.theme().link
                } else {
                    t.text_secondary
                }),
        )
        .child(
            v_flex()
                .prefer_width(t.layout.card_min_width)
                .gap(t.space.hair)
                .child(
                    div()
                        .sovra_text(TextRole::SectionTitle, cx)
                        .child(format!("{} · {}", info.label, info.product)),
                )
                .child(caption(info.explanation, cx)),
        )
        .child(
            Switch::new(("google-service-switch", index))
                .checked(on)
                .label(status)
                .accessibility_label(format!("Let Sovra use your {}", info.product))
                .on_click(cx.listener(move |_, checked: &bool, _, cx| {
                    send(
                        GoogleCommand::SetService {
                            service,
                            on: *checked,
                        },
                        cx,
                    );
                })),
        )
}

fn connected(
    account: &GoogleAccount,
    confirm_disconnect: bool,
    cx: &mut Context<GoogleCard>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let rows: Vec<_> = SERVICES
        .iter()
        .enumerate()
        .map(|(index, info)| service_row(info, account, index, cx).into_any_element())
        .collect();
    let examples: Vec<_> = SERVICES
        .iter()
        .filter(|info| account.is_active(info.service))
        .enumerate()
        .map(|(index, info)| {
            let prompt = example(info.service);
            design::button(("google-try", index), format!("“{prompt}”"), cx)
                .small()
                .ghost()
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(TryAsking {
                        prompt: prompt.to_owned(),
                    });
                }))
                .into_any_element()
        })
        .collect();
    let disconnect = if confirm_disconnect {
        h_flex()
            .id("google-disconnect-confirm")
            .test_support()
            .flex_wrap()
            .gap(t.space.small)
            .items_center()
            .child(paragraph(
                "Disconnect Google? Sovra will stop using your account until you connect again.",
                cx,
            ))
            .child(
                design::button("google-disconnect-yes", "Disconnect", cx)
                    .danger()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.confirm_disconnect = false;
                        send(GoogleCommand::Disconnect, cx);
                        cx.notify();
                    })),
            )
            .child(
                design::button("google-disconnect-no", "Keep connected", cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.confirm_disconnect = false;
                        cx.notify();
                    },
                )),
            )
            .into_any_element()
    } else {
        design::button("google-disconnect", "Disconnect Google", cx)
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
        .child(
            v_flex()
                .w_full()
                .border_t_1()
                .border_color(t.material_border)
                .children(rows),
        )
        .children((!examples.is_empty()).then(|| {
            v_flex()
                .gap(t.space.compact)
                .child(caption("Try asking Sovra:", cx))
                .child(h_flex().flex_wrap().gap(t.space.compact).children(examples))
        }))
        .child(h_flex().flex_wrap().child(disconnect))
}
