//! Sleek Sovra Login view (adapted from `SovraSRXProduction/frontend/src/components/auth/login-form.tsx`).

use gpui_kit::assets::IconName; // full Lucide catalog; component::IconName is a small subset
use gpui_kit::component::{
    Icon, Sizable as _,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, div, px,
};

use crate::design::{self, SovraStyled as _, Surface, TextRole};
use crate::shell::{Shell, logo};

pub(crate) fn render_login_screen(
    email_input: &Entity<InputState>,
    password_input: &Entity<InputState>,
    cx: &mut Context<Shell>,
) -> impl IntoElement + use<> {
    let t = design::tokens(cx);

    v_flex()
        .id("sovra-login-screen")
        .flex_1()
        .min_h_0()
        .w_full()
        .items_center()
        .justify_center()
        .p(t.space.large)
        .child(
            v_flex()
                .id("login-glass-card")
                .w_full()
                .max_w(px(432.))
                .sovra_surface(Surface::Glass, cx)
                .items_center()
                .gap(t.space.medium)
                .p(t.space.large)
                .child(logo("login-logo", px(64.)))
                .child(
                    v_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .sovra_text(TextRole::PageTitle, cx)
                                .child("Welcome back"),
                        )
                        .child(
                            div()
                                .sovra_text(TextRole::Body, cx)
                                .text_color(t.text_secondary)
                                .child("Sign in to continue to Sovra."),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap(t.space.small)
                        .child(
                            Input::new(email_input)
                                .id("login-email-input")
                                .aria_label("Email address")
                                .prefix(
                                    Icon::new(IconName::Mail)
                                        .small()
                                        .text_color(t.text_secondary),
                                ),
                        )
                        .child(
                            Input::new(password_input)
                                .id("login-password-input")
                                .aria_label("Password")
                                .mask_toggle()
                                .prefix(
                                    Icon::new(IconName::Lock)
                                        .small()
                                        .text_color(t.text_secondary),
                                ),
                        ),
                )
                .child(
                    design::primary_button("login-submit-btn", "Sign in to Sovra", cx)
                        .w_full()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.submit_login(cx);
                        })),
                )
                .child(
                    design::button("login-google-btn", "Continue with Google", cx)
                        .w_full()
                        .on_click(cx.listener(|shell, _, _, cx| {
                            shell.submit_login(cx);
                        })),
                ),
        )
}
