//! The sign-in gate: a name on a new account, optional email and password, and
//! Google Sign-In via the browser (RFC 8252 loopback hand-off).
//!
//! Pure rendering over [`sovra_core::AuthState`]; every decision lives in the
//! core reducer and the runtime. The adapted glass look comes from
//! `SovraSRXProduction/frontend/src/components/auth/login-form.tsx`.

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
    prelude::FluentBuilder as _,
};
use sovra_core::auth::{AuthPhase, AuthState};
use sovra_core::models::{ChromeKey, ModelCommand, UiLanguage};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_login_screen(
    name_input: &Entity<InputState>,
    email_input: &Entity<InputState>,
    password_input: &Entity<InputState>,
    auth: &AuthState,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    // Until the runtime reports, the form reads as a sign-in; the first
    // submit on a fresh install creates the account instead.
    let creating = !auth.has_accounts && !matches!(auth.phase, AuthPhase::Unknown);
    let (title, subtitle) = if creating {
        (
            design::chrome(cx, ChromeKey::LoginCreateTitle),
            design::chrome(cx, ChromeKey::LoginCreateSubtitle),
        )
    } else {
        (
            design::chrome(cx, ChromeKey::LoginWelcomeTitle),
            design::chrome(cx, ChromeKey::LoginWelcomeSubtitle),
        )
    };

    v_flex()
        .id("sovra-login-screen")
        .test_support()
        .flex_1()
        .min_h_0()
        .w_full()
        .items_center()
        .overflow_y_scroll()
        .p(t.space.large)
        .child(
            v_flex()
                .id("login-glass-card")
                .test_support()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .my_auto()
                .max_w(t.layout.dialog_width)
                .sovra_surface(Surface::Glass, cx)
                .items_center()
                .gap(t.space.small)
                .p(t.space.large)
                .child(language_switch(cx))
                .child({
                    let mut row = h_flex()
                        .w_full()
                        .min_w_0()
                        .flex_wrap()
                        .items_center()
                        .gap(t.space.small)
                        .pb(t.space.small);
                    if design::is_rtl(cx) {
                        row = row.flex_row_reverse();
                    }
                    row.child(logo("login-logo", t.layout.logo_hero)).child(
                        design::page_header(title, subtitle, cx)
                            .id("login-welcome-title")
                            .test_support()
                            .aria_label(title)
                            .flex_1()
                            .min_w_0(),
                    )
                })
                .map(|card| match &auth.phase {
                    AuthPhase::WaitingForGoogle { auth_url } => {
                        card.child(google_waiting(auth_url, cx))
                    }
                    AuthPhase::Unknown
                    | AuthPhase::SignedOut
                    | AuthPhase::Submitting
                    | AuthPhase::SignedIn(_) => card.child(login_form(
                        name_input,
                        email_input,
                        password_input,
                        auth,
                        creating,
                        cx,
                    )),
                }),
        )
}

