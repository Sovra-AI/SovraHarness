//! The sign-in gate: email + password (first submit creates the account) and
//! Google Sign-In via the browser (RFC 8252 loopback hand-off).
//!
//! Pure rendering over [`sovra_core::AuthState`]; every decision lives in the
//! core reducer and the runtime. The adapted glass look comes from
//! `SovraSRXProduction/frontend/src/components/auth/login-form.tsx`.

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, div,
    prelude::FluentBuilder as _,
};
use sovra_core::auth::{AuthPhase, AuthState};
use sovra_core::models::ChromeKey;

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_login_screen(
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
                    | AuthPhase::SignedIn(_) => {
                        card.child(login_form(email_input, password_input, auth, creating, cx))
                    }
                }),
        )
}

fn login_form(
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
                    .child(message),
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
        .child(
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
        .child(
            div()
                .w_full()
                .text_color(t.text_secondary)
                .sovra_text(TextRole::Caption, cx)
                .child(if creating {
                    design::chrome(cx, ChromeKey::LoginHintCreate)
                } else {
                    design::chrome(cx, ChromeKey::LoginHintSignIn)
                }),
        )
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
            div()
                .sovra_text(TextRole::Body, cx)
                .child(design::chrome(cx, ChromeKey::LoginGoogleWaiting)),
        )
        .child(
            div()
                .text_color(t.text_secondary)
                .sovra_text(TextRole::Caption, cx)
                .child(design::chrome(cx, ChromeKey::LoginGoogleWaitingHint)),
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