fn login_form(
    name_input: &Entity<InputState>,
    email_input: &Entity<InputState>,
    password_input: &Entity<InputState>,
    auth: &AuthState,
    creating: bool,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let busy = auth.busy();
    let submit_label = match (busy, creating) {
        (true, true) => design::chrome(cx, ChromeKey::LoginCreating),
        (true, false) => design::chrome(cx, ChromeKey::LoginSigningIn),
        (false, true) => design::chrome(cx, ChromeKey::LoginCreateStart),
        (false, false) => design::chrome(cx, ChromeKey::LoginSignIn),
    };

    v_flex()
        .w_full()
        .gap(t.space.small)
        .when(!creating && !auth.local_profiles.is_empty(), |form| {
            form.child(
                v_flex().w_full().gap(t.space.small).children(
                    auth.local_profiles
                        .iter()
                        .enumerate()
                        .map(|(index, profile)| {
                            let account_id = profile.account_id.clone();
                            let label = format!(
                                "{} · {}",
                                design::chrome(cx, ChromeKey::LoginSignIn),
                                profile.full_name
                            );
                            design::primary_button(("login-local-profile", index), "", cx)
                                .w_full()
                                .min_w_0()
                                .accessibility_label(label.clone())
                                .tooltip(label.clone())
                                .child(div().min_w_0().w_full().truncate().child(label))
                                .disabled(busy)
                                .on_click(cx.listener(move |shell, _, _, cx| {
                                    shell.continue_local_profile(account_id.clone(), cx);
                                }))
                        }),
                ),
            )
        })
        .when(creating, |form| {
            form.child(
                design::paragraph(design::chrome(cx, ChromeKey::LoginName))
                    .id("login-name-label")
                    .sovra_text(TextRole::Caption, cx),
            )
            .child(
                Input::new(name_input)
                    .id("login-name-input")
                    .aria_label(design::chrome(cx, ChromeKey::LoginName))
                    .prefix(
                        Icon::new(IconName::User)
                            .small()
                            .text_color(t.text_secondary),
                    ),
            )
        })
        .child(
            design::paragraph(design::chrome(cx, ChromeKey::LoginEmail))
                .id("login-email-label")
                .sovra_text(TextRole::Caption, cx),
        )
        .child(
            Input::new(email_input)
                .id("login-email-input")
                .aria_label(design::chrome(cx, ChromeKey::LoginEmail))
                .prefix(
                    Icon::new(IconName::Mail)
                        .small()
                        .text_color(t.text_secondary),
                ),
        )
        .child(
            design::paragraph(design::chrome(cx, ChromeKey::LoginPassword))
                .id("login-password-label")
                .sovra_text(TextRole::Caption, cx),
        )
        .child(
            Input::new(password_input)
                .id("login-password-input")
                .aria_label(design::chrome(cx, ChromeKey::LoginPassword))
                .mask_toggle()
                .prefix(
                    Icon::new(IconName::Lock)
                        .small()
                        .text_color(t.text_secondary),
                ),
        )
        .when_some(auth.error.clone(), |form, (_, message)| {
            form.child(
                div()
                    .id("login-error")
                    .test_support()
                    .w_full()
                    .text_color(cx.theme().danger)
                    .sovra_text(TextRole::Caption, cx)
                    .child(design::paragraph(message)),
            )
        })
        .child(
            design::primary_button("login-submit-btn", submit_label, cx)
                .w_full()
                .disabled(busy)
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.submit_login(cx);
                })),
        )
        // Only offered when this copy can sign in with Google; otherwise the button
        // would lead to an error about setup the person cannot do yet.
        .when(auth.google_ready, |form| {
            form.child(
                design::button(
                    "login-google-btn",
                    design::chrome(cx, ChromeKey::LoginGoogle),
                    cx,
                )
                .w_full()
                .disabled(busy)
                .on_click(cx.listener(|shell, _, _, cx| {
                    shell.start_google_signin(cx);
                })),
            )
        })
        .child(
            div()
                .w_full()
                .text_color(t.text_secondary)
                .sovra_text(TextRole::Caption, cx)
                .child(design::paragraph(if creating {
                    design::chrome(cx, ChromeKey::LoginHintCreate)
                } else {
                    design::chrome(cx, ChromeKey::LoginHintSignIn)
                })),
        )
}

/// English or Arabic, before anything else, so nobody has to read a language they
/// don't know to get in. Every other language is in Settings.
fn language_switch(cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let current = design::ui_language(cx);
    let choices = [
        (
            UiLanguage::english(),
            "login-language-en",
            "English",
            "Show Sovra in English",
        ),
        (
            UiLanguage::arabic(),
            "login-language-ar",
            "العربية",
            "اعرض Sovra بالعربية",
        ),
    ];
    h_flex()
        .id("login-language")
        .test_support()
        .w_full()
        .flex_wrap()
        .justify_end()
        .gap(t.space.compact)
        .children(choices.into_iter().map(|(language, id, label, a11y)| {
            let selected = current.tag() == language.tag();
            design::button(id, label, cx)
                .small()
                .selected(selected)
                .accessibility_label(a11y)
                .on_click(move |_, _, cx| {
                    design::set_ui_language(language.clone(), cx);
                    let _ =
                        cx.global::<crate::RuntimeClient>()
                            .0
                            .send(sovra_core::Command::Models(ModelCommand::SetLanguage {
                                language: language.clone(),
                            }));
                    cx.refresh_windows();
                })
        }))
}

/// The browser hand-off: Google is open in the person's browser; the loopback
/// listener finishes the sign-in and this screen updates by itself.
fn google_waiting(auth_url: &str, cx: &mut Context<Shell>) -> impl IntoElement + use<> {
    let t = design::tokens(cx);
    let auth_url = auth_url.to_owned();
    v_flex()
        .id("login-google-waiting")
        .test_support()
        .w_full()
        .items_center()
        .gap(t.space.small)
        .child(
            div().sovra_text(TextRole::Body, cx).child(
                design::paragraph(design::chrome(cx, ChromeKey::LoginGoogleWaiting))
                    .id("google-waiting-title"),
            ),
        )
        .child(
            div()
                .text_color(t.text_secondary)
                .sovra_text(TextRole::Caption, cx)
                .child(
                    design::paragraph(design::chrome(cx, ChromeKey::LoginGoogleWaitingHint))
                        .id("google-waiting-hint"),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .flex_wrap()
                .gap(t.space.small)
                .justify_center()
                .child(
                    design::button(
                        "login-google-open-again-btn",
                        design::chrome(cx, ChromeKey::LoginOpenAgain),
                        cx,
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.open_url(&auth_url);
                    })),
                )
                .child(
                    design::button(
                        "login-google-cancel-btn",
                        design::chrome(cx, ChromeKey::LoginCancel),
                        cx,
                    )
                    .on_click(cx.listener(|shell, _, _, cx| {
                        shell.cancel_google_signin(cx);
                    })),
                ),
        )
}
